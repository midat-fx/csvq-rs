//! The CSV model of legacy lib/csvlib.awk, reproduced byte for byte.
//!
//! The legacy reader is not RFC 4180: it joins physical lines while the
//! *total* number of double quotes seen so far is odd, wherever they sit, and
//! the field parser lets a quote open a quoted section mid-field. Both quirks
//! are part of the observable behaviour, so they are kept.

use std::io::{self, BufRead};

/// Guess the delimiter from the first physical line: the most frequent of
/// comma, tab, semicolon, pipe; earlier candidates win ties.
pub fn detect_delim(line: &[u8]) -> u8 {
    let mut best = b',';
    let mut bestc = -1i64;
    for c in *b",\t;|" {
        let n = line.iter().filter(|&&b| b == c).count() as i64;
        if n > bestc {
            bestc = n;
            best = c;
        }
    }
    best
}

pub fn delim_name(sep: &[u8]) -> &[u8] {
    match sep {
        b"," => b"comma",
        b"\t" => b"tab",
        b";" => b"semicolon",
        b"|" => b"pipe",
        other => other,
    }
}

/// The fields of one logical record, stored back to back to avoid an
/// allocation per field.
#[derive(Default)]
pub struct Fields {
    buf: Vec<u8>,
    ends: Vec<usize>,
}

impl Fields {
    /// `csv_parse`: a field separator only counts outside quotes, `""` inside
    /// quotes is one quote, and a quote anywhere outside quotes opens a
    /// quoted section. A multi-byte separator (`-d ab`) never matches, since
    /// awk compares it with one character at a time.
    pub fn parse(&mut self, rec: &[u8], sep: &[u8]) {
        self.buf.clear();
        self.ends.clear();
        let sep = if sep.len() == 1 { Some(sep[0]) } else { None };
        let mut inq = false;
        let mut i = 0;
        while i < rec.len() {
            if inq {
                let run = rec[i..]
                    .iter()
                    .position(|&b| b == b'"')
                    .unwrap_or(rec.len() - i);
                self.buf.extend_from_slice(&rec[i..i + run]);
                i += run;
                if i < rec.len() {
                    if rec.get(i + 1) == Some(&b'"') {
                        self.buf.push(b'"');
                        i += 1;
                    } else {
                        inq = false;
                    }
                    i += 1;
                }
            } else {
                let run = rec[i..]
                    .iter()
                    .position(|&b| b == b'"' || Some(b) == sep)
                    .unwrap_or(rec.len() - i);
                self.buf.extend_from_slice(&rec[i..i + run]);
                i += run;
                if i < rec.len() {
                    if rec[i] == b'"' {
                        inq = true;
                    } else {
                        self.ends.push(self.buf.len());
                    }
                    i += 1;
                }
            }
        }
        self.ends.push(self.buf.len());
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Field `i`, 1-based like awk.
    pub fn get(&self, i: usize) -> &[u8] {
        let start = if i == 1 { 0 } else { self.ends[i - 2] };
        &self.buf[start..self.ends[i - 1]]
    }

    /// Field `i` (1-based), or "" past the end, the way an unset awk array
    /// element reads.
    pub fn get_or_empty(&self, i: usize) -> &[u8] {
        if i >= 1 && i <= self.len() {
            self.get(i)
        } else {
            b""
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    Int,
    Float,
    Bool,
    Date,
    Str,
}

impl Ty {
    pub fn name(self) -> &'static str {
        match self {
            Ty::Int => "int",
            Ty::Float => "float",
            Ty::Bool => "bool",
            Ty::Date => "date",
            Ty::Str => "string",
        }
    }
}

fn all_digits(s: &[u8]) -> bool {
    s.iter().all(u8::is_ascii_digit)
}

/// `csv_type`: int, float, bool, date (YYYY-MM-DD) or string.
pub fn csv_type(v: &[u8]) -> Ty {
    let s = v.strip_prefix(b"-").unwrap_or(v);
    if !s.is_empty() && all_digits(s) {
        return Ty::Int;
    }
    if let Some(p) = s.iter().position(|&b| b == b'.') {
        let (a, b) = (&s[..p], &s[p + 1..]);
        if all_digits(a) && all_digits(b) && !(a.is_empty() && b.is_empty()) {
            return Ty::Float;
        }
    }
    if let Some(p) = s.iter().position(|&b| b == b'e' || b == b'E') {
        let (a, b) = (&s[..p], &s[p + 1..]);
        let b = b
            .strip_prefix(b"-")
            .or_else(|| b.strip_prefix(b"+"))
            .unwrap_or(b);
        if !a.is_empty() && all_digits(a) && !b.is_empty() && all_digits(b) {
            return Ty::Float;
        }
    }
    if matches!(v, b"true" | b"false" | b"TRUE" | b"FALSE") {
        return Ty::Bool;
    }
    if v.len() == 10
        && v.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return Ty::Date;
    }
    Ty::Str
}

/// Physical lines as awk reads them with RS="\n": no terminator, a final
/// unterminated line still counts, one trailing CR is stripped (`sub(/\r$/)`).
pub struct Lines<R> {
    r: R,
    buf: Vec<u8>,
}

impl<R: BufRead> Lines<R> {
    pub fn new(r: R) -> Self {
        Lines { r, buf: Vec::new() }
    }

    /// The next line and whether a trailing CR was removed from it.
    pub fn next_line(&mut self) -> io::Result<Option<(&[u8], bool)>> {
        self.buf.clear();
        if self.r.read_until(b'\n', &mut self.buf)? == 0 {
            return Ok(None);
        }
        let mut line = &self.buf[..];
        if line.last() == Some(&b'\n') {
            line = &line[..line.len() - 1];
        }
        let cr = line.last() == Some(&b'\r');
        if cr {
            line = &line[..line.len() - 1];
        }
        Ok(Some((line, cr)))
    }
}

/// Walk the logical records of a file exactly like every legacy command's
/// main rule: detect the delimiter on the first physical line (unless one was
/// forced), then join lines with "\n" while the running quote count is odd.
/// Returns the number of physical lines that ended in CRLF.
pub fn scan_records<R: BufRead>(
    r: R,
    sep: &mut Vec<u8>,
    mut on_record: impl FnMut(&[u8], &[u8]),
) -> io::Result<u64> {
    let mut lines = Lines::new(r);
    let mut rec: Vec<u8> = Vec::new();
    let mut inrec = false;
    let mut quotes = 0usize;
    let mut first = true;
    let mut crlf = 0;
    while let Some((line, cr)) = lines.next_line()? {
        crlf += u64::from(cr);
        if first {
            first = false;
            if sep.is_empty() {
                *sep = vec![detect_delim(line)];
            }
        }
        let q = line.iter().filter(|&&b| b == b'"').count();
        if !inrec {
            if q % 2 == 0 {
                on_record(line, sep);
                continue;
            }
            rec.clear();
            rec.extend_from_slice(line);
            quotes = q;
            inrec = true;
            continue;
        }
        rec.push(b'\n');
        rec.extend_from_slice(line);
        quotes += q;
        if quotes % 2 == 0 {
            on_record(&rec, sep);
            inrec = false;
        }
    }
    if inrec {
        on_record(&rec, sep);
    }
    Ok(crlf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(rec: &str, sep: &str) -> Vec<String> {
        let mut f = Fields::default();
        f.parse(rec.as_bytes(), sep.as_bytes());
        (1..=f.len())
            .map(|i| String::from_utf8(f.get(i).to_vec()).unwrap())
            .collect()
    }

    #[test]
    fn parse_like_csv_parse() {
        assert_eq!(fields(r#"a,"b,c",d"#, ","), ["a", "b,c", "d"]);
        assert_eq!(fields(r#""x""y""#, ","), [r#"x"y"#]);
        assert_eq!(fields(r#"ab"c,d"e"#, ","), ["abc,de"]);
        assert_eq!(fields("", ","), [""]);
        assert_eq!(fields("a,b", "ab"), ["a,b"]);
        assert_eq!(fields(r#""open,end"#, ","), ["open,end"]);
    }

    #[test]
    fn types() {
        assert_eq!(csv_type(b"-12"), Ty::Int);
        assert_eq!(csv_type(b"."), Ty::Str);
        assert_eq!(csv_type(b"-.5"), Ty::Float);
        assert_eq!(csv_type(b"5."), Ty::Float);
        assert_eq!(csv_type(b"1e-3"), Ty::Float);
        assert_eq!(csv_type(b"1.5e3"), Ty::Str);
        assert_eq!(csv_type(b"True"), Ty::Str);
        assert_eq!(csv_type(b"2024-13-99"), Ty::Date);
        assert_eq!(csv_type(b"-"), Ty::Str);
    }

    #[test]
    fn records_join_on_odd_quotes() {
        let mut sep = Vec::new();
        let mut recs = Vec::new();
        let crlf = scan_records(&b"a;b\r\n\"x\r\ny\";z\nlast"[..], &mut sep, |r, _| {
            recs.push(r.to_vec())
        })
        .unwrap();
        assert_eq!(sep, b";");
        assert_eq!(crlf, 2);
        assert_eq!(
            recs,
            [b"a;b".to_vec(), b"\"x\ny\";z".to_vec(), b"last".to_vec()]
        );
    }
}
