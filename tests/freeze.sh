#!/usr/bin/env bash
# freeze.sh NAME INPUT_DIR ARG... — turn a fuzz finding (or any hand-made
# case) into a hermetic regression test: copy INPUT_DIR, run the legacy
# script on it under BWK awk, and store what it produced as the expected
# result. tests/regressions.rs then replays every case against the Rust binary.
set -euo pipefail
export LC_ALL=C
root="$(cd "$(dirname "$0")/.." && pwd)"
name="$1" input="$2"
shift 2
case_dir="$root/tests/cases/$name"
rm -rf "$case_dir"
mkdir -p "$case_dir/expected"
cp -R "$input" "$case_dir/input"
if [ $# -gt 0 ]; then printf '%s\n' "$@" >"$case_dir/args"; else : >"$case_dir/args"; fi
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cp -R "$case_dir/input/." "$work/"
code=0
(cd "$work" && CSVQ_AWK="${AWK:-/usr/bin/awk}" bash "$root/legacy/csvq.sh" "$@" \
  >"$case_dir/expected/stdout" 2>"$case_dir/expected/stderr" </dev/null) || code=$?
echo "$code" >"$case_dir/expected/code"
# Files the command created or changed.
(cd "$work" && find . -type f | LC_ALL=C sort) | while IFS= read -r f; do
  if ! cmp -s "$work/$f" "$case_dir/input/$f" 2>/dev/null; then
    mkdir -p "$(dirname "$case_dir/expected/files/$f")"
    cp "$work/$f" "$case_dir/expected/files/$f"
  fi
done
echo "froze $name (exit $code)"
