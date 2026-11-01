# Divergences

Every difference between the legacy tool and the port that came up during
the migration, and what was done about it. Case names refer to frozen
reproducers in `tests/cases/` (input, arguments and the legacy output under
BWK awk), which `cargo test` replays against the Rust binary.

## Port bugs found by the differential fuzzer (fixed)

The first version of the port passed the full legacy golden suite. These are
the mismatches the fuzzer found after that, in the order it found them.

| ID | Input that exposed it | What the port got wrong | Case |
|---|---|---|---|
| P1 | `-d '\q'` | awk 20200816 drops the backslash of an unknown `-v` escape (`\q` → `q`); I had ported a newer awk's rule that keeps it | `p1-v-escape-drops-backslash` |
| P2 | schema `max:-` | `strtod("-")` converts nothing and returns `+0`; the port returned `-0` | `p2-strtod-bare-minus-is-plus-zero` |
| P3 | schema `max:-0` | `pend_max + 0` turns `-0` into `+0`, so the message says `max 0`, not `max -0` | `p3-minus-zero-plus-zero-prints-0` |
| P4 | schema `regex:[z-a]` | in BWK awk an empty bracket class matches the empty string, not nothing | `p4-empty-bracket-matches-empty` |
| P5 | diff of `1` vs `1.0`, or anything vs `nan` | whole lines compare as numbers when both look numeric (see L3); the port compared bytes | `p5-l3-diff-strnum-line-compare` |

## Legacy bugs (preserved for equivalence)

Found by reading the legacy code or by the fuzzer. The port reproduces each
one so the fuzzer can stay a strict oracle (see DESIGN.md §1); they are the
backlog for a behaviour-changing v2.

| ID | Command | Legacy behaviour | Case |
|---|---|---|---|
| L1 | diff | An empty OLD file makes awk's `NR == FNR` test true while reading NEW, so NEW is loaded as OLD and every row is reported **removed** | `l1-diff-empty-old-reports-removed` |
| L2 | diff | Changed columns are compared by position but labelled with NEW's header, so reordered columns produce wrong `col=old->new` pairs | `l2-diff-changed-cols-by-position` |
| L3 | diff | `oldrow[k] != line` compares numeric-looking lines as numbers: `1` vs `1.0` is "unchanged", and any line vs `nan` is "unchanged" | `p5-l3-diff-strnum-line-compare` |
| L4 | audit | A duplicated row containing a TAB is cut at the TAB in duplicates_found.txt (the fact stream is TAB-separated) | `l4-audit-dup-row-cut-at-tab` |
| L5 | audit | A duplicated quoted multi-line row can inject facts into the stream: a field line `@ROWS<TAB>999` adds 999 to the row totals | `l5-audit-fact-stream-injection` |
| L6 | validate | `required`, `unique`, `type:` on a line without `col:` are not reset and leak into the next rule | `l6-schema-flags-leak-to-next-rule` |
| L7 | validate | A column named twice runs its checks twice, and the second `unique` pass flags every value as "not unique" | `l7-duplicate-rule-false-unique` |
| L8 | audit | A missing path is reported, collection stops there, but the files before it are audited and the exit status is 0 | `l8-audit-missing-file-keeps-going` |
| L9 | all | An option missing its value (`stats f.csv -d`) exits 1 with no message (`shift` fails under `set -e`) | `l9-missing-option-value-exit-1` |
| L10 | validate | `min:`/`max:` are checked on non-numeric values too (`abc` is 0, so `abc < min 1` is reported next to the type error) | `l10-min-checked-on-non-numbers` |
| L11 | all | Records are joined while the running count of `"` is odd, wherever the quotes are, so a stray quote in an unquoted field swallows the following lines | `l11-quote-parity-joins-lines` |
| L12 | diff | Duplicate keys in NEW each produce an "added" line | `l12-diff-duplicate-new-keys` |
| L13 | diff | Multi-column keys are joined with `,`, so `("x,y","z")` and `("x","y,z")` collide | `l13-diff-key-join-collision` |
| L14 | dedupe | Output records lose their CRs: dedupe silently converts CRLF files to LF | `l14-dedupe-drops-crlf` |
| L15 | all | A multi-character `-d` never matches (awk compares one character at a time), so every row is one field | `l15-multichar-delim-one-field` |
| L16 | all | A UTF-8 BOM stays glued to the first header name, so `col:id` does not find the first column | `l16-bom-hides-first-column` |
| L17 | audit | The header row takes part in duplicate detection; a data row equal to the header counts as a duplicate and trips `--strict` | `l17-header-counted-as-duplicate` |
| L18 | validate | `0x10`, `inf` and `nan` are numbers in schema bounds and values (BWK uses `strtod`); under gawk and mawk `0x10` and `inf` are 0 | `l18-hex-and-inf-are-numbers` |
| L19 | validate | `\d` in a schema regex is a literal `d` (awk ERE has no `\d`) | `l19-regex-backslash-d-is-literal` |

Command-line cases frozen alongside: `cli-no-args`, `cli-normalize-in-place`,
`cli-audit-dir-trailing-slash` (macOS `find` keeps `d/` as `d/a.csv`).
Long-record cases: `edge-100k-field-audit` and `edge-100k-field-stats`, a
CRLF file whose duplicated row holds a 100 KB quoted field with embedded
quotes, commas and about 14,000 newlines.

## Deliberate differences (not preserved)

Outside the output contract (DESIGN.md §1). None of these inputs is
generated by the fuzzer.

| ID | Input | Legacy | Rust |
|---|---|---|---|
| D1 | A file operand of the form `name=value`, e.g. `stats x=y.csv` | awk treats the operand as a variable assignment and reads stdin instead (reports 0 rows) | reads the file |
| D2 | `normalize --in-place` | rewrites via `mktemp` + `mv`, so the file ends up mode 0600 | keeps the file's permissions |
| D3 | A schema regex that does not compile, e.g. `a(b` | awk's own fatal error text, exit 2 | `csvq: bad regex in schema: ...`, exit 2 |
| D4 | A backslash in the `--schema` or `-o` path | awk opens the escape-processed path (`a\tb` becomes `a<TAB>b`), so the schema silently has zero rules | opens the path as given (displayed names still use awk's escaping, as legacy does) |
| D5 | NUL bytes in input | awk strings are C strings; behaviour differs between awks | not specified; not fuzzed |
