//! `csvq validate`: schema DSL loading and row checks (legacy validate.awk).

use crate::awk::{atof, num_to_str, translate_ere};
use crate::csv::{csv_type, scan_records, Fields, Ty};
use regex::bytes::Regex;
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};

#[derive(Default, Clone)]
struct Rule {
    ty: Vec<u8>,
    required: bool,
    unique: bool,
    min: Option<f64>,
    max: Option<f64>,
    regex: Option<Regex>,
    regex_src: Vec<u8>,
}

pub struct Schema {
    /// Rule order as written; a column named twice appears twice (L7).
    names: Vec<Vec<u8>>,
    rules: HashMap<Vec<u8>, Rule>,
}

/// Parse the schema the way `load_schema` does, quirks included: the
/// per-line "pending" settings are only reset after a line that names a
/// column, so `required` or `type:int` on a line without `col:` leak into the
/// next rule (L6); `min:`/`max:` stick to a column across redefinitions.
pub fn load_schema(text: &[u8]) -> Result<Schema, String> {
    let mut s = Schema {
        names: Vec::new(),
        rules: HashMap::new(),
    };
    let mut req = false;
    let mut uniq = false;
    let mut ty: Vec<u8> = Vec::new();
    let mut min: Option<Vec<u8>> = None;
    let mut max: Option<Vec<u8>> = None;
    let mut re: Vec<u8> = Vec::new();
    let body = text.strip_suffix(b"\n").unwrap_or(text);
    if text.is_empty() {
        return Ok(s);
    }
    for line in body.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let lead = line
            .iter()
            .take_while(|&&b| b == b' ' || b == b'\t')
            .count();
        if lead == line.len() || line[lead] == b'#' {
            continue;
        }
        let mut name: Vec<u8> = Vec::new();
        for tok in line
            .split(|&b| b == b' ' || b == b'\t')
            .filter(|t| !t.is_empty())
        {
            if tok == b"required" {
                req = true;
                continue;
            }
            if tok == b"unique" {
                uniq = true;
                continue;
            }
            let (k, v) = match tok.iter().position(|&b| b == b':') {
                Some(p) => (&tok[..p], &tok[p + 1..]),
                None => (tok, &b""[..]),
            };
            match k {
                b"col" => name = v.to_vec(),
                b"type" => ty = v.to_vec(),
                b"min" => min = Some(v.to_vec()),
                b"max" => max = Some(v.to_vec()),
                b"regex" => re = v.to_vec(),
                _ => {}
            }
        }
        if name.is_empty() {
            continue;
        }
        s.names.push(name.clone());
        let r = s.rules.entry(name).or_default();
        r.ty = std::mem::take(&mut ty);
        r.required = std::mem::take(&mut req);
        r.unique = std::mem::take(&mut uniq);
        // `pend_min + 0`: the addition also turns -0 into +0.
        if let Some(m) = min.take() {
            r.min = Some(atof(&m) + 0.0);
        }
        if let Some(m) = max.take() {
            r.max = Some(atof(&m) + 0.0);
        }
        r.regex_src = std::mem::take(&mut re);
        r.regex = if r.regex_src.is_empty() {
            None
        } else {
            let pat = translate_ere(&r.regex_src)?;
            Some(Regex::new(&pat).map_err(|e| e.to_string())?)
        };
    }
    Ok(s)
}

fn type_ok(v: &[u8], want: &[u8]) -> bool {
    if want.is_empty() || want == b"string" {
        return true;
    }
    let t = csv_type(v);
    if want == b"float" {
        return matches!(t, Ty::Int | Ty::Float);
    }
    t.name().as_bytes() == want
}

/// Returns the number of violations; the report goes to `out`.
pub fn run(
    input: impl BufRead,
    schema: &Schema,
    sep_arg: &[u8],
    fname: &[u8],
    schema_name: &[u8],
    out: &mut impl Write,
) -> io::Result<u64> {
    let mut sep = sep_arg.to_vec();
    let mut f = Fields::default();
    let mut total: u64 = 0;
    let mut drow: u64 = 0;
    let mut pos: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut warned: HashSet<Vec<u8>> = HashSet::new();
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut shown: Vec<Vec<u8>> = Vec::new();
    let mut nviol: u64 = 0;

    scan_records(input, &mut sep, |rec, sep| {
        f.parse(rec, sep);
        total += 1;
        if total == 1 {
            for i in 1..=f.len() {
                pos.insert(f.get(i).to_vec(), i);
            }
            return;
        }
        drow += 1;
        // add_viol: count every violation, show only the first 200.
        fn viol(nviol: &mut u64, shown: &mut Vec<Vec<u8>>, msg: Vec<u8>) {
            *nviol += 1;
            if *nviol <= 200 {
                shown.push(msg);
            }
        }
        let cat = |parts: &[&[u8]]| -> Vec<u8> { parts.concat() };
        for name in &schema.names {
            let r = &schema.rules[name];
            let Some(&p) = pos.get(name) else {
                if warned.insert(name.clone()) {
                    shown.push(cat(&[
                        b"  schema warning: column '",
                        name,
                        b"' not found in header",
                    ]));
                }
                continue;
            };
            let v = f.get_or_empty(p);
            let row = drow.to_string();
            let row = row.as_bytes();
            if v.is_empty() {
                if r.required {
                    viol(
                        &mut nviol,
                        &mut shown,
                        cat(&[
                            b"  row ",
                            row,
                            b": column '",
                            name,
                            b"' is required but empty",
                        ]),
                    );
                }
                continue;
            }
            if !type_ok(v, &r.ty) {
                viol(
                    &mut nviol,
                    &mut shown,
                    cat(&[
                        b"  row ",
                        row,
                        b": column '",
                        name,
                        b"' value '",
                        v,
                        b"' is not type ",
                        &r.ty,
                    ]),
                );
            }
            // BWK compares numbers by the sign of x - y, so NaN never fails.
            if let Some(m) = r.min {
                if atof(v) - m < 0.0 {
                    let ms = num_to_str(m);
                    viol(
                        &mut nviol,
                        &mut shown,
                        cat(&[
                            b"  row ",
                            row,
                            b": column '",
                            name,
                            b"' value ",
                            v,
                            b" < min ",
                            ms.as_bytes(),
                        ]),
                    );
                }
            }
            if let Some(m) = r.max {
                if atof(v) - m > 0.0 {
                    let ms = num_to_str(m);
                    viol(
                        &mut nviol,
                        &mut shown,
                        cat(&[
                            b"  row ",
                            row,
                            b": column '",
                            name,
                            b"' value ",
                            v,
                            b" > max ",
                            ms.as_bytes(),
                        ]),
                    );
                }
            }
            if let Some(re) = &r.regex {
                if !re.is_match(v) {
                    viol(
                        &mut nviol,
                        &mut shown,
                        cat(&[
                            b"  row ",
                            row,
                            b": column '",
                            name,
                            b"' value '",
                            v,
                            b"' fails regex ",
                            &r.regex_src,
                        ]),
                    );
                }
            }
            if r.unique {
                // awk key `name SUBSEP v`; SUBSEP is \034.
                let key = cat(&[name, b"\x1c", v]);
                if !seen.insert(key) {
                    viol(
                        &mut nviol,
                        &mut shown,
                        cat(&[
                            b"  row ",
                            row,
                            b": column '",
                            name,
                            b"' value '",
                            v,
                            b"' is not unique",
                        ]),
                    );
                }
            }
        }
    })?;

    out.write_all(b"CSV SCHEMA VALIDATION\n=====================\nFile:   ")?;
    out.write_all(fname)?;
    out.write_all(b"\nSchema: ")?;
    out.write_all(schema_name)?;
    writeln!(
        out,
        "\nRules:  {}   Data rows: {}\n",
        schema.names.len(),
        drow
    )?;
    for line in &shown {
        out.write_all(line)?;
        out.write_all(b"\n")?;
    }
    if !shown.is_empty() {
        out.write_all(b"\n")?;
    }
    if nviol == 0 {
        out.write_all(b"OK: no violations\n")?;
    } else {
        writeln!(out, "FAIL: {nviol} violation(s)")?;
    }
    Ok(nviol)
}
