#!/usr/bin/env bash
# bench/run.sh — wall time and peak memory of legacy csvq (BWK awk) vs the
# Rust port on data from the legacy generator (legacy/bench/gen.awk).
#
#   SIZE=1G  bench/run.sh            # both implementations, every command
#   SIZE=10G LEGACY_CMDS="normalize" bench/run.sh
#
# SIZE        target file size (100M, 1G, 10G ...)
# CMDS        commands to time (default: all seven below)
# LEGACY_CMDS commands to also time on the legacy script (default: CMDS)
# RUNS        Rust repetitions, median reported (default 3; legacy runs once)
# OUT         results directory (default results/bench/$SIZE)
# RUST        csvq binary to time (default target/release/csvq)
#
# Data lives in a temp directory that is deleted on exit. For each command
# the stdout (or audit reports) of both implementations is hashed, so every
# timing row also says whether the outputs were byte-identical at scale.
set -euo pipefail
export LC_ALL=C
root="$(cd "$(dirname "$0")/.." && pwd)"
SIZE="${SIZE:-1G}"
CMDS="${CMDS:-normalize stats audit validate dedupe dedupe_key diff}"
LEGACY_CMDS="${LEGACY_CMDS-$CMDS}"
RUNS="${RUNS:-3}"
OUT="${OUT:-$root/results/bench/$SIZE}"
AWK="${AWK:-/usr/bin/awk}"
GENAWK="$(command -v mawk || echo awk)"
RUST="${RUST:-$root/target/release/csvq}"
LEGACY=(bash "$root/legacy/csvq.sh")

case "$SIZE" in
  *G) bytes=$((${SIZE%G} * 1024 * 1024 * 1024)) ;;
  *M) bytes=$((${SIZE%M} * 1024 * 1024)) ;;
  *) echo "SIZE must end in M or G" >&2; exit 2 ;;
esac
[ -x "$RUST" ] || { echo "build first: cargo build --release" >&2; exit 2; }

data="$(mktemp -d "${TMPDIR:-/tmp}/csvq-bench.XXXXXX")"
trap 'rm -rf "$data"' EXIT
copies=1
case " $CMDS " in *" diff "*) copies=2 ;; esac
free_kb=$(df -k "$data" | awk 'NR==2 {print $4}')
need_kb=$((bytes * copies / 1024 + 4 * 1024 * 1024))
if [ "$free_kb" -lt "$need_kb" ]; then
  echo "not enough disk in $data: need ~$((need_kb / 1048576)) GiB" >&2
  exit 1
fi
mkdir -p "$OUT"

# ~40 bytes per generated row at these id widths; the real size is recorded.
rows=$((bytes / 40))
echo "generating $rows rows into $data" >&2
"$GENAWK" -v rows="$rows" -v seed=42 -f "$root/legacy/bench/gen.awk" >"$data/big.csv"
# NEW for diff: every 997th amount changed, every 1999th row dropped, 1000 added.
[ "$copies" = 2 ] && "$GENAWK" -F, -v OFS=, -v rows="$rows" '
  NR > 1 && NR % 1999 == 0 { next }
  NR > 1 && NR % 997 == 0 { $4 = $4 + 1 }
  { print }
  END { for (i = 1; i <= 1000; i++) printf "%d,new_%d,Almaty,1.00,true\n", rows + i, i }' \
  "$data/big.csv" >"$data/new.csv"
printf 'col:id type:int required unique min:1\ncol:city type:string regex:^[A-Z][a-z]+$\ncol:amount type:float min:0 max:999\ncol:active type:bool required\n' \
  >"$data/bench.schema"
actual=$(wc -c <"$data/big.csv" | tr -d ' ')

args_for() {
  case "$1" in
    normalize) echo "normalize big.csv" ;;
    stats) echo "stats big.csv" ;;
    audit) echo "audit big.csv -o reports" ;;
    validate) echo "validate big.csv --schema bench.schema" ;;
    dedupe) echo "dedupe big.csv" ;;
    dedupe_key) echo "dedupe big.csv --key city" ;;
    diff) echo "diff big.csv new.csv --key id" ;;
  esac
}

# run IMPL CMD RUN# HASH(0/1) -> appends a TSV row
run() {
  local impl="$1" cmd="$2" n="$3" hash="$4" t md5 load
  local -a bin
  if [ "$impl" = rust ]; then bin=("$RUST"); else bin=("${LEGACY[@]}"); fi
  t="$data/time.txt"
  rm -rf "$data/reports"
  load=$(sysctl -n vm.loadavg | awk '{print $2}')
  # shellcheck disable=SC2046
  if [ "$hash" = 1 ]; then
    (cd "$data" && CSVQ_AWK="$AWK" /usr/bin/time -l "${bin[@]}" $(args_for "$cmd") 2>"$t" | md5 >"$data/out.md5") || true
    if [ "$cmd" = audit ]; then cat "$data"/reports/*.txt | md5 >"$data/out.md5"; fi
    md5=$(cat "$data/out.md5")
  else
    (cd "$data" && CSVQ_AWK="$AWK" /usr/bin/time -l "${bin[@]}" $(args_for "$cmd") 2>"$t" >/dev/null) || true
    md5=-
  fi
  awk -v impl="$impl" -v cmd="$cmd" -v n="$n" -v md5="$md5" -v load="$load" -v size="$actual" -v rows="$rows" '
    / real / { real = $1; user = $3; sys = $5 }
    /maximum resident set size/ { rss = $1 }
    END { printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n", size, rows, impl, cmd, n, real, user, sys, rss, md5, load }' \
    "$t" >>"$OUT/runs.tsv"
  tail -1 "$OUT/runs.tsv" >&2
}

{
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "commit: $(git -C "$root" rev-parse --short HEAD)"
  echo "machine: $(sysctl -n hw.model) $(sysctl -n machdep.cpu.brand_string 2>/dev/null), $(sysctl -n hw.ncpu) cpus, $(($(sysctl -n hw.memsize) / 1073741824)) GiB"
  echo "os: $(sw_vers -productVersion 2>/dev/null || uname -r)"
  echo "awk: $("$AWK" --version 2>&1 | head -1)"
  echo "rustc: $(rustc --version)"
  echo "file: $actual bytes, $rows data rows"
  echo "load average at start: $(sysctl -n vm.loadavg)"
} >"$OUT/env.txt"
printf 'size\trows\timpl\tcmd\trun\treal_s\tuser_s\tsys_s\tmaxrss_bytes\tout_md5\tload1\n' >"$OUT/runs.tsv"

for cmd in $CMDS; do
  run rust "$cmd" 0 1 # hashed run (also warms the page cache)
  for n in $(seq 1 "$RUNS"); do run rust "$cmd" "$n" 0; done
  case " $LEGACY_CMDS " in *" $cmd "*) run legacy "$cmd" 1 1 ;; esac
done
echo "load average at end: $(sysctl -n vm.loadavg)" >>"$OUT/env.txt"
"$root/bench/summarize.sh" "$OUT/runs.tsv" >"$OUT/summary.md"
cat "$OUT/summary.md"
