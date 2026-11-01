//! `csvq diff`: keyed row diff (legacy diff.awk + the `sort | awk` pipeline
//! in csvq.sh). Line-oriented on purpose, like the original: one record per
//! physical line, so quoted newlines are not joined here.

use crate::awk::{is_number, split_commas};
use crate::csv::{detect_delim, Fields, Lines};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};

/// A physical line and whether `sub(/\r$/)` removed a CR from it. awk's
/// `line = $0` keeps $0's numeric-string status unless the sub actually
/// fired (sub assigns a plain string).
struct Line {
    text: Vec<u8>,
    cr: bool,
}

/// `oldrow[k] != line`: two strnums compare numerically (by the sign of
/// their difference, so NaN equals anything), otherwise as strings. Lines
/// that are numerically equal but textually different ("1" vs "1.0") are
/// therefore never reported as changed (legacy bug L3, preserved).
/// Equal bytes are never "different" either way, so the strnum check only
/// runs when the bytes differ.
// Not `x - y != 0.0`: that is true for NaN, and awk says NaN is equal.
#[allow(clippy::double_comparisons)]
fn differs(old: &Line, new: &[u8], new_cr: bool) -> bool {
    if old.text == new {
        return false;
    }
    let num = |t: &[u8], cr: bool| if cr { None } else { is_number(t) };
    match (num(&old.text, old.cr), num(new, new_cr)) {
        (Some(x), Some(y)) => x - y < 0.0 || x - y > 0.0,
        _ => true,
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

/// OLD rows by key; the flag records whether NEW had the key. Later rows
/// with the same key overwrite earlier ones, like `oldrow[k] = line`.
type OldRows = HashMap<Vec<u8>, (Line, bool)>;

/// Load a header line and the rows under it as OLD. Returns false if the
/// input had no lines at all.
fn load_old(
    lines: &mut Lines<impl BufRead>,
    sep: &mut Vec<u8>,
    keys: &[Vec<u8>],
    f: &mut Fields,
    oldrow: &mut OldRows,
) -> io::Result<bool> {
    let Some((head, _)) = lines.next_line()? else {
        return Ok(false);
    };
    if sep.is_empty() {
        *sep = vec![detect_delim(head)];
    }
    f.parse(head, sep);
    let opos = header_pos(f);
    while let Some((l, cr)) = lines.next_line()? {
        let k = keystr(f, l, sep, keys, &opos);
        oldrow.insert(
            k,
            (
                Line {
                    text: l.to_vec(),
                    cr,
                },
                false,
            ),
        );
    }
    Ok(true)
}

pub fn run(
    old: impl BufRead,
    new: impl BufRead,
    sep_arg: &[u8],
    key: &[u8],
    out: &mut impl Write,
) -> io::Result<()> {
    let keys = split_commas(key);
    let mut sep = sep_arg.to_vec();
    let mut f = Fields::default();
    let mut ops: Vec<Vec<u8>> = Vec::new();
    let mut oldrow = OldRows::new();
    let mut new = Lines::new(new);

    // awk's `NR == FNR` idiom: with an empty OLD file it is still true while
    // reading NEW, so NEW is loaded as if it were OLD and every row comes out
    // as "removed" (legacy bug L1, preserved).
    if !load_old(&mut Lines::new(old), &mut sep, &keys, &mut f, &mut oldrow)? {
        load_old(&mut new, &mut sep, &keys, &mut f, &mut oldrow)?;
    }

    if let Some((head, _)) = new.next_line()? {
        f.parse(head, &sep);
        let nh: Vec<Vec<u8>> = (1..=f.len()).map(|i| f.get(i).to_vec()).collect();
        let npos = header_pos(&f);
        let mut fo = Fields::default();
        while let Some((line, cr)) = new.next_line()? {
            let k = keystr(&mut f, line, &sep, &keys, &npos);
            match oldrow.get_mut(&k) {
                None => ops.push([b"A\t", &k[..], b"\t", line].concat()),
                Some((o, seen)) => {
                    *seen = true;
                    if differs(o, line, cr) {
                        // changed_cols: compares by position against NEW's
                        // header names, even when the two headers differ (L2).
                        fo.parse(&o.text, &sep);
                        f.parse(line, &sep);
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
                }
            }
        }
    }
    for (k, (line, seen)) in &oldrow {
        if !seen {
            ops.push([b"R\t", &k[..], b"\t", &line.text].concat());
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
