# audit.awk — per-file data-quality facts for `csvq audit`.
# Run as: awk -f lib/csvlib.awk -f lib/audit.awk -v sep=SEP -v fname=NAME -v bytes=N FILE
# Emits a tab-separated fact stream (see docs/design.md) consumed by report_build.awk.

function process(rec,   nf) {
  nf = csv_parse(rec, F, sep)
  total++
  if (total == 1) cols = nf
  else if (nf != cols) ragged++
  dup[rec]++
}

{
  line = $0
  if (sub(/\r$/, "", line)) crlf++
  if (!seen) { seen = 1; if (sep == "") sep = csv_detect_delim(line) }
  if (inrec) buf = buf "\n" line
  else { buf = line; inrec = 1 }
  if (csv_nquotes(buf) % 2 == 1) next   # inside a quoted, multi-line field
  process(buf)
  inrec = 0; buf = ""
}

END {
  if (inrec) process(buf)
  if (sep == "") sep = ","

  printf "@FILE\t%s\n", fname
  printf "@ROWS\t%d\n", (total > 0 ? total - 1 : 0)
  printf "@COLS\t%d\n", cols + 0
  printf "@DELIM\t%s\n", csv_delim_name(sep)
  printf "@CRLF\t%d\t%d\n", (crlf > 0 ? 1 : 0), crlf + 0
  printf "@RAGGED\t%d\n", ragged + 0
  printf "@BYTES\t%d\n", bytes + 0
  printf "@EMPTY\t%d\n", (total == 0 ? 1 : 0)

  groups = 0; extra = 0
  for (k in dup) if (dup[k] > 1) { groups++; extra += dup[k] - 1 }
  printf "@DUPGROUPS\t%d\n", groups
  printf "@DUPROWS\t%d\n", extra

  # Top 5 duplicated records by count (desc), record text (asc) as tie-break.
  for (r = 1; r <= 5; r++) {
    bestc = -1; bestk = ""
    for (k in dup) {
      if (dup[k] <= 1 || used[k]) continue
      if (bestc == -1 || dup[k] > bestc || (dup[k] == bestc && k < bestk)) {
        bestc = dup[k]; bestk = k
      }
    }
    if (bestc == -1) break
    used[bestk] = 1
    show = bestk
    if (length(show) > 80) show = substr(show, 1, 77) "..."
    printf "#DUP\t%d\t%s\n", bestc, show
  }
  printf "@EOB\n"
}
