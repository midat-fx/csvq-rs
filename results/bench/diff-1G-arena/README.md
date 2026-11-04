# diff at 1 GiB, before and after the OLD-row arena

`csvq diff big.csv new.csv --key id` on the 1.06 GiB benchmark file
(26.8 M rows), run by `bench/run.sh` with `CMDS=diff LEGACY_CMDS=`:

- `before/`: the binary built from 66d1694, passed in with `RUST=`. Each OLD
  row was a key `Vec`, a line `Vec` and a `HashMap` entry.
- `after/`: 70aad21. Keys and lines are appended to one byte arena, each row
  is a 24-byte record, and the map goes from a 64-bit key hash to a row index.

`env.txt` says 70aad21 in both, because the script records the checkout, not
the binary passed in with `RUST=`.

Times are medians of runs 1–3. RSS is the range over all four runs. Both come
from `runs.tsv`:

| | real (s) | user (s) | sys (s) | peak RSS (GiB) |
|---|---:|---:|---:|---:|
| before | 15.75 | 14.62 | 1.14 | 5.05–5.68 |
| after | 15.44 | 14.83 | 0.60 | 2.80–4.18 |

The output md5 is the same in both (`7fc80a9b…`), and it matches legacy in
`../1G/runs.tsv`. The 1-minute load average was 3–5, much lower than during
`../diff-1G/` (26–134). The "before" binary there peaked at 3.4–4.7 GiB,
lower than here. On macOS, `maxrss` under heavy memory pressure counts
compressed and swapped pages differently, so compare RSS only within one
directory.

Peak RSS fell by 1.5–2.9 GiB; time did not change. In run 0 the "after" binary
peaked at 2.80 GiB and in runs 1–3 at 4.15–4.18 GiB. My guess is the transient
copy when the arena, the row vector or the index doubles, which depends on
where the allocator can grow in place. I did not dig further.

Equivalence (`fuzz.txt`): the 66d1694 binary and 70aad21 agree on 100,000 of
100,000 `--only diff` cases (seed 21). Legacy under BWK awk and 70aad21 agree
on 5,999 of 6,000 (seed 22). The one mismatch is not caused by this change: the
old binary gives the same output on it. It is port bug P6, a CR before a
trailing space in a numeric line, fixed in the next commit and frozen as
`tests/cases/p6-strnum-allows-trailing-cr`.
