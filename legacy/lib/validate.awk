# validate.awk — schema validation for `csvq validate`.
# Run as: awk -f lib/csvlib.awk -f lib/validate.awk -v sep=SEP -v fname=NAME -v schema=SCHEMA FILE
#
# Schema DSL (one rule per non-blank, non-comment line):
#   col:<name> type:<int|float|bool|date|string> [required] [unique] [min:<n>] [max:<n>] [regex:<re>]
# regex values may not contain whitespace (see docs/schema-dsl.md).

function load_schema(   line, i, ntok, tok, k, v, name) {
  nrules = 0
  while ((getline line < schema) > 0) {
    sub(/\r$/, "", line)
    if (line ~ /^[ \t]*#/ || line ~ /^[ \t]*$/) continue
    ntok = split(line, tok, /[ \t]+/)
    name = ""
    for (i = 1; i <= ntok; i++) {
      if (tok[i] == "") continue
      if (tok[i] == "required") { creq_pending = 1; continue }
      if (tok[i] == "unique")   { cuniq_pending = 1; continue }
      k = tok[i]; v = ""
      p = index(tok[i], ":")
      if (p > 0) { k = substr(tok[i], 1, p - 1); v = substr(tok[i], p + 1) }
      if (k == "col")   name = v
      else if (k == "type")  pend_type = v
      else if (k == "min")   { pend_min = v; pend_hasmin = 1 }
      else if (k == "max")   { pend_max = v; pend_hasmax = 1 }
      else if (k == "regex") pend_regex = v
    }
    if (name == "") continue
    nrules++
    rname[nrules] = name
    rtype[name] = pend_type
    rreq[name] = creq_pending
    runiq[name] = cuniq_pending
    if (pend_hasmin) { rmin[name] = pend_min + 0; rhasmin[name] = 1 }
    if (pend_hasmax) { rmax[name] = pend_max + 0; rhasmax[name] = 1 }
    rregex[name] = pend_regex
    # reset per-line accumulators
    creq_pending = 0; cuniq_pending = 0; pend_type = ""
    pend_min = ""; pend_max = ""; pend_hasmin = 0; pend_hasmax = 0; pend_regex = ""
  }
  close(schema)
}

function type_ok(v, want) {
  if (want == "" || want == "string") return 1
  t = csv_type(v)
  if (want == "float") return (t == "int" || t == "float")
  return (t == want)
}

function add_viol(msg) {
  nviol++
  if (nviol <= 200) { nshown++; vbuf[nshown] = "  " msg }
}

function process(rec,   nf, i, name, p, v) {
  nf = csv_parse(rec, F, sep)
  total++
  if (total == 1) {
    for (i = 1; i <= nf; i++) pos[F[i]] = i
    return
  }
  drow++
  for (i = 1; i <= nrules; i++) {
    name = rname[i]
    p = pos[name]
    v = (p != "" && p <= nf ? F[p] : "")
    if (p == "") { if (!warned[name]++) { nshown++; vbuf[nshown] = "  schema warning: column '" name "' not found in header" } continue }
    if (v == "") {
      if (rreq[name]) add_viol(sprintf("row %d: column '%s' is required but empty", drow, name))
      continue
    }
    if (!type_ok(v, rtype[name]))
      add_viol(sprintf("row %d: column '%s' value '%s' is not type %s", drow, name, v, rtype[name]))
    if (rhasmin[name] && (v + 0) < rmin[name])
      add_viol(sprintf("row %d: column '%s' value %s < min %s", drow, name, v, rmin[name]))
    if (rhasmax[name] && (v + 0) > rmax[name])
      add_viol(sprintf("row %d: column '%s' value %s > max %s", drow, name, v, rmax[name]))
    if (rregex[name] != "" && v !~ rregex[name])
      add_viol(sprintf("row %d: column '%s' value '%s' fails regex %s", drow, name, v, rregex[name]))
    if (runiq[name]) {
      ukey = name SUBSEP v
      if (ukey in useen) add_viol(sprintf("row %d: column '%s' value '%s' is not unique", drow, name, v))
      else useen[ukey] = 1
    }
  }
}

BEGIN { load_schema() }

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
  print "CSV SCHEMA VALIDATION"
  print "====================="
  printf "File:   %s\n", fname
  printf "Schema: %s\n", schema
  printf "Rules:  %d   Data rows: %d\n", nrules + 0, drow + 0
  print ""
  for (i = 1; i <= nshown; i++) print vbuf[i]
  if (nshown > 0) print ""
  if (nviol == 0) print "OK: no violations"
  else printf "FAIL: %d violation(s)\n", nviol
  # gate signal for the shell wrapper
  print nviol + 0 > (gate == "" ? "/dev/null" : gate)
}
