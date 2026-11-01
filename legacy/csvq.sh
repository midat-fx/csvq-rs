#!/usr/bin/env bash
#
# csvq — deterministic CSV data-quality toolkit.
# Pure Bash + POSIX awk, zero runtime dependencies. See README.md.
#
# Determinism: LC_ALL=C everywhere; outputs carry no timestamps, hostnames,
# or absolute-path-of-the-day. The same input always yields byte-identical
# output on any POSIX awk.

set -euo pipefail
export LC_ALL=C

VERSION="1.0.0"

# Resolve the directory of this script so lib/*.awk is found when installed.
_src="${BASH_SOURCE[0]}"
while [ -h "$_src" ]; do
  _dir="$(cd -P "$(dirname "$_src")" && pwd)"
  _src="$(readlink "$_src")"
  case "$_src" in /*) ;; *) _src="$_dir/$_src" ;; esac
done
DIR="$(cd -P "$(dirname "$_src")" && pwd)"
LIB="$DIR/lib"
AWK="${CSVQ_AWK:-awk}"
TAB="$(printf '\t')"

die() {
  printf 'csvq: %s\n' "$1" >&2
  exit "${2:-2}"
}

require_file() { [ -f "$1" ] || die "no such file: $1"; }

# Expand a list of paths: directories become their *.csv children (sorted),
# regular files pass through. Prints one path per line.
collect_files() {
  local p
  for p in "$@"; do
    if [ -d "$p" ]; then
      find "$p" -maxdepth 1 -type f -name '*.csv' | LC_ALL=C sort
    else
      require_file "$p"
      printf '%s\n' "$p"
    fi
  done
}

usage() {
  cat <<'EOF'
csvq — deterministic CSV data-quality toolkit

Usage:
  csvq audit    <paths...> [-o DIR] [-d DELIM] [--strict]
  csvq stats    <file> [-d DELIM]
  csvq validate <file> --schema FILE [-d DELIM]
  csvq diff     <old> <new> --key COL[,COL] [-d DELIM]
  csvq dedupe   <file> [--key COL[,COL]] [-d DELIM]
  csvq normalize <file> [--in-place]
  csvq version
  csvq help

Commands:
  audit      Write analysis_report.txt, data_summary.txt, duplicates_found.txt
             into DIR (default ./out). --strict exits 1 if any file has
             duplicate or ragged rows (use it as a CI gate).
  stats      Per-column type, null rate, cardinality, min/max/mean, top values.
  validate   Check a file against a schema DSL (docs/schema-dsl.md). Exits 1
             on any violation.
  diff       Keyed row diff between two CSVs (added / changed / removed).
  dedupe     Drop duplicate rows (whole row, or by --key), keeping the first.
  normalize  CRLF->LF, strip trailing whitespace, ensure a final newline.

Global:
  -d, --delim DELIM   Force the delimiter (default: auto-detect from header).
  All output is deterministic and LC_ALL=C sorted.
EOF
}

cmd_normalize() {
  local file="" inplace=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --in-place) inplace=1 ;;
      -*) die "normalize: unknown option $1" ;;
      *) file="$1" ;;
    esac
    shift
  done
  [ -n "$file" ] || die "normalize: missing file"
  require_file "$file"
  if [ "$inplace" -eq 1 ]; then
    local tmp
    tmp="$(mktemp)"
    "$AWK" '{ sub(/\r$/, ""); sub(/[ \t]+$/, ""); print }' "$file" >"$tmp"
    mv "$tmp" "$file"
    printf 'csvq normalize: rewrote %s\n' "$file" >&2
  else
    "$AWK" '{ sub(/\r$/, ""); sub(/[ \t]+$/, ""); print }' "$file"
  fi
}

cmd_audit() {
  local outdir="out" delim="" strict=0
  local -a paths=()
  while [ $# -gt 0 ]; do
    case "$1" in
      -o)
        shift
        outdir="${1:-}"
        [ -n "$outdir" ] || die "audit: -o needs a directory"
        ;;
      -d | --delim)
        shift
        delim="${1:-}"
        ;;
      --strict) strict=1 ;;
      -*) die "audit: unknown option $1" ;;
      *) paths+=("$1") ;;
    esac
    shift
  done
  [ "${#paths[@]}" -gt 0 ] || die "audit: no input files"

  local -a files=()
  local f
  while IFS= read -r f; do [ -n "$f" ] && files+=("$f"); done < <(collect_files "${paths[@]}")
  [ "${#files[@]}" -gt 0 ] || die "audit: no .csv files found"

  mkdir -p "$outdir"
  local facts
  facts="$(mktemp)"
  local bytes
  for f in "${files[@]}"; do
    bytes="$(wc -c <"$f" | tr -d ' ')"
    "$AWK" -f "$LIB/csvlib.awk" -f "$LIB/audit.awk" \
      -v sep="$delim" -v fname="$f" -v bytes="$bytes" -- "$f" >>"$facts"
  done
  "$AWK" -f "$LIB/report_build.awk" -v outdir="$outdir" -- "$facts"
  rm -f "$facts"

  printf 'csvq audit: wrote 3 report(s) to %s/\n' "$outdir" >&2
  local dupes ragged
  read -r dupes ragged <"$outdir/.gate"
  rm -f "$outdir/.gate"
  if [ "$strict" -eq 1 ] && { [ "$dupes" -gt 0 ] || [ "$ragged" -gt 0 ]; }; then
    printf 'csvq audit: --strict gate failed (%s file(s) with dupes, %s with ragged rows)\n' \
      "$dupes" "$ragged" >&2
    exit 1
  fi
}

cmd_stats() {
  local file="" delim=""
  while [ $# -gt 0 ]; do
    case "$1" in
      -d | --delim)
        shift
        delim="${1:-}"
        ;;
      -*) die "stats: unknown option $1" ;;
      *) file="$1" ;;
    esac
    shift
  done
  [ -n "$file" ] || die "stats: missing file"
  require_file "$file"
  "$AWK" -f "$LIB/csvlib.awk" -f "$LIB/stats.awk" -v sep="$delim" -v fname="$file" -- "$file"
}

cmd_validate() {
  local file="" schema="" delim=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --schema)
        shift
        schema="${1:-}"
        ;;
      -d | --delim)
        shift
        delim="${1:-}"
        ;;
      -*) die "validate: unknown option $1" ;;
      *) file="$1" ;;
    esac
    shift
  done
  [ -n "$file" ] || die "validate: missing file"
  [ -n "$schema" ] || die "validate: --schema is required"
  require_file "$file"
  require_file "$schema"
  local gate
  gate="$(mktemp)"
  "$AWK" -f "$LIB/csvlib.awk" -f "$LIB/validate.awk" \
    -v sep="$delim" -v fname="$file" -v schema="$schema" -v gate="$gate" -- "$file"
  local n
  n="$(cat "$gate")"
  rm -f "$gate"
  [ "$n" -eq 0 ] || exit 1
}

cmd_diff() {
  local old="" new="" key="" delim=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --key)
        shift
        key="${1:-}"
        ;;
      -d | --delim)
        shift
        delim="${1:-}"
        ;;
      -*) die "diff: unknown option $1" ;;
      *) if [ -z "$old" ]; then old="$1"; elif [ -z "$new" ]; then new="$1"; else die "diff: too many files"; fi ;;
    esac
    shift
  done
  if [ -z "$old" ] || [ -z "$new" ]; then die "diff: needs OLD and NEW files"; fi
  [ -n "$key" ] || die "diff: --key is required"
  require_file "$old"
  require_file "$new"
  printf 'CSV DIFF (key: %s)\n' "$key"
  printf '======================\n'
  # shellcheck disable=SC2016  # the awk program is single-quoted on purpose
  "$AWK" -f "$LIB/csvlib.awk" -f "$LIB/diff.awk" -v sep="$delim" -v key="$key" -- "$old" "$new" |
    LC_ALL=C sort -t"$TAB" -k2,2 -k1,1 |
    "$AWK" -F"$TAB" '
        $1=="A"{ a++; printf "+ added:   %s | %s\n", $2, $3 }
        $1=="C"{ c++; printf "~ changed: %s | %s\n", $2, $3 }
        $1=="R"{ r++; printf "- removed: %s | %s\n", $2, $3 }
        END { printf "\nSummary: %d added, %d changed, %d removed\n", a+0, c+0, r+0 }'
}

cmd_dedupe() {
  local file="" key="" delim=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --key)
        shift
        key="${1:-}"
        ;;
      -d | --delim)
        shift
        delim="${1:-}"
        ;;
      -*) die "dedupe: unknown option $1" ;;
      *) file="$1" ;;
    esac
    shift
  done
  [ -n "$file" ] || die "dedupe: missing file"
  require_file "$file"
  "$AWK" -f "$LIB/csvlib.awk" -f "$LIB/dedupe.awk" -v sep="$delim" -v key="$key" -- "$file"
}

main() {
  [ $# -ge 1 ] || {
    usage >&2
    exit 2
  }
  local cmd="$1"
  shift
  case "$cmd" in
    audit) cmd_audit "$@" ;;
    stats) cmd_stats "$@" ;;
    validate) cmd_validate "$@" ;;
    diff) cmd_diff "$@" ;;
    dedupe) cmd_dedupe "$@" ;;
    normalize) cmd_normalize "$@" ;;
    version | -V | --version) printf 'csvq %s\n' "$VERSION" ;;
    help | -h | --help) usage ;;
    *) die "unknown command: $cmd (try 'csvq help')" ;;
  esac
}

main "$@"
