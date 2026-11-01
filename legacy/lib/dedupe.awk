# dedupe.awk — drop duplicate rows for `csvq dedupe`, keeping first occurrence.
# Run as: awk -f lib/csvlib.awk -f lib/dedupe.awk -v sep=SEP [-v key="c1,c2"] FILE
# Header is always kept. With --key, uniqueness is decided by the key columns
# only; otherwise by the whole record. Row order is preserved. A count of
# removed rows is written to stderr.

BEGIN { if (key != "") nkeys = split(key, keyname, ",") }

function process(rec,   nf, i, k) {
  nf = csv_parse(rec, F, sep)
  total++
  if (total == 1) {
    for (i = 1; i <= nf; i++) pos[F[i]] = i
    print rec
    return
  }
  if (nkeys > 0) {
    k = ""
    for (i = 1; i <= nkeys; i++)
      k = k (i == 1 ? "" : SUBSEP) ((keyname[i] in pos) && pos[keyname[i]] <= nf ? F[pos[keyname[i]]] : "")
  } else k = rec
  if (k in seen) { removed++; return }
  seen[k] = 1
  print rec
}

{
  line = $0
  if (sub(/\r$/, "", line)) crlf++
  if (!seenline) { seenline = 1; if (sep == "") sep = csv_detect_delim(line) }
  if (inrec) buf = buf "\n" line
  else { buf = line; inrec = 1 }
  if (csv_nquotes(buf) % 2 == 1) next
  process(buf); inrec = 0; buf = ""
}

END { if (inrec) process(buf); printf "csvq dedupe: removed %d duplicate row(s)\n", removed + 0 > "/dev/stderr" }
