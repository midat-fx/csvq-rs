# csvq-rs

A Rust port of [csvq](https://github.com/midat-fx/csv-data-quality-cli), my
CSV data-quality CLI written in Bash and POSIX awk, with evidence that the
two produce the same bytes: the legacy golden suite, a differential fuzzer
that runs both implementations on generated inputs, and a corpus of frozen
edge cases.

The port is deliberately **bug-compatible**. The fuzzer found real bugs in
the original (an empty file breaks `diff`, numerically equal lines are
"unchanged", schema flags leak between rules, and more). They are written
down in [docs/DIVERGENCES.md](docs/DIVERGENCES.md) and reproduced, not fixed,
so that any mismatch the fuzzer reports is a bug in the port. Why, and the
other design decisions, are in [docs/DESIGN.md](docs/DESIGN.md).

```
make test        # unit + golden + frozen-case tests, then a 1,500-case fuzz run
make fuzz-full   # 50,000 fuzz cases, writes results/fuzz/
make fuzz-smoke ONLY=diff CASES=2000   # only diff cases (any of audit,cli,dedupe,diff,normalize,stats,validate)
make bench       # SIZE=1G by default, writes results/bench/
```

Requirements: Rust 1.82+, and for the fuzz and benchmark targets `bash` plus
BWK awk (the macOS `/usr/bin/awk`), which is the reference implementation.
`cargo test` alone needs neither.

## Commands

Same CLI as the original: `audit`, `stats`, `validate`, `diff`, `dedupe`,
`normalize`, `version`, `help`, with the same flags, messages and exit codes
(2 for usage errors, 1 for failed gates).

```
csvq stats data.csv
csvq validate people.csv --schema people.schema
csvq diff old.csv new.csv --key id
csvq audit data/ -o reports --strict
```

## How equivalence is checked

1. **Legacy golden suite.** The original's fixtures and expected outputs,
   replayed against the Rust binary (`tests/golden.rs`).
2. **Differential fuzzing.** `difffuzz` generates a CSV (or several), a
   schema and a command line, runs it through `legacy/csvq.sh` under BWK awk
   and through the Rust binary in separate scratch directories, and compares
   stdout, stderr, exit status and every file left behind. Inputs are drawn
   from pools of known-hard values: quotes in odd places, CRLF and mixed line
   endings, a BOM, empty and ragged rows, 100 KB fields, numeric look-alikes
   (`0x1A`, `-0`, `1e400`, `nan`), invalid UTF-8, awk's SUBSEP byte, broken
   schemas and broken command lines. A mismatch is shrunk to a minimal
   reproducer. The fuzzer has its own test (`tests/difffuzz_selftest.rs`).
   Run on two copies of the same binary, it must report nothing. Run on a
   planted bug, a wrapper that adds one byte of output whenever an input
   contains a CR, it must report the bug and shrink the case to at most
   16 bytes that still contain the CR.
3. **Frozen cases.** Every reproducer, plus a probe for each known legacy
   quirk, is stored with the legacy output (`tests/cases/`, made by
   `tests/freeze.sh`) and replayed by `cargo test` without needing awk.
   [docs/FUZZING.md](docs/FUZZING.md) walks through getting from a red fuzz
   run to a fixed, frozen case.

The legacy code under `legacy/` is an unmodified copy of the original at
commit `85721ad`, checksummed by `make legacy-check`.

## Results

All numbers below come from runs on one machine (Apple M4 Max, 16 cores,
48 GiB, macOS 26.6, BWK awk 20200816, rustc 1.97). The raw output behind
each one is in `results/`.

### Equivalence

| check | result | raw data |
|---|---|---|
| legacy golden suite | all cases pass | `cargo test --test golden` |
| differential fuzzing, Rust vs legacy under BWK awk | **50,000 of 50,000 cases byte-identical**, 0 timeouts (seed 2026, 1,141 s) | `results/fuzz/rust-vs-bwk.{txt,json,meta}` |
| frozen cases (reproducers and quirk probes) | 28 cases, replayed by `cargo test` | `tests/cases/` |
| benchmark outputs at 1 GiB | identical md5 for all seven commands | `results/bench/1G/runs.tsv` |

Per command in the 50,000-case run: audit 7,766, cli 3,891, dedupe 7,679,
diff 7,611, normalize 3,832, stats 11,665, validate 7,556.

Before that run, the fuzzer found five bugs in the port that the legacy
golden suite had not caught (P1–P5). Along the way it also found legacy bugs
that the port reproduces on purpose (L1–L19), and five deliberate
differences outside the output contract (D1–D5).
All are listed with reproducers in
[docs/DIVERGENCES.md](docs/DIVERGENCES.md).

The same fuzzer also checks the original's claim of identical output "on
any POSIX awk". The legacy script under gawk 5.4.1 differs from itself under
BWK awk in 218 of 5,000 cases, and under mawk 1.3.4 in 243 of 5,000
(`results/fuzz/bwk-vs-{gawk,mawk}.txt`). That is why the oracle is pinned to
one awk.

### Speed

Generated data from the legacy benchmark generator (`legacy/bench/gen.awk`,
5 columns, about 40 bytes per row). Legacy was run once per command; Rust
ran four times, and the table shows the median of the last three. The
machine was shared with other work: the 1-minute load average ranged from 8
to 116 during the 1 GiB run and from 30 to 51 during the 3 GiB run (it is
logged next to every row in `runs.tsv`), so treat the ratios as rough.

**1.06 GiB, 26.8 M rows** (`results/bench/1G/`):

| command | legacy (s) | Rust (s) | speedup | legacy peak RSS | Rust peak RSS | same output |
|---|---:|---:|---:|---:|---:|---|
| normalize | 39.4 | 0.61 | 65x | 2 MiB | 3 MiB | yes |
| stats | 1861.5 | 24.66 | 75x | 1594 MiB | 3310 MiB | yes |
| audit | 786.2 | 29.20 | 27x | 3168 MiB | 2038 MiB | yes |
| validate | 680.6 | 29.40 | 23x | 2509 MiB | 1411 MiB | yes |
| dedupe | 751.3 | 26.18 | 29x | 2923 MiB | 1781 MiB | yes |
| dedupe --key city | 467.3 | 2.61 | 179x | 2 MiB | 3 MiB | yes |
| diff --key id | 1022.1 | 271.95 | 4x | 2610 MiB | 3307 MiB | yes |

**3.21 GiB, 80.5 M rows**, for the two commands whose memory does not grow
with the input (`results/bench/3G/`):

| command | legacy (s) | Rust (s) | speedup | legacy peak RSS | Rust peak RSS | same output |
|---|---:|---:|---:|---:|---:|---|
| normalize | 150.6 | 3.57 | 42x | 2 MiB | 3 MiB | yes |
| dedupe --key city | 1262.7 | 7.25 | 174x | 2 MiB | 3 MiB | yes |

A 10 GB run was planned but not done: the machine had 11 GiB of
free disk at the time, and `bench/run.sh` needs the file plus 4 GiB of
headroom. From 1 GiB to 3 GiB, peak memory of both commands stayed at
3 MiB. For three times the data, Rust time grew 2.8x for `dedupe --key` and
5.9x for `normalize` (0.61 s to 3.57 s, a run short enough to be sensitive to
load and page cache). The other five commands already peaked at 1.4–3.2 GiB of RSS at 1 GiB, and that
grows with the data in both implementations (DESIGN.md §9).

`diff` gained least in this run. Its Rust time was mostly kernel time
(sys 98–190 s, user 40–43 s), and wall time varied from 224 to 368 s between
runs. It used to read both files whole into memory, then copy every OLD line
again into the key map. Now NEW is streamed and OLD lines are moved into the
map. In a before/after run on the same 1 GiB data
(`results/bench/diff-1G/`), the median went from 85.7 s to 48.5 s, user time
from 33.4 s to 21.7 s and sys time from 27.2 s to 8.8 s, with the same
output. The load was higher during the after run. A second change keeps OLD
rows in one byte arena (`results/bench/diff-1G-arena/`): at a load of 3–5,
peak RSS fell from 5.05–5.68 GiB to 2.80–4.18 GiB, with the same output and
the same time (15.75 s vs 15.44 s). `stats` keeps every
distinct value per column, as the awk arrays do, and used about twice awk's memory on this data.

**Long multi-line records** (`results/quadratic/times.txt`). The input is the
frozen case `edge-100k-field-audit`: 228 KB and 28,654 physical lines, but
only 3 logical records, because one quoted field holds about 14,000 newlines.

| command | legacy (s) | Rust (s) | same output |
|---|---:|---:|---|
| audit | 309.5 | < 0.01 | yes |
| stats | 304.4 | 0.01 | yes |

The legacy reader re-counts quotes across the whole joined record on every
physical line, so its cost grows with the square of the record length.

## Layout

```
src/main.rs          the csvq.sh wrapper: argument parsing, messages, exit codes
src/csv.rs           the legacy CSV model: quote-parity record joining, field parser, types
src/awk.rs           awk semantics the output depends on: strtod, %g, strnums, -v escapes, ERE translation
src/{audit,stats,validate,diff,dedupe}.rs   one module per legacy awk program
src/bin/difffuzz.rs  the differential fuzzer
tests/golden.rs      the legacy golden suite
tests/regressions.rs replays tests/cases/*
tests/difffuzz_selftest.rs  checks that the fuzzer finds and shrinks a planted bug
legacy/              the original scripts (oracle), unmodified
bench/               benchmark harness
results/             raw fuzz and benchmark output behind every number above
```

## How it was built

The code, tests and this README were written by Claude Code, Anthropic's AI coding agent, working to Midat's problem statement: Midat chose the problem, set the requirements, reviewed the work and accepted it. Every number above comes from the files in this repository and the commands that produced them.

## License

MIT, like the original.
