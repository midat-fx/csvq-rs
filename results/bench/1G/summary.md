File: 1.06 GiB, 26843545 data rows.

| command | legacy (s) | rust median (s) | speedup | legacy peak RSS (MiB) | rust peak RSS (MiB) | identical output |
|---|---:|---:|---:|---:|---:|---|
| normalize | 38.8 | 0.54 | 72x | 2 | 3 | yes |
| stats | 567.1 | 19.40 | 29x | 9134 | 4033 | yes |
| audit | 480.1 | 7.29 | 66x | 3378 | 2838 | yes |
| validate | 563.4 | 20.03 | 28x | 2953 | 2015 | yes |
| dedupe | 447.2 | 8.37 | 53x | 3384 | 2326 | yes |
| dedupe_key | 426.6 | 2.69 | 159x | 2 | 3 | yes |
| diff | 894.6 | 23.32 | 38x | 5743 | 2427 | yes |
