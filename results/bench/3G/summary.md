File: 3.21 GiB, 80530636 data rows.

| command | legacy (s) | rust median (s) | speedup | legacy peak RSS (MiB) | rust peak RSS (MiB) | identical output |
|---|---:|---:|---:|---:|---:|---|
| normalize | 150.6 | 3.57 | 42x | 2 | 3 | yes |
| dedupe_key | 1262.7 | 7.25 | 174x | 2 | 3 | yes |
