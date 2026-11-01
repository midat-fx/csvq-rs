# stats.awk — per-column statistics for `csvq stats`.
# Run as: awk -f lib/csvlib.awk -f lib/stats.awk -v sep=SEP -v fname=NAME FILE

function process(rec,   nf, c, v, t, key) {
  nf = csv_parse(rec, F, sep)
  total++
  if (total == 1) {
    cols = nf
    for (c = 1; c <= nf; c++) name[c] = F[c]
    return
  }
  datarows++
  for (c = 1; c <= cols; c++) {
    v = (c <= nf ? F[c] : "")
    if (v == "") { nulls[c]++; continue }
    nn[c]++
    t = csv_type(v)
    if (t == "int")        { vint[c]++; vfloat[c]++ }
    else if (t == "float") { vfloat[c]++ }
    else if (t == "bool")  { vbool[c]++ }
    else if (t == "date")  { vdate[c]++ }
    else                   { vstr[c]++ }
    key = c SUBSEP v
    if (!(key in seen)) {
      seen[key] = 1; distinct[c]++
      if (vcount[c] < 500) vals[c, ++vcount[c]] = v   # bounded sample for top-values
    }
    freq[key]++
    if (t == "int" || t == "float") {
      num = v + 0
      sum[c] += num; ncnt[c]++
      if (!(c in hasmin) || num < minv[c]) { minv[c] = num; mins[c] = v; hasmin[c] = 1 }
      if (!(c in hasmax) || num > maxv[c]) { maxv[c] = num; maxs[c] = v; hasmax[c] = 1 }
    }
  }
}

function coltype(c) {
  if (nn[c] == 0) return "empty"
  if (vint[c] == nn[c]) return "int"
  if (vfloat[c] == nn[c]) return "float"
  if (vbool[c] == nn[c]) return "bool"
  if (vdate[c] == nn[c]) return "date"
  return "string"
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

END {
  if (inrec) process(buf)
  print "CSV COLUMN STATISTICS"
  print "====================="
  printf "File: %s\n", fname
  printf "Rows: %d (excluding header)\n", datarows + 0

  for (c = 1; c <= cols; c++) {
    t = coltype(c)
    print ""
    printf "Column %d: %s\n", c, name[c]
    printf "  type:      %s\n", t
    printf "  non-null:  %d\n", nn[c] + 0
    rate = (datarows > 0 ? (nulls[c] * 100.0 / datarows) : 0)
    printf "  nulls:     %d (%.1f%%)\n", nulls[c] + 0, rate
    printf "  distinct:  %d\n", distinct[c] + 0
    if ((t == "int" || t == "float") && ncnt[c] > 0) {
      printf "  min:       %s\n", mins[c]
      printf "  max:       %s\n", maxs[c]
      printf "  mean:      %.4f\n", sum[c] / ncnt[c]
    }
    # Top 3 values by frequency (desc), value (asc) tie-break. Only computed
    # for low-cardinality columns; iterates that column's values, not all.
    if (distinct[c] > 0 && distinct[c] <= 200) {
      line3 = ""
      for (r = 1; r <= 3; r++) {
        bestf = -1; bestv = ""; bestj = 0
        for (j = 1; j <= vcount[c]; j++) {
          if ((c, j) in topused) continue
          vv = vals[c, j]; f = freq[c SUBSEP vv]
          if (bestf == -1 || f > bestf || (f == bestf && vv < bestv)) { bestf = f; bestv = vv; bestj = j }
        }
        if (bestf == -1) break
        topused[c, bestj] = 1
        line3 = line3 (line3 == "" ? "" : ", ") sprintf("%s (%d)", bestv, bestf)
      }
      if (line3 != "") printf "  top values: %s\n", line3
    } else if (distinct[c] > 200) {
      print "  top values: (high cardinality; omitted)"
    }
  }
}
