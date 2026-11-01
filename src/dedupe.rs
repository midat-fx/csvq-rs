//! `csvq dedupe`: drop duplicate rows, keep the first (legacy dedupe.awk).

use crate::awk::split_commas;
use crate::csv::{scan_records, Fields};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};

/// Writes the kept records to `out`; returns how many were removed.
pub fn run(
    input: impl BufRead,
    sep_arg: &[u8],
    key: &[u8],
    out: &mut impl Write,
) -> io::Result<u64> {
    let keys = split_commas(key);
    let mut sep = sep_arg.to_vec();
    let mut f = Fields::default();
    let mut pos: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut seen: HashSet<Box<[u8]>> = HashSet::new();
    let mut total: u64 = 0;
    let mut removed: u64 = 0;
    let mut k: Vec<u8> = Vec::new();
    let mut err: Option<io::Error> = None;
    scan_records(input, &mut sep, |rec, sep| {
        if err.is_some() {
            return;
        }
        total += 1;
        let keep = if total == 1 {
            f.parse(rec, sep);
            for i in 1..=f.len() {
                pos.insert(f.get(i).to_vec(), i);
            }
            true
        } else {
            let key: &[u8] = if keys.is_empty() {
                rec
            } else {
                // Key parts joined with SUBSEP (\034), like the awk subscript.
                f.parse(rec, sep);
                k.clear();
                for (i, name) in keys.iter().enumerate() {
                    if i > 0 {
                        k.push(0x1c);
                    }
                    if let Some(&p) = pos.get(name) {
                        k.extend_from_slice(f.get_or_empty(p));
                    }
                }
                &k
            };
            if seen.contains(key) {
                removed += 1;
                false
            } else {
                seen.insert(key.into());
                true
            }
        };
        if keep {
            if let Err(e) = out.write_all(rec).and_then(|()| out.write_all(b"\n")) {
                err = Some(e);
            }
        }
    })?;
    match err {
        Some(e) => Err(e),
        None => Ok(removed),
    }
}
