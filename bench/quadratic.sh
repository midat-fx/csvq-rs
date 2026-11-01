#!/usr/bin/env bash
# quadratic.sh — legacy vs Rust on one long quoted multi-line record (the
# frozen case edge-100k-field-audit). The legacy reader re-counts quotes in
# the whole joined buffer for every physical line, and its field parser
# grows strings one character at a time, so its cost is quadratic in the
# record length.
set -euo pipefail
export LC_ALL=C
root="$(cd "$(dirname "$0")/.." && pwd)"
in="$root/tests/cases/edge-100k-field-audit/input/big.csv"
out="$root/results/quadratic"
mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
{
  echo "input: $(wc -c <"$in" | tr -d ' ') bytes, $(awk 'END{print NR}' "$in") physical lines, 3 logical records"
  echo "load average at start: $(sysctl -n vm.loadavg)"
  for cmd in audit stats; do
    for impl in rust legacy; do
      if [ "$impl" = rust ]; then bin=("$root/target/release/csvq"); else bin=(bash "$root/legacy/csvq.sh"); fi
      args=("$cmd" "$in")
      [ "$cmd" = audit ] && args+=(-o "$work/$impl")
      t=$( { /usr/bin/time -p "${bin[@]}" "${args[@]}" >"$work/$impl.$cmd.out"; } 2>&1 | awk '/^real/{print $2}')
      echo "$cmd $impl real_s=$t"
    done
    if [ "$cmd" = audit ]; then cmp -s <(cat "$work"/rust/*.txt) <(cat "$work"/legacy/*.txt) && echo "audit reports identical"; fi
    [ "$cmd" = stats ] && cmp -s "$work/rust.stats.out" "$work/legacy.stats.out" && echo "stats output identical"
  done
} | tee "$out/times.txt"
