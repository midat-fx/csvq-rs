# diff at 1 GiB, before and after streaming NEW

`csvq diff big.csv new.csv --key id` on the 1.06 GiB benchmark file
(26.8 M rows), run by `bench/run.sh` with `CMDS=diff LEGACY_CMDS=`:

- `before/`: the binary built from cf6efdb, passed in with `RUST=`. Both
  files were read into vectors, and every OLD line was cloned into the map.
- `after/`: the working tree with the change to `src/diff.rs` (NEW is
  streamed, OLD lines are moved into the map, and a flag replaces the set of
  NEW keys). `env.txt` says cf6efdb because the change was not committed yet.

Times are medians of runs 1–3. RSS is the range over all four runs. Both come from `runs.tsv`:

| | real (s) | user (s) | sys (s) | peak RSS (MiB) |
|---|---:|---:|---:|---:|
| before | 85.70 | 33.39 | 27.15 | 3523–4775 |
| after | 48.46 | 21.69 | 8.83 | 3000–3738 |

The output md5 is the same in both runs (`7fc80a9b…`), and it matches legacy
in `../1G/runs.tsv`.

The machine was heavily loaded by other work. The 1-minute load average was
26–98 during `before` and 116–134 during `after`, so wall times are noisy.
User time is the steadiest figure: it fell by about a third. Peak RSS barely
moved, because the OLD map still holds every OLD row with its own key and
line allocations.

Equivalence after the change (`fuzz.txt`): the cf6efdb binary and the new one
agree on 100,000 of 100,000 fuzz cases (seed 11), and legacy under BWK awk and
the new one agree on 6,000 of 6,000 (seed 12).
