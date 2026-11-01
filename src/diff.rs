//! `csvq diff`: keyed row diff (legacy diff.awk + the `sort | awk` pipeline
//! in csvq.sh). Line-oriented on purpose, like the original: one record per
//! physical line, so quoted newlines are not joined here.

use crate::awk::{is_number, split_commas};
use crate::csv::{detect_delim, Fields, Lines};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};

/// A physical line plus its numeric value when awk would treat it as a
/// strnum: `line = $0` keeps $0's numeric-string status unless `sub(/\r$/)`
/// actually removed a CR (sub assigns a plain string).
type Line = (Vec<u8>, Option<f64>);

fn read_lines(r: impl BufRead) -> io::Result<Vec<Line>> {
    let mut lines = Lines::new(r);
    let mut out = Vec::new();
    while let Some((l, cr)) = lines.next_line()? {
        out.push((l.to_vec(), if cr { None } else { is_number(l) }));
    }
    Ok(out)
}

/// `oldrow[k] != line`: two strnums compare numerically (by the sign of
/// their difference, so NaN equals anything), otherwise as strings. Lines
/// that are numerically equal but textually different ("1" vs "1.0") are
/// therefore never reported as changed (legacy bug L3, preserved).
// Not `x - y != 0.0`: that is true for NaN, and awk says NaN is equal.
#[allow(clippy::double_comparisons)]
fn differs(a: &Line, b: &Line) -> bool {
    match (a.1, b.1) {
        (Some(x), Some(y)) => x - y < 0.0 || x - y > 0.0,
        _ => a.0 != b.0,
    }
}

fn header_pos(f: &Fields) -> HashMap<Vec<u8>, usize> {
    (1..=f.len()).map(|i| (f.get(i).to_vec(), i)).collect()
}

fn keystr(
    f: &mut Fields,
    line: &[u8],
    sep: &[u8],
    keys: &[Vec<u8>],
    pos: &HashMap<Vec<u8>, usize>,
) -> Vec<u8> {
    f.parse(line, sep);
    let mut s = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            s.push(b',');
        }
        if let Some(&p) = pos.get(k) {
            s.extend_from_slice(f.get_or_empty(p));
        }
    }
    s
}

pub fn run(
    old: impl BufRead,
    new: impl BufRead,
    sep_arg: &[u8],
    key: &[u8],
    out: &mut impl Write,
) -> io::Result<()> {
    let keys = split_commas(key);
    let mut old_lines = read_lines(old)?;
    let mut new_lines = read_lines(new)?;
    // awk's `NR == FNR` idiom: with an empty OLD file it is still true while
    // reading NEW, so NEW is loaded as if it were OLD and every row comes out
    // as "removed" (legacy bug L1, preserved).
    if old_lines.is_empty() {
        old_lines = std::mem::take(&mut new_lines);
    }

    let mut sep = sep_arg.to_vec();
    let mut f = Fields::default();
    let mut ops: Vec<Vec<u8>> = Vec::new();
    let mut oldrow: HashMap<Vec<u8>, Line> = HashMap::new();

    if let Some((head, rows)) = old_lines.split_first() {
        if sep.is_empty() {
            sep = vec![detect_delim(&head.0)];
        }
        f.parse(&head.0, &sep);
        let opos = header_pos(&f);
        for line in rows {
            let k = keystr(&mut f, &line.0, &sep, &keys, &opos);
            oldrow.insert(k, line.clone());
        }
    }
    let mut newseen: HashSet<Vec<u8>> = HashSet::new();
    if let Some((head, rows)) = new_lines.split_first() {
        f.parse(&head.0, &sep);
        let nh: Vec<Vec<u8>> = (1..=f.len()).map(|i| f.get(i).to_vec()).collect();
        let npos = header_pos(&f);
        let mut fo = Fields::default();
        for line in rows {
            let k = keystr(&mut f, &line.0, &sep, &keys, &npos);
            newseen.insert(k.clone());
            match oldrow.get(&k) {
                None => ops.push([b"A\t", &k[..], b"\t", &line.0].concat()),
                Some(o) if differs(o, line) => {
                    // changed_cols: compares by position against NEW's header
                    // names, even when the two headers differ (L2).
                    fo.parse(&o.0, &sep);
                    f.parse(&line.0, &sep);
                    let mut res = Vec::new();
                    for (i, name) in nh.iter().enumerate() {
                        let (a, b) = (fo.get_or_empty(i + 1), f.get_or_empty(i + 1));
                        if a != b {
                            if !res.is_empty() {
                                res.extend_from_slice(b"; ");
                            }
                            res.extend_from_slice(&[&name[..], b"=", a, b"->", b].concat());
                        }
                    }
                    ops.push([b"C\t", &k[..], b"\t", &res].concat());
                }
                Some(_) => {}
            }
        }
    }
    for (k, line) in &oldrow {
        if !newseen.contains(k) {
            ops.push([b"R\t", &k[..], b"\t", &line.0].concat());
        }
    }

    // LC_ALL=C sort -t TAB -k2,2 -k1,1: field 2, then field 1, then the
    // whole line as the last-resort key.
    let field =
        |l: &[u8], n: usize| -> Vec<u8> { l.split(|&b| b == b'\t').nth(n).unwrap_or(b"").to_vec() };
    ops.sort_by_cached_key(|l| (field(l, 1), field(l, 0), l.clone()));

    let (mut a, mut c, mut r) = (0, 0, 0);
    for l in &ops {
        // awk -F'\t' prints $2 and $3 only: a TAB inside the detail cuts it.
        let label: &[u8] = match l[0] {
            b'A' => {
                a += 1;
                b"+ added:   "
            }
            b'C' => {
                c += 1;
                b"~ changed: "
            }
            _ => {
                r += 1;
                b"- removed: "
            }
        };
        out.write_all(label)?;
        out.write_all(&field(l, 1))?;
        out.write_all(b" | ")?;
        out.write_all(&field(l, 2))?;
        out.write_all(b"\n")?;
    }
    writeln!(out, "\nSummary: {a} added, {c} changed, {r} removed")?;
    Ok(())
}
