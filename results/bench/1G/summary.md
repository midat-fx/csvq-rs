File: 1.06 GiB, 26843545 data rows.

| command | legacy (s) | rust median (s) | speedup | legacy peak RSS (MiB) | rust peak RSS (MiB) | identical output |
|---|---:|---:|---:|---:|---:|---|
| normalize | 39.4 | 0.61 | 65x | 2 | 3 | yes |
| stats | 1861.5 | 24.66 | 75x | 1594 | 3310 | yes |
| audit | 786.2 | 29.20 | 27x | 3168 | 2038 | yes |
| validate | 680.6 | 29.40 | 23x | 2509 | 1411 | yes |
| dedupe | 751.3 | 26.18 | 29x | 2923 | 1781 | yes |
| dedupe_key | 467.3 | 2.61 | 179x | 2 | 3 | yes |
| diff | 1022.1 | 271.95 | 4x | 2610 | 3307 | yes |
