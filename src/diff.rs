//! `csvq diff`: keyed row diff (legacy diff.awk + the `sort | awk` pipeline
//! in csvq.sh). Line-oriented on purpose, like the original: one record per
//! physical line, so quoted newlines are not joined here.

use crate::awk::{is_number, split_commas};
use crate::csv::{detect_delim, Fields, Lines};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::io::{self, BufRead, Write};

/// `oldrow[k] != line`, where `cr` says whether `sub(/\r$/)` removed a CR
/// from the line: awk's `line = $0` keeps $0's numeric-string status unless
/// the sub actually fired (sub assigns a plain string). Two strnums compare numerically (by the sign of
/// their difference, so NaN equals anything), otherwise as strings. Lines
/// that are numerically equal but textually different ("1" vs "1.0") are
/// therefore never reported as changed (legacy bug L3, preserved).
/// Equal bytes are never "different" either way, so the strnum check only
/// runs when the bytes differ.
// Not `x - y != 0.0`: that is true for NaN, and awk says NaN is equal.
#[allow(clippy::double_comparisons)]
fn differs(old: &[u8], old_cr: bool, new: &[u8], new_cr: bool) -> bool {
    if old == new {
        return false;
    }
    let num = |t: &[u8], cr: bool| if cr { None } else { is_number(t) };
    match (num(old, old_cr), num(new, new_cr)) {
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

/// The map key is already a hash, so the map uses it as is.
#[derive(Default)]
struct PassThrough(u64);
impl Hasher for PassThrough {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!("only u64 keys")
    }
    fn write_u64(&mut self, h: u64) {
        self.0 = h;
    }
}

/// 64-bit multiply-xorshift hash of a key. Collisions only cost a byte
/// comparison, never a wrong answer, so it only needs to spread well.
fn hash(key: &[u8]) -> u64 {
    let mut h = 0x9E37_79B9_7F4A_7C15u64 ^ key.len() as u64;
    let mut chunks = key.chunks_exact(8);
    for c in &mut chunks {
        h = (h ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 31;
    }
    for &b in chunks.remainder() {
        h = (h ^ u64::from(b)).wrapping_mul(0x94D0_49BB_1331_11EB);
    }
    h ^= h >> 32;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^ (h >> 32)
}

const NONE: u32 = u32::MAX;

/// One OLD row: its key and then its line, both in the arena at `at`.
struct Row {
    at: usize,
    key_len: u32,
    line_len: u32,
    /// Next row whose key has the same hash.
    next: u32,
    cr: bool,
    /// NEW had this key.
    seen: bool,
}

/// OLD rows by key, like awk's `oldrow[k] = line`: a later row with the same
/// key replaces the earlier one. All bytes live in one arena and the map goes
/// from key hash to the first row with that hash, so a row costs its bytes
/// plus about 50, instead of two heap allocations and a map entry
/// (DESIGN.md §9).
#[derive(Default)]
struct OldRows {
    arena: Vec<u8>,
    rows: Vec<Row>,
    index: HashMap<u64, u32, BuildHasherDefault<PassThrough>>,
}

impl OldRows {
    fn key(&self, r: &Row) -> &[u8] {
        &self.arena[r.at..r.at + r.key_len as usize]
    }
    fn line(&self, r: &Row) -> &[u8] {
        let at = r.at + r.key_len as usize;
        &self.arena[at..at + r.line_len as usize]
    }
    fn find(&self, h: u64, key: &[u8]) -> Option<usize> {
        let mut i = *self.index.get(&h)?;
        while i != NONE {
            let r = &self.rows[i as usize];
            if self.key(r) == key {
                return Some(i as usize);
            }
            i = r.next;
        }
        None
    }
    fn insert(&mut self, key: &[u8], line: &[u8], cr: bool) {
        self.insert_hashed(hash(key), key, line, cr);
    }
    fn insert_hashed(&mut self, h: u64, key: &[u8], line: &[u8], cr: bool) {
        let at = self.arena.len();
        self.arena.extend_from_slice(key);
        self.arena.extend_from_slice(line);
        let len = |b: &[u8]| u32::try_from(b.len()).expect("diff: line over 4 GiB");
        let (key_len, line_len) = (len(key), len(line));
        if let Some(i) = self.find(h, key) {
            // The replaced bytes stay in the arena unused; duplicate keys
            // are rare enough not to compact.
            let r = &mut self.rows[i];
            (r.at, r.key_len, r.line_len, r.cr) = (at, key_len, line_len, cr);
            return;
        }
        // ponytail: u32 row ids cap OLD at 4.29 billion rows.
        let id = u32::try_from(self.rows.len())
            .ok()
            .filter(|&i| i != NONE)
            .expect("diff: more than 4 billion OLD rows");
        let next = self.index.insert(h, id).unwrap_or(NONE);
        self.rows.push(Row {
            at,
            key_len,
            line_len,
            next,
            cr,
            seen: false,
        });
    }
}

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
        oldrow.insert(&k, l, cr);
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
    let mut oldrow = OldRows::default();
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
            match oldrow.find(hash(&k), &k) {
                None => ops.push([b"A\t", &k[..], b"\t", line].concat()),
                Some(i) => {
                    oldrow.rows[i].seen = true;
                    let o = &oldrow.rows[i];
                    if differs(oldrow.line(o), o.cr, line, cr) {
                        // changed_cols: compares by position against NEW's
                        // header names, even when the two headers differ (L2).
                        fo.parse(oldrow.line(o), &sep);
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
    for r in oldrow.rows.iter().filter(|r| !r.seen) {
        ops.push([b"R\t", oldrow.key(r), b"\t", oldrow.line(r)].concat());
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

#[cfg(test)]
mod tests {
    use super::OldRows;

    #[test]
    fn old_rows_survive_hash_collisions_and_replacement() {
        let mut m = OldRows::default();
        // Every key on one hash, so lookups have to walk the chain.
        for (k, l) in [("a", "1"), ("b", "2"), ("", "3"), ("a", "4"), ("ab", "5")] {
            m.insert_hashed(7, k.as_bytes(), l.as_bytes(), false);
        }
        let got = |k: &str| m.find(7, k.as_bytes()).map(|i| m.line(&m.rows[i]).to_vec());
        assert_eq!(got("a"), Some(b"4".to_vec()), "later row replaces earlier");
        assert_eq!(got("b"), Some(b"2".to_vec()));
        assert_eq!(got(""), Some(b"3".to_vec()));
        assert_eq!(got("ab"), Some(b"5".to_vec()));
        assert_eq!(got("abc"), None);
        assert_eq!(m.find(8, b"a"), None);
        assert_eq!(m.rows.len(), 4, "a replaced key is not a new row");
    }
}
