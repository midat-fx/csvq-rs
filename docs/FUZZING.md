# From a fuzz finding to a frozen case

What I do when `difffuzz` goes red. The commands below are the ones I ran on a
bug I planted on purpose (last section); the output is in
[results/fuzz/walkthrough-planted-bug.txt](../results/fuzz/walkthrough-planted-bug.txt).

## 1. Find the reproducer

A finding prints one line per case and exits 1:

```
DIVERGENCE case 0 [stats] stderr: csvq stats in.csv -> .../fuzz-out/seed1-case0
```

That is the case index, the command bucket, what differed (`stdout`, `stderr`,
exit status or a written file), the shrunk command line, and the directory
holding the reproducer. Several lines from one run are often one bug, so I read
them all before fixing anything. `--max-fail` is checked only when a worker
starts a case, so with `--jobs 4` a run can report a few more than the limit.

`TIMEOUT` lines are something else: a side hit the 30 s limit, which says
nothing about equivalence, and nothing is saved.

When CI is red, the seed is the run number (`--seed ${{ github.run_number }}`
in `ci.yml`) and the reproducers are in the `fuzz-findings` artifact:

```bash
gh run download RUN_ID -n fuzz-findings -D fuzz-out
```

To replay a run locally, pass the same seed and at least as many cases.
`seedS-caseN` always names the same generated input, because each case gets its
own RNG from the seed and the index.

## 2. Read the reproducer

```
fuzz-out/seed1-case0/
  README.txt    what differed, the command, and each side's exit status, stdout, stderr and written files
  input/        the shrunk input files
```

The first section of `README.txt` is `--a`, by default legacy under BWK awk. The
second is `--b`, by default the Rust binary. Before I trust a finding I check
that `--a` really was `legacy:/usr/bin/awk` and that `awk --version` says
20200816. gawk and mawk disagree with BWK awk on their own, so a difference
under them is not a port bug
([DIVERGENCES.md](DIVERGENCES.md#the-legacy-tool-against-itself-on-other-awks)).

## 3. Decide what it is

| The difference is | Change `src/`? | Record in `docs/DIVERGENCES.md` | Freeze? |
|---|---|---|---|
| A **port bug**: I misread what legacy or awk does | yes, match legacy | "Port bugs", next free `P` number | yes, as `pN-...` |
| A **legacy bug**: legacy is wrong and the port accidentally did better | yes, reproduce the bug | "Legacy bugs", next free `L` number | yes, as `lN-...` |
| **Outside the contract** ([DESIGN.md §1](DESIGN.md#1-bug-compatible-first-fixes-later)): permissions, temp files, awk's own fatal error text, inputs legacy cannot represent | no | "Deliberate differences", next free `D` number | no |

The first two both end with the port matching legacy; they differ only in where
the row goes. That row matters for the legacy bugs, because the table is the
list of things a behaviour-changing v2 could fix.

The third kind is not frozen because a frozen case stores legacy's output, so it
would stay red for as long as the port differs by design. The fuzzer would also
keep reporting it, so `gen_case` in `src/bin/difffuzz.rs` has to stop generating
that input. None of the `D` inputs are generated today.

## 4. Freeze it, before fixing

The `command:` line in `README.txt` is shell-quoted, so I paste it after the case
name and the input directory, leaving out `csvq`:

```bash
tests/freeze.sh p6-short-description fuzz-out/seed1-case0/input stats in.csv
```

This copies the input, runs `legacy/csvq.sh` on it under `/usr/bin/awk` (or
`$AWK`), and stores stdout, stderr, the exit code and any written files in
`tests/cases/NAME/`. Three things to watch:

- It deletes an existing `tests/cases/NAME` without asking.
- `args` holds one argument per line, so an argument containing a newline
  cannot be frozen as it stands. Shrink it by hand first.
- `README.txt` prints invalid UTF-8 lossily, and `tests/regressions.rs` reads
  `args` as UTF-8 and panics on anything else. A finding that needs a non-UTF-8
  argument cannot be frozen until that test reads `args` as bytes.

## 5. Watch it fail, fix, watch it pass

```bash
cargo test -j4 --test regressions
```

There is one test over the whole corpus, so a pass does not list case names. A
failure does:

```
2 of 29 cases differ:
l14-dedupe-drops-crlf: stderr
p6-scratch-cr-on-stderr: stderr
```

The new case has to be on that list. If it isn't, the frozen input doesn't
reproduce the bug and the fix would prove nothing. Other cases on the list are
fine if the same bug explains them.

Then I fix `src/`, run the same test until it passes, and rerun the original
fuzz command with the same seed. A legacy-bug case fails the same way until the
port reproduces the bug. Everything goes into one commit: the fix, the case and
the `DIVERGENCES.md` row. `fuzz-out/` is ignored by git.

## Trying this on a planted bug

To rehearse without a real finding, work in a throwaway clone and add this line
at the top of `main()` in `src/main.rs`, just above `let cmd = v.remove(0);`:

```rust
if std::fs::read("in.csv").is_ok_and(|d| d.contains(&b'\r')) { eprint!("X"); }
```

Then:

```bash
cargo build --release -j4
./target/release/difffuzz --cases 300 --seed 1 --jobs 1 --max-fail 1
tests/freeze.sh p6-scratch-cr-on-stderr fuzz-out/seed1-case0/input stats in.csv
cargo test -j4 --test regressions          # fails, names the new case
git checkout src/main.rs                   # the "fix"
cargo test -j4 --test regressions          # passes
cargo build --release -j4
./target/release/difffuzz --cases 300 --seed 1 --jobs 1 --max-fail 1
```

The input shrinks to a single CR byte. Delete the scratch case afterwards; it
guards against nothing real.
