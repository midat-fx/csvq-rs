#!/usr/bin/env bash
# summarize.sh runs.tsv — markdown table from bench/run.sh output: legacy
# time (one run), Rust median time, speedup, peak RSS, and whether the two
# implementations produced byte-identical output (md5 of stdout or reports).
set -euo pipefail
awk -F'\t' '
  NR == 1 { next }
  { size = $1; rows = $2 }
  $3 == "rust" && $5 > 0 { t[$4, ++n[$4]] = $6; if ($9 > rrss[$4]) rrss[$4] = $9 }
  $3 == "rust" && $5 == 0 { rmd5[$4] = $10; if ($9 > rrss[$4]) rrss[$4] = $9 }
  $3 == "legacy" { lt[$4] = $6; lrss[$4] = $9; lmd5[$4] = $10 }
  !($4 in seen) { seen[$4] = 1; order[++k] = $4 }
  END {
    printf "File: %.2f GiB, %d data rows.\n\n", size / 1073741824, rows
    print "| command | legacy (s) | rust median (s) | speedup | legacy peak RSS (MiB) | rust peak RSS (MiB) | identical output |"
    print "|---|---:|---:|---:|---:|---:|---|"
    for (i = 1; i <= k; i++) {
      c = order[i]; m = n[c]
      # median of the timed Rust runs (insertion sort, m is small)
      for (a = 1; a <= m; a++) v[a] = t[c, a]
      for (a = 2; a <= m; a++) for (b = a; b > 1 && v[b-1] > v[b]; b--) { x = v[b]; v[b] = v[b-1]; v[b-1] = x }
      med = (m % 2) ? v[(m + 1) / 2] : (v[m / 2] + v[m / 2 + 1]) / 2
      if (c in lt) {
        printf "| %s | %.1f | %.2f | %.0fx | %.0f | %.0f | %s |\n", c, lt[c], med, lt[c] / med, lrss[c] / 1048576, rrss[c] / 1048576, (lmd5[c] == rmd5[c] ? "yes" : "NO")
      } else {
        printf "| %s | - | %.2f | - | - | %.0f | - |\n", c, med, rrss[c] / 1048576
      }
    }
  }' "$1"
