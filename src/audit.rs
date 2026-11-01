//! `csvq audit`: per-file facts (legacy audit.awk) and the three reports
//! (legacy report_build.awk).
//!
//! The tab-separated fact stream between the two halves is kept on purpose.
//! report_build.awk re-splits each fact line on tabs, so a duplicated row
//! that contains a TAB or a newline is cut short in duplicates_found.txt
//! (legacy bug L4). Re-parsing the same stream reproduces that exactly
//! instead of approximating it.

use crate::awk::atof;
use crate::csv::{delim_name, scan_records, Fields};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufReader, Write};
use std::path::Path;

/// Facts for one file, as audit.awk prints them.
pub fn facts(path: &Path, fname: &[u8], sep_arg: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
    let bytes = fs::metadata(path)?.len();
    let file = BufReader::with_capacity(1 << 20, fs::File::open(path)?);
    let mut sep = sep_arg.to_vec();
    let mut fields = Fields::default();
    let mut total: u64 = 0;
    let mut cols = 0usize;
    let mut ragged: u64 = 0;
    // ponytail: every distinct record is kept in memory, like the awk `dup`
    // array; bounded by the number of distinct rows, not by the file size.
    let mut dup: HashMap<Box<[u8]>, u64> = HashMap::new();
    let crlf = scan_records(file, &mut sep, |rec, sep| {
        fields.parse(rec, sep);
        total += 1;
        if total == 1 {
            cols = fields.len();
        } else if fields.len() != cols {
            ragged += 1;
        }
        match dup.get_mut(rec) {
            Some(n) => *n += 1,
            None => {
                dup.insert(rec.into(), 1);
            }
        }
    })?;
    if sep.is_empty() {
        sep = b",".to_vec();
    }

    let mut w = |parts: &[&[u8]]| {
        for p in parts {
            out.extend_from_slice(p);
        }
        out.push(b'\n');
    };
    let num = |n: u64| n.to_string().into_bytes();
    w(&[b"@FILE\t", fname]);
    w(&[b"@ROWS\t", &num(total.saturating_sub(1))]);
    w(&[b"@COLS\t", &num(cols as u64)]);
    w(&[b"@DELIM\t", delim_name(&sep)]);
    w(&[b"@CRLF\t", &num(u64::from(crlf > 0)), b"\t", &num(crlf)]);
    w(&[b"@RAGGED\t", &num(ragged)]);
    w(&[b"@BYTES\t", &num(bytes)]);
    w(&[b"@EMPTY\t", &num(u64::from(total == 0))]);

    let mut dups: Vec<(&[u8], u64)> = dup
        .iter()
        .filter(|(_, &n)| n > 1)
        .map(|(k, &n)| (&k[..], n))
        .collect();
    let extra: u64 = dups.iter().map(|(_, n)| n - 1).sum();
    w(&[b"@DUPGROUPS\t", &num(dups.len() as u64)]);
    w(&[b"@DUPROWS\t", &num(extra)]);
    // Top 5 by count desc, then record bytes asc (awk string <, C locale).
    dups.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    for (rec, n) in dups.iter().take(5) {
        let shown: Vec<u8> = if rec.len() > 80 {
            [&rec[..77], b"..."].concat()
        } else {
            rec.to_vec()
        };
        w(&[b"#DUP\t", &num(*n), b"\t", &shown]);
    }
    w(&[b"@EOB"]);
    Ok(())
}

/// Gate counters for `--strict`: files with duplicates, files with ragged rows.
pub struct Gate {
    pub files_dupes: u64,
    pub files_ragged: u64,
}

/// report_build.awk: render the three reports into `outdir` from the facts.
pub fn build_reports(facts: &[u8], outdir: &Path) -> io::Result<Gate> {
    let mut a: Vec<u8> =
        b"CSV DATA QUALITY - ANALYSIS REPORT\n=================================\n".to_vec();
    let mut d: Vec<u8> =
        b"CSV DATA QUALITY - DUPLICATES REPORT\n===================================\n".to_vec();

    let mut f: Vec<u8> = Vec::new();
    let (mut rows, mut cols, mut crlf, mut crlfn, mut ragged, mut bytes, mut empty) =
        (0f64, 0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    let (mut dgroups, mut drows) = (0f64, 0f64);
    let mut delim: Vec<u8> = Vec::new();
    let mut dupbuf: Vec<u8> = Vec::new();
    let (mut totrows, mut totbytes, mut totdup) = (0f64, 0f64, 0f64);
    let (mut files_crlf, mut files_ragged, mut files_empty, mut files_dupes) =
        (0u64, 0u64, 0u64, 0u64);
    let mut nfiles = 0u64;
    let mut mincols = -1f64;
    let mut maxcols = 0f64;

    // awk %d of a number: truncate toward zero.
    let d_ = |x: f64| (x as i64).to_string();

    let body = facts.strip_suffix(b"\n").unwrap_or(facts);
    for line in body.split(|&b| b == b'\n') {
        let fl: Vec<&[u8]> = line.split(|&b| b == b'\t').collect();
        let fld = |i: usize| -> &[u8] { fl.get(i - 1).copied().unwrap_or(b"") };
        let n2 = atof(fld(2));
        match fld(1) {
            b"@FILE" => f = fld(2).to_vec(),
            b"@ROWS" => {
                rows = n2;
                totrows += n2;
            }
            b"@COLS" => cols = n2,
            b"@DELIM" => delim = fld(2).to_vec(),
            b"@CRLF" => {
                crlf = n2;
                crlfn = atof(fld(3));
                files_crlf += u64::from(n2 == 1.0);
            }
            b"@RAGGED" => {
                ragged = n2;
                files_ragged += u64::from(n2 > 0.0);
            }
            b"@BYTES" => {
                bytes = n2;
                totbytes += n2;
            }
            b"@EMPTY" => {
                empty = n2;
                files_empty += u64::from(n2 == 1.0);
            }
            b"@DUPGROUPS" => {
                dgroups = n2;
                files_dupes += u64::from(n2 > 0.0);
            }
            b"@DUPROWS" => {
                drows = n2;
                totdup += n2;
                dupbuf.clear();
            }
            b"#DUP" => {
                dupbuf.extend_from_slice(format!("    {:>6}  ", d_(n2)).as_bytes());
                dupbuf.extend_from_slice(fld(3));
                dupbuf.push(b'\n');
            }
            b"@EOB" => {
                nfiles += 1;
                if mincols == -1.0 || cols < mincols {
                    mincols = cols;
                }
                if cols > maxcols {
                    maxcols = cols;
                }
                a.extend_from_slice(b"\nFile:        ");
                a.extend_from_slice(&f);
                a.extend_from_slice(
                    format!(
                        "\n  rows:      {} (excluding header)\n  columns:   {}\n  delimiter: ",
                        d_(rows),
                        d_(cols)
                    )
                    .as_bytes(),
                );
                a.extend_from_slice(&delim);
                a.extend_from_slice(
                    format!(
                        "\n  crlf:      {} ({} line(s))\n  ragged:    {} row(s)\n  bytes:     {}\n  empty:     {}\n",
                        if crlf == 1.0 { "yes" } else { "no" },
                        d_(crlfn),
                        d_(ragged),
                        d_(bytes),
                        if empty == 1.0 { "yes" } else { "no" }
                    )
                    .as_bytes(),
                );
                d.extend_from_slice(b"\nFile: ");
                d.extend_from_slice(&f);
                d.extend_from_slice(
                    format!(
                        "\n  duplicate groups: {}\n  duplicate rows:   {}\n",
                        d_(dgroups),
                        d_(drows)
                    )
                    .as_bytes(),
                );
                if dgroups > 0.0 {
                    d.extend_from_slice(b"  top duplicated rows (count, row):\n");
                    d.extend_from_slice(&dupbuf);
                } else {
                    d.extend_from_slice(b"  no duplicate rows\n");
                }
            }
            _ => {}
        }
    }

    let s = format!(
        "CSV DATA QUALITY - SUMMARY\n=========================\n\
         files:              {}\n\
         total data rows:    {}\n\
         total bytes:        {}\n\
         columns min/max:    {} / {}\n\
         files with crlf:    {}\n\
         files with dupes:   {}\n\
         files with ragged:  {}\n\
         files empty:        {}\n\
         total duplicate rows: {}\n",
        nfiles,
        d_(totrows),
        d_(totbytes),
        if mincols == -1.0 {
            "0".into()
        } else {
            d_(mincols)
        },
        d_(maxcols),
        files_crlf,
        files_dupes,
        files_ragged,
        files_empty,
        d_(totdup),
    );
    fs::File::create(outdir.join("analysis_report.txt"))?.write_all(&a)?;
    fs::File::create(outdir.join("data_summary.txt"))?.write_all(s.as_bytes())?;
    fs::File::create(outdir.join("duplicates_found.txt"))?.write_all(&d)?;
    Ok(Gate {
        files_dupes,
        files_ragged,
    })
}
