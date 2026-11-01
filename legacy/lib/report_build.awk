# report_build.awk — assemble the three csvq audit reports from the fact stream.
# Run as: awk -f lib/report_build.awk -v outdir=DIR facts
# Writes analysis_report.txt, data_summary.txt, duplicates_found.txt into outdir.
# Output is deterministic: files are visited in input order, duplicates are
# pre-sorted by audit.awk, and no timestamps or paths-of-the-day are emitted.

BEGIN {
  FS = "\t"
  A = outdir "/analysis_report.txt"
  S = outdir "/data_summary.txt"
  D = outdir "/duplicates_found.txt"
  print "CSV DATA QUALITY - ANALYSIS REPORT" > A
  print "=================================" >> A
  print "CSV DATA QUALITY - DUPLICATES REPORT" > D
  print "===================================" >> D
  mincols = -1
}

$1 == "@FILE"      { f = $2 }
$1 == "@ROWS"      { rows = $2; totrows += $2 }
$1 == "@COLS"      { cols = $2 }
$1 == "@DELIM"     { delim = $2 }
$1 == "@CRLF"      { crlf = $2; crlfn = $3; if ($2 == 1) files_crlf++ }
$1 == "@RAGGED"    { ragged = $2; if ($2 > 0) files_ragged++ }
$1 == "@BYTES"     { bytes = $2; totbytes += $2 }
$1 == "@EMPTY"     { empty = $2; if ($2 == 1) files_empty++ }
$1 == "@DUPGROUPS" { dgroups = $2; if ($2 > 0) files_dupes++ }
$1 == "@DUPROWS"   { drows = $2; totdup += $2; dupbuf = "" }
$1 == "#DUP"       { dupbuf = dupbuf sprintf("    %6d  %s\n", $2, $3) }

$1 == "@EOB" {
  nfiles++
  if (mincols == -1 || cols < mincols) mincols = cols
  if (cols > maxcols) maxcols = cols

  print "" >> A
  printf "File:        %s\n", f >> A
  printf "  rows:      %d (excluding header)\n", rows >> A
  printf "  columns:   %d\n", cols >> A
  printf "  delimiter: %s\n", delim >> A
  printf "  crlf:      %s (%d line(s))\n", (crlf == 1 ? "yes" : "no"), crlfn >> A
  printf "  ragged:    %d row(s)\n", ragged >> A
  printf "  bytes:     %d\n", bytes >> A
  printf "  empty:     %s\n", (empty == 1 ? "yes" : "no") >> A

  print "" >> D
  printf "File: %s\n", f >> D
  printf "  duplicate groups: %d\n", dgroups >> D
  printf "  duplicate rows:   %d\n", drows >> D
  if (dgroups > 0) {
    print "  top duplicated rows (count, row):" >> D
    printf "%s", dupbuf >> D
  } else {
    print "  no duplicate rows" >> D
  }
}

END {
  print "CSV DATA QUALITY - SUMMARY" > S
  print "=========================" >> S
  printf "files:              %d\n", nfiles + 0 >> S
  printf "total data rows:    %d\n", totrows + 0 >> S
  printf "total bytes:        %d\n", totbytes + 0 >> S
  printf "columns min/max:    %d / %d\n", (mincols == -1 ? 0 : mincols), maxcols + 0 >> S
  printf "files with crlf:    %d\n", files_crlf + 0 >> S
  printf "files with dupes:   %d\n", files_dupes + 0 >> S
  printf "files with ragged:  %d\n", files_ragged + 0 >> S
  printf "files empty:        %d\n", files_empty + 0 >> S
  printf "total duplicate rows: %d\n", totdup + 0 >> S

  # Exit status is decided by the shell wrapper; expose gate counters here.
  gatefile = outdir "/.gate"
  printf "%d %d\n", files_dupes + 0, files_ragged + 0 > gatefile
}
