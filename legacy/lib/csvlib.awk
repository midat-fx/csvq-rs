# csvlib.awk — shared CSV helpers for csvq.
#
# Portable POSIX awk: verified against the one true awk (BSD/macOS),
# GNU awk, and busybox awk. No gawk-only features (no gensub, no asort,
# no --csv). Character-by-character parsing keeps behaviour identical
# across implementations at the cost of raw speed (see docs/benchmarks.md).

# Number of double-quote characters in s (used to detect records that
# span multiple physical lines because a quoted field contains a newline).
function csv_nquotes(s,   t) { t = s; return gsub(/"/, "", t) }

# Literal (non-regex) count of single character ch in s. Used for
# delimiter detection so that regex metacharacters like | are safe.
function csv_count_lit(s, ch,   n, p) {
  n = 0
  p = index(s, ch)
  while (p > 0) {
    n++
    s = substr(s, p + 1)
    p = index(s, ch)
  }
  return n
}

# Guess the delimiter of a header line: the candidate (comma, tab,
# semicolon, pipe) with the most occurrences wins; comma breaks ties.
function csv_detect_delim(line,   cands, i, c, n, best, bestc) {
  cands[1] = ","; cands[2] = "\t"; cands[3] = ";"; cands[4] = "|"
  best = ","; bestc = -1
  for (i = 1; i <= 4; i++) {
    c = cands[i]
    n = csv_count_lit(line, c)
    if (n > bestc) { bestc = n; best = c }
  }
  return best
}

# Human-readable name for a delimiter character.
function csv_delim_name(sep) {
  if (sep == ",")  return "comma"
  if (sep == "\t") return "tab"
  if (sep == ";")  return "semicolon"
  if (sep == "|")  return "pipe"
  return sep
}

# Parse one complete logical record into arr[1..n]; returns n.
# Honours RFC 4180 quoting: fields may be wrapped in double quotes,
# an escaped quote is written "", and quoted fields may contain the
# delimiter or embedded newlines (already joined into rec by the caller).
function csv_parse(rec, arr, sep,   i, c, nx, n, field, inq, len) {
  n = 0; field = ""; inq = 0; len = length(rec)
  for (i = 1; i <= len; i++) {
    c = substr(rec, i, 1)
    if (inq) {
      if (c == "\"") {
        nx = substr(rec, i + 1, 1)
        if (nx == "\"") { field = field "\""; i++ }
        else inq = 0
      } else {
        field = field c
      }
    } else {
      if (c == "\"") inq = 1
      else if (c == sep) { n++; arr[n] = field; field = "" }
      else field = field c
    }
  }
  n++; arr[n] = field
  return n
}

# Classify a scalar value: int, float, bool, date (YYYY-MM-DD), or string.
# Empty strings are reported by the caller as nulls, not here.
function csv_type(v) {
  if (v ~ /^-?[0-9]+$/) return "int"
  if (v ~ /^-?[0-9]*\.[0-9]+$/ || v ~ /^-?[0-9]+\.[0-9]*$/) return "float"
  if (v ~ /^-?[0-9]+[eE][-+]?[0-9]+$/) return "float"
  if (v == "true" || v == "false" || v == "TRUE" || v == "FALSE") return "bool"
  if (v ~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/) return "date"
  return "string"
}
