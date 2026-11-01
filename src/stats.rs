//! `csvq stats`: per-column statistics (legacy stats.awk).

use crate::awk::fmt_f;
use crate::csv::{csv_type, scan_records, Fields, Ty};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};

#[derive(Default)]
struct Col {
    name: Vec<u8>,
    nulls: u64,
    nn: u64,
    vint: u64,
    vfloat: u64,
    vbool: u64,
    vdate: u64,
    // ponytail: exact distinct counts keep every distinct value, as the awk
    // `seen` array does; memory grows with column cardinality.
    freq: HashMap<Box<[u8]>, u64>,
    sum: f64,
    ncnt: u64,
    min: Option<(f64, Vec<u8>)>,
    max: Option<(f64, Vec<u8>)>,
}

impl Col {
    fn coltype(&self) -> Ty {
        if self.nn == 0 {
            return Ty::Str; // printed as "empty" by the caller
        }
        if self.vint == self.nn {
            Ty::Int
        } else if self.vfloat == self.nn {
            Ty::Float
        } else if self.vbool == self.nn {
            Ty::Bool
        } else if self.vdate == self.nn {
            Ty::Date
        } else {
            Ty::Str
        }
    }
}

pub fn run(
    input: impl BufRead,
    sep_arg: &[u8],
    fname: &[u8],
    out: &mut impl Write,
) -> io::Result<()> {
    let mut sep = sep_arg.to_vec();
    let mut f = Fields::default();
    let mut cols: Vec<Col> = Vec::new();
    let mut total: u64 = 0;
    let mut datarows: u64 = 0;
    scan_records(input, &mut sep, |rec, sep| {
        f.parse(rec, sep);
        total += 1;
        if total == 1 {
            cols = (1..=f.len())
                .map(|c| Col {
                    name: f.get(c).to_vec(),
                    ..Col::default()
                })
                .collect();
            return;
        }
        datarows += 1;
        for (i, col) in cols.iter_mut().enumerate() {
            let v = f.get_or_empty(i + 1);
            if v.is_empty() {
                col.nulls += 1;
                continue;
            }
            col.nn += 1;
            let t = csv_type(v);
            match t {
                Ty::Int => {
                    col.vint += 1;
                    col.vfloat += 1;
                }
                Ty::Float => col.vfloat += 1,
                Ty::Bool => col.vbool += 1,
                Ty::Date => col.vdate += 1,
                Ty::Str => {}
            }
            match col.freq.get_mut(v) {
                Some(n) => *n += 1,
                None => {
                    col.freq.insert(v.into(), 1);
                }
            }
            if matches!(t, Ty::Int | Ty::Float) {
                // csv_type guarantees plain decimal syntax, which Rust parses
                // with the same correct rounding as strtod.
                let num: f64 = std::str::from_utf8(v)
                    .unwrap()
                    .trim_end_matches('.')
                    .parse()
                    .unwrap();
                col.sum += num;
                col.ncnt += 1;
                if col.min.as_ref().is_none_or(|(m, _)| num < *m) {
                    col.min = Some((num, v.to_vec()));
                }
                if col.max.as_ref().is_none_or(|(m, _)| num > *m) {
                    col.max = Some((num, v.to_vec()));
                }
            }
        }
    })?;

    out.write_all(b"CSV COLUMN STATISTICS\n=====================\nFile: ")?;
    out.write_all(fname)?;
    writeln!(out, "\nRows: {datarows} (excluding header)")?;
    for (i, col) in cols.iter().enumerate() {
        let t = col.coltype();
        let tname = if col.nn == 0 { "empty" } else { t.name() };
        write!(out, "\nColumn {}: ", i + 1)?;
        out.write_all(&col.name)?;
        let rate = if datarows > 0 {
            col.nulls as f64 * 100.0 / datarows as f64
        } else {
            0.0
        };
        write!(
            out,
            "\n  type:      {tname}\n  non-null:  {}\n  nulls:     {} ({}%)\n  distinct:  {}\n",
            col.nn,
            col.nulls,
            fmt_f(rate, 1),
            col.freq.len()
        )?;
        if col.nn > 0 && matches!(t, Ty::Int | Ty::Float) && col.ncnt > 0 {
            out.write_all(b"  min:       ")?;
            out.write_all(&col.min.as_ref().unwrap().1)?;
            out.write_all(b"\n  max:       ")?;
            out.write_all(&col.max.as_ref().unwrap().1)?;
            writeln!(
                out,
                "\n  mean:      {}",
                fmt_f(col.sum / col.ncnt as f64, 4)
            )?;
        }
        let distinct = col.freq.len();
        if distinct > 0 && distinct <= 200 {
            let mut top: Vec<(&[u8], u64)> = col.freq.iter().map(|(k, &n)| (&k[..], n)).collect();
            top.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            out.write_all(b"  top values: ")?;
            for (j, (v, n)) in top.iter().take(3).enumerate() {
                if j > 0 {
                    out.write_all(b", ")?;
                }
                out.write_all(v)?;
                write!(out, " ({n})")?;
            }
            out.write_all(b"\n")?;
        } else if distinct > 200 {
            out.write_all(b"  top values: (high cardinality; omitted)\n")?;
        }
    }
    Ok(())
}
