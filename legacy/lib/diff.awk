# diff.awk — keyed row diff between two CSVs for `csvq diff`.
# Run as: awk -f lib/csvlib.awk -f lib/diff.awk -v sep=SEP -v key="c1,c2" OLD NEW
# Emits TSV: <op>\t<key>\t<detail>  (op is A/C/R). The wrapper sorts + formats.
# Line-oriented: assumes one record per physical line (no embedded newlines);
# quoted delimiters within a line are still handled. See docs/design.md.

BEGIN { nkeys = split(key, keyname, ",") }

function keystr(line, posmap,   arr, nf, i, s, kn) {
  nf = csv_parse(line, arr, sep); s = ""
  for (i = 1; i <= nkeys; i++) {
    kn = keyname[i]
    s = s (i == 1 ? "" : ",") ((kn in posmap) && posmap[kn] <= nf ? arr[posmap[kn]] : "")
  }
  return s
}

function changed_cols(o, n,   ao, an, i, name, res) {
  csv_parse(o, ao, sep); csv_parse(n, an, sep); res = ""
  for (i = 1; i <= hn; i++) {
    name = NH[i]
    if (ao[i] != an[i]) res = res (res == "" ? "" : "; ") name "=" ao[i] "->" an[i]
  }
  return res
}

{ line = $0; sub(/\r$/, "", line) }

NR == FNR {                       # OLD file
  if (FNR == 1) {
    if (sep == "") sep = csv_detect_delim(line)
    on = csv_parse(line, OH, sep); for (i = 1; i <= on; i++) opos[OH[i]] = i; next
  }
  k = keystr(line, opos); oldrow[k] = line; next
}

FNR == 1 { hn = csv_parse(line, NH, sep); for (i = 1; i <= hn; i++) npos[NH[i]] = i; next }

{
  k = keystr(line, npos); newseen[k] = 1
  if (!(k in oldrow)) print "A\t" k "\t" line
  else if (oldrow[k] != line) print "C\t" k "\t" changed_cols(oldrow[k], line)
}

END { for (k in oldrow) if (!(k in newseen)) print "R\t" k "\t" oldrow[k] }
