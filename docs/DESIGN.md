# Design notes

Why the port looks the way it does. Each section is a decision, the
alternative I rejected, and the reason.

## 1. Bug-compatible first, fixes later

The Rust binary reproduces the legacy tool's observable behaviour exactly,
including its bugs. Fixes are a separate, later step.

The alternative was to "port and improve" in one go. I rejected it because
it destroys the oracle: once the new version is allowed to differ, every
mismatch the fuzzer reports has to be triaged by hand as "intended" or "bug",
and real port bugs hide among intended changes. With strict equivalence, any
mismatch is a defect in the port, full stop. The legacy bugs are not lost:
each one is written down in [DIVERGENCES.md](DIVERGENCES.md) with a frozen
reproducer, so fixing them later is a list of small, reviewable behaviour
changes, each of which updates one frozen case on purpose.

**The contract** that has to match byte for byte: stdout, stderr, the exit
status, and every file the command writes. **Outside the contract**: file
permissions, temp files, the exact text of awk's own fatal errors, and inputs
the legacy tool cannot represent at all (NUL bytes, file names that awk
mistakes for `var=value` assignments). Those are listed as deliberate
differences D1–D5.

## 2. The oracle is one specific awk

The legacy script claims identical output "on any POSIX awk". It is not true
(see the cross-awk numbers in the README), so "equivalent to the legacy tool"
has to name an awk. I picked BWK awk 20200816, the macOS `/usr/bin/awk`:
it is the default awk on the machine the tool was written on and the one its
macOS CI job used. gawk and mawk disagree with it on things like `"0x1A" + 0`
(26 vs 0) and on how `1e20` prints.

## 3. Vendored legacy code, not a submodule

`legacy/` holds an unmodified copy of the original scripts with SHA-256
checksums; `make legacy-check` verifies them. A submodule would make
`make test` depend on network access and on the upstream repository staying
put. The checksums keep the "unmodified" claim checkable.

## 4. Bytes, not strings

Everything is `&[u8]`/`Vec<u8>`; there is no `String` on the data path. The
legacy tool runs under `LC_ALL=C`, so it compares, slices and measures bytes.
Rust `String` would force a UTF-8 decision the original never made, and the
fuzzer feeds invalid UTF-8 on purpose. `length(show) > 80` truncating in the
middle of a multi-byte character is legacy behaviour and is reproduced.

## 5. awk's number semantics are emulated, not approximated

Several outputs depend on how awk turns strings into numbers and back:

- **String to number** is libc `strtod` on the longest prefix (`awk::strtod`):
  leading whitespace, hex floats, `inf`, `nan(...)`, and `+0` (not `-0`) when
  nothing converts.
- **Number to string** is `%.30g` for integral values and `%.6g` otherwise
  (`awk::num_to_str`); Rust's `{:e}` and `{:.N}` formatting are correctly
  rounded with ties to even, the same as macOS printf, which I checked before
  relying on it.
- **Numeric strings ("strnums")**: a line read from input compares as a
  number if it looks like one (`awk::is_number`, BWK's rule: `strtod` consumes
  it, only blanks follow, the result is not `+HUGE_VAL` and not out of range).
  The fuzzer found that this leaks into `diff` (legacy bug L3).
- **Comparisons** use the sign of `x - y`, so NaN equals everything.
- `-0 + 0` is `+0`, which matters when a schema says `max:-0`.

## 6. The audit fact stream is kept as the internal seam

Legacy `audit` is two awk programs: one prints a tab-separated "fact stream"
per file, the other re-reads it and renders three reports. The obvious port
passes structs between the two halves. I kept the text stream instead,
because the re-parse *is* behaviour: a duplicated row that contains a TAB is
cut short in the report (L4), and a quoted multi-line row can inject fake
facts (L5). Re-parsing the same stream reproduces both exactly; emulating them
on top of structs would be a pile of special cases.

## 7. Regexes are translated, not passed through

Schema regexes are awk EREs (BWK dialect). Passing them to the `regex` crate
unchanged would silently change meaning: `\d` is a literal `d` in awk, a
reversed range `[z-a]` matches the empty string, `.` matches a newline, and
the input is bytes. `awk::translate_ere` resolves every escape and every
bracket expression into explicit byte sets, following BWK's `relex` and
`cclenter`, and runs `regex::bytes` with Unicode off and `(?s)`. Only the
operators `^ $ . ( ) | * + ? {n,m}` pass through, because they mean the same
thing in both engines for well-formed patterns. An invalid pattern is a
deliberate difference (D3): awk's error text is not part of the contract.

## 8. The fuzzer is a custom generator, not libFuzzer

The oracle is an external process (`bash` + `awk`, roughly 10–30 ms per
run), so in-process, coverage-guided fuzzing (`cargo fuzz`) does not fit:
the interesting branches are inside awk, not inside the Rust code. Instead
`difffuzz` generates inputs from weighted pools of known-hard values (quotes,
CRLF, BOM, empty fields, huge fields, numeric look-alikes, invalid UTF-8,
SUBSEP bytes, schema quirks, broken command lines) and compares the two
implementations as black boxes:

- every case is generated from `(seed, case index)`, so a run is reproducible
  regardless of thread scheduling;
- each implementation runs in its own scratch directory, and the whole
  directory is snapshotted afterwards, so written reports and in-place edits
  are compared along with stdout, stderr and the exit status;
- a mismatch is shrunk greedily (chunks of lines, then single bytes) while it
  still fails the same way, and saved with a README showing both outputs;
- `tests/freeze.sh` turns a reproducer into a frozen case with the legacy
  output stored next to it, so `cargo test` needs neither bash nor awk.

## 9. Same memory model as the original

Distinct counts, duplicate detection and diff are exact, so like the awk
arrays they keep every distinct value or row in memory. Approximate
structures (hash-only sets, HyperLogLog) would break equivalence on
collisions. That bounds which commands can run on 10 GB of high-cardinality
data on a laptop; the largest benchmark (3 GiB) therefore only covers commands whose
memory does not grow with the input.

## 10. Dependencies

One: `regex`. Hashing uses the standard library's SipHash. A faster hasher
would help `stats` and `audit`, but the port is already one to two orders of
magnitude faster than the original, and every dependency is something to
audit.

## 11. What would change after parity

The legacy bugs in DIVERGENCES.md are the backlog for a v2. The ones I would
fix first: L1 (diffing against an empty file reports everything as removed),
L3 (numerically equal lines treated as unchanged), L6 (schema flags leaking
into the next rule) and L8 (a missing file in `audit` is reported but the run
still exits 0).
