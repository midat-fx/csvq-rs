//! The bits of awk semantics the legacy output depends on, reproduced from
//! the reference implementation (BWK "one true awk" 20200816, the macOS
//! /usr/bin/awk). See docs/DESIGN.md for why this awk is the oracle.

/// Escape processing that awk applies to `-v name=value` assignments
/// (`qstring` in BWK lib.c). The legacy wrapper passes the delimiter, file
/// names, key lists and the schema path through `-v`, so `-d '\t'` means TAB.
pub fn qstring(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        i += 1;
        if c != b'\\' {
            out.push(c);
            continue;
        }
        let Some(&c) = s.get(i) else {
            out.push(b'\\');
            break;
        };
        i += 1;
        match c {
            b'\\' => out.push(b'\\'),
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'r' => out.push(b'\r'),
            b'v' => out.push(11),
            b'a' => out.push(7),
            b'0'..=b'9' => {
                // isdigit(), not isoctdigit(): BWK happily folds 8 and 9 in.
                let mut n = u32::from(c - b'0');
                for _ in 0..2 {
                    match s.get(i) {
                        Some(&d) if d.is_ascii_digit() => {
                            n = 8 * n + u32::from(d - b'0');
                            i += 1;
                        }
                        _ => break,
                    }
                }
                out.push(n as u8);
            }
            // In this awk version an unknown escape just drops the backslash.
            _ => out.push(c),
        }
    }
    // The result is a C string: it ends at the first NUL (e.g. from `\0`).
    if let Some(p) = out.iter().position(|&b| b == 0) {
        out.truncate(p);
    }
    out
}

/// awk's string-to-number conversion: BWK calls atof(), i.e. the libc
/// strtod() on the longest valid prefix. macOS strtod accepts leading
/// whitespace, hex floats, `inf`/`infinity` and `nan`, so `"0x1A" + 0` is 26
/// here (gawk and mawk say 0 — one of the cross-awk divergences).
pub fn atof(s: &[u8]) -> f64 {
    strtod(s).0
}

/// BWK `is_number`: does a string read from input count as numeric (a
/// "strnum")? strtod must consume something, only blanks may follow, and the
/// result may be neither +HUGE_VAL nor out of range (ERANGE).
pub fn is_number(s: &[u8]) -> Option<f64> {
    let (v, n, erange) = strtod(s);
    let ok = n > 0
        && v != f64::INFINITY
        && !erange
        && s[n..]
            .iter()
            .all(|&b| b == b' ' || b == b'\t' || b == b'\n');
    ok.then_some(v)
}

/// macOS strtod: (value, bytes consumed or 0 if no conversion, ERANGE set).
pub fn strtod(s: &[u8]) -> (f64, usize, bool) {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let rest = &s[i..];
    let starts_ci = |p: &[u8]| rest.len() >= p.len() && rest[..p.len()].eq_ignore_ascii_case(p);
    let (mag, len, erange) = if starts_ci(b"inf") {
        (
            f64::INFINITY,
            if starts_ci(b"infinity") { 8 } else { 3 },
            false,
        )
    } else if starts_ci(b"nan") {
        // optional "(n-char-sequence)"
        let tail = rest[3..].strip_prefix(b"(").map(|t| {
            let k = t
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                .count();
            if t.get(k) == Some(&b')') {
                k + 2
            } else {
                0
            }
        });
        (f64::NAN, 3 + tail.unwrap_or(0), false)
    } else if rest.len() > 2
        && rest[0] == b'0'
        && (rest[1] | 0x20) == b'x'
        && (rest[2].is_ascii_hexdigit()
            || (rest[2] == b'.' && rest.get(3).is_some_and(u8::is_ascii_hexdigit)))
    {
        let (v, n) = hex_float(&rest[2..]);
        (v, 2 + n, v.is_infinite())
    } else {
        match decimal_prefix(rest) {
            Some(x) => x,
            // No conversion at all: strtod returns +0 even after a '-'.
            None => return (0.0, 0, false),
        }
    };
    (if neg { -mag } else { mag }, i + len, erange)
}

fn decimal_prefix(s: &[u8]) -> Option<(f64, usize, bool)> {
    let digits = |from: usize| s[from..].iter().take_while(|b| b.is_ascii_digit()).count();
    let int = digits(0);
    let mut end = int;
    let mut frac = 0;
    if s.get(end) == Some(&b'.') {
        frac = digits(end + 1);
        end += 1 + frac;
    }
    if int + frac == 0 {
        return None;
    }
    let mant_end = end;
    if matches!(s.get(end), Some(b'e' | b'E')) {
        let mut j = end + 1;
        if matches!(s.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp = digits(j);
        if exp > 0 {
            end = j + exp;
        }
    }
    // Rust's parser rejects a bare trailing '.', strtod does not.
    let text = std::str::from_utf8(&s[..end]).unwrap();
    let v: f64 = text.trim_end_matches('.').parse().ok()?;
    // ERANGE: overflow, or underflow to zero/subnormal from nonzero digits.
    let nonzero = s[..mant_end].iter().any(|&b| (b'1'..=b'9').contains(&b));
    let erange = v.is_infinite() || (nonzero && v.abs() < f64::MIN_POSITIVE);
    Some((v, end, erange))
}

// ponytail: keeps the first 15 significant hex digits and truncates the rest,
// so hex strings longer than that may round differently from strtod. Hex
// numbers only reach csvq through schema min/max and validated values.
fn hex_float(s: &[u8]) -> (f64, usize) {
    let mut mant: u64 = 0;
    let mut sig = 0;
    let mut exp2: i64 = 0;
    let mut i = 0;
    let mut seen_dot = false;
    while i < s.len() {
        let c = s[i];
        if c == b'.' && !seen_dot {
            seen_dot = true;
        } else if let Some(d) = (c as char).to_digit(16) {
            if sig < 15 {
                if mant != 0 || d != 0 {
                    sig += 1;
                }
                mant = mant * 16 + u64::from(d);
                if seen_dot {
                    exp2 -= 4;
                }
            } else if !seen_dot {
                exp2 += 4;
            }
        } else {
            break;
        }
        i += 1;
    }
    if matches!(s.get(i), Some(b'p' | b'P')) {
        let mut j = i + 1;
        let mut eneg = false;
        if matches!(s.get(j), Some(b'+' | b'-')) {
            eneg = s[j] == b'-';
            j += 1;
        }
        let start = j;
        let mut e: i64 = 0;
        while j < s.len() && s[j].is_ascii_digit() {
            e = (e * 10 + i64::from(s[j] - b'0')).min(100_000);
            j += 1;
        }
        if j > start {
            exp2 += if eneg { -e } else { e };
            i = j;
        }
    }
    (mant as f64 * 2f64.powi(exp2.clamp(-2000, 2000) as i32), i)
}

/// C printf `%.<prec>g`.
pub fn fmt_g(x: f64, prec: usize) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf" } else { "inf" }.into();
    }
    let p = prec.max(1);
    let e = format!("{:.*e}", p - 1, x);
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    if exp < -4 || exp >= p as i32 {
        format!(
            "{}e{}{:02}",
            trim_frac(mant),
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        )
    } else {
        trim_frac(&format!("{:.*}", (p as i32 - 1 - exp) as usize, x)).to_string()
    }
}

fn trim_frac(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// C printf `%.<prec>f` (macOS prints NaN as "nan" whatever its sign).
pub fn fmt_f(x: f64, prec: usize) -> String {
    if x.is_nan() {
        "nan".into()
    } else if x.is_infinite() {
        if x < 0.0 { "-inf" } else { "inf" }.into()
    } else {
        format!("{x:.prec$}")
    }
}

/// Number-to-string conversion of BWK 20200816: integral values (modf == 0,
/// which includes infinities) use "%.30g", everything else CONVFMT "%.6g".
pub fn num_to_str(x: f64) -> String {
    if x.is_infinite() || x.fract() == 0.0 {
        fmt_g(x, 30)
    } else {
        fmt_g(x, 6)
    }
}

/// awk `split(s, a, ",")` with a single-character separator.
pub fn split_commas(s: &[u8]) -> Vec<Vec<u8>> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split(|&b| b == b',').map(<[u8]>::to_vec).collect()
}

/// Translate an awk dynamic regex (BWK dialect) into a `regex::bytes`
/// pattern. Every escape and every bracket expression is resolved to explicit
/// bytes so the two engines cannot disagree on what a character means.
/// Operators `^ $ . ( ) | * + ? {n,m}` pass through: they mean the same thing
/// in both engines for well-formed patterns.
pub fn translate_ere(re: &[u8]) -> Result<String, String> {
    let mut out = String::from("(?s-u)");
    let mut i = 0;
    while i < re.len() {
        let c = re[i];
        i += 1;
        match c {
            b'\\' => {
                if i >= re.len() {
                    out.push_str(r"\x5C");
                } else {
                    let b = quoted(re, &mut i);
                    push_byte(&mut out, b);
                }
            }
            b'[' => {
                let set = bracket(re, &mut i)?;
                if !set.iter().any(|&x| x) {
                    // BWK quirk: a class left empty (e.g. `[z-a]`) matches
                    // the empty string rather than nothing.
                    out.push_str("(?:)");
                    continue;
                }
                out.push('[');
                for b in 0..=255u8 {
                    if set[b as usize] {
                        push_byte(&mut out, b);
                    }
                }
                out.push(']');
            }
            b'^' | b'$' | b'.' | b'(' | b')' | b'|' | b'*' | b'+' | b'?' | b'{' | b'}' => {
                out.push(c as char)
            }
            _ if c.is_ascii_alphanumeric() || c == b',' => out.push(c as char),
            _ => push_byte(&mut out, c),
        }
    }
    Ok(out)
}

fn push_byte(out: &mut String, b: u8) {
    out.push_str(&format!("\\x{b:02X}"));
}

/// `quoted()` from BWK b.c: what a backslash sequence inside a regex means.
fn quoted(re: &[u8], i: &mut usize) -> u8 {
    let c = re[*i];
    *i += 1;
    match c {
        b't' => b'\t',
        b'n' => b'\n',
        b'f' => 12,
        b'r' => b'\r',
        b'b' => 8,
        b'v' => 11,
        b'a' => 7,
        b'\\' => b'\\',
        b'x' => {
            let mut n: u32 = 0;
            let mut k = 0;
            while k < 2 && *i < re.len() && re[*i].is_ascii_hexdigit() {
                n = n * 16 + (re[*i] as char).to_digit(16).unwrap();
                *i += 1;
                k += 1;
            }
            n as u8
        }
        b'0'..=b'7' => {
            let mut n = u32::from(c - b'0');
            for _ in 0..2 {
                match re.get(*i) {
                    Some(&d @ b'0'..=b'7') => {
                        n = 8 * n + u32::from(d - b'0');
                        *i += 1;
                    }
                    _ => break,
                }
            }
            n as u8
        }
        other => other,
    }
}

/// A bracket expression, following BWK relex() + cclenter(): `]` right after
/// `[` or `[^` is literal, POSIX classes expand to their C-locale members,
/// ranges run over raw byte values and a reversed range drops its start.
fn bracket(re: &[u8], i: &mut usize) -> Result<[bool; 256], String> {
    let negate = re.get(*i) == Some(&b'^');
    if negate {
        *i += 1;
    }
    // Pass 1 (relex): collect the class body, expanding [:name:].
    let mut body: Vec<(u8, bool)> = Vec::new(); // (byte, was_escaped)
    loop {
        let Some(&c) = re.get(*i) else {
            return Err("nonterminated character class".into());
        };
        *i += 1;
        if c == b'\\' {
            if *i >= re.len() {
                return Err("nonterminated character class".into());
            }
            let b = quoted(re, i);
            body.push((b, true));
        } else if c == b'[' && re.get(*i) == Some(&b':') {
            if let Some((members, len)) = posix_class(&re[*i + 1..]) {
                *i += len;
                body.extend(members.into_iter().map(|b| (b, false)));
            } else {
                body.push((c, false));
            }
        } else if body.is_empty() {
            body.push((c, false));
        } else if c == b']' {
            break;
        } else {
            body.push((c, false));
        }
    }
    // Pass 2 (cclenter): ranges.
    let mut set: Vec<u8> = Vec::new();
    let mut k = 0;
    while k < body.len() {
        let (c, esc) = body[k];
        k += 1;
        if c == b'-' && !esc && !set.is_empty() && *set.last().unwrap() != 0 && k < body.len() {
            let lo = *set.last().unwrap();
            let hi = body[k].0;
            k += 1;
            if lo > hi {
                set.pop();
                continue;
            }
            set.extend(lo + 1..=hi);
            continue;
        }
        set.push(c);
    }
    let mut out = [negate; 256];
    for b in set {
        out[b as usize] = !negate;
    }
    Ok(out)
}

/// `[:name:]` at the start of `s` (just after the `[:`): members and how many
/// bytes of `s` plus the leading ':' it consumed.
fn posix_class(s: &[u8]) -> Option<(Vec<u8>, usize)> {
    type Pred = fn(&u8) -> bool;
    const CLASSES: &[(&[u8], Pred)] = &[
        (b"alnum", u8::is_ascii_alphanumeric),
        (b"alpha", u8::is_ascii_alphabetic),
        (b"blank", |b| *b == b' ' || *b == b'\t'),
        (b"cntrl", u8::is_ascii_control),
        (b"digit", u8::is_ascii_digit),
        (b"graph", u8::is_ascii_graphic),
        (b"lower", u8::is_ascii_lowercase),
        (b"print", |b| b.is_ascii_graphic() || *b == b' '),
        (b"punct", u8::is_ascii_punctuation),
        (b"space", |b| {
            matches!(b, b' ' | b'\t' | b'\n' | 11 | 12 | b'\r')
        }),
        (b"upper", u8::is_ascii_uppercase),
        (b"xdigit", u8::is_ascii_hexdigit),
    ];
    for (name, pred) in CLASSES {
        if s.starts_with(name) && s[name.len()..].starts_with(b":]") {
            // BWK writes the members into a C string, so a NUL member (cntrl)
            // would end the class early; start at 1 like it effectively does.
            let members = (1..=255u8).filter(|b| pred(b)).collect();
            return Some((members, 1 + name.len() + 2));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qstring_matches_bwk() {
        assert_eq!(qstring(br"\t"), b"\t");
        assert_eq!(qstring(br"\054"), b",");
        assert_eq!(qstring(br"a\qb"), b"aqb");
        assert_eq!(qstring(br"\9"), b"\t");
        assert_eq!(qstring(br"x\"), br"x\");
        assert_eq!(qstring(br"ab\0cd"), b"ab");
    }

    #[test]
    fn atof_is_strtod_prefix() {
        assert_eq!(atof(b"0x1A"), 26.0);
        assert_eq!(atof(b" 12abc"), 12.0);
        assert_eq!(atof(b"1e5x"), 100000.0);
        assert_eq!(atof(b"1e"), 1.0);
        assert_eq!(atof(b"-.5"), -0.5);
        assert_eq!(atof(b"5."), 5.0);
        assert_eq!(atof(b"abc"), 0.0);
        assert!(atof(b"-").is_sign_positive());
        assert!(atof(b"-0").is_sign_negative());
        assert_eq!(atof(b"Infinity"), f64::INFINITY);
        assert!(atof(b"nan").is_nan());
        assert_eq!(atof(b"0x1p4"), 16.0);
        assert_eq!(atof(b"0x"), 0.0);
        assert_eq!(atof(b"1e400"), f64::INFINITY);
    }

    #[test]
    fn strnum_rules() {
        assert_eq!(is_number(b" 1 "), Some(1.0));
        assert_eq!(is_number(b"0x10"), Some(16.0));
        assert_eq!(is_number(b"-inf"), Some(f64::NEG_INFINITY));
        assert!(is_number(b"nan(12)").unwrap().is_nan());
        assert_eq!(is_number(b"inf"), None);
        assert_eq!(is_number(b"-1e400"), None);
        assert_eq!(is_number(b"1e-320"), None);
        assert_eq!(is_number(b"0e-999"), Some(0.0));
        assert_eq!(is_number(b"1e5x"), None);
        assert_eq!(is_number(b""), None);
        assert_eq!(is_number(b"."), None);
    }

    #[test]
    fn number_to_string() {
        assert_eq!(num_to_str(18.0), "18");
        assert_eq!(num_to_str(1e20), "100000000000000000000");
        assert_eq!(num_to_str(1e300), "1.0000000000000000525047602552e+300");
        assert_eq!(num_to_str(0.1), "0.1");
        assert_eq!(num_to_str(1.0 / 3.0), "0.333333");
        assert_eq!(num_to_str(1234567.5), "1.23457e+06");
        assert_eq!(num_to_str(0.00001), "1e-05");
        assert_eq!(num_to_str(-0.0), "-0");
        assert_eq!(num_to_str(f64::NEG_INFINITY), "-inf");
        assert_eq!(num_to_str(f64::NAN), "nan");
    }

    #[test]
    fn ere_translation() {
        let m = |re: &str, s: &str| {
            regex::bytes::Regex::new(&translate_ere(re.as_bytes()).unwrap())
                .unwrap()
                .is_match(s.as_bytes())
        };
        assert!(m("^[^@]+@[^@]+$", "a@b"));
        assert!(!m("^[^@]+@[^@]+$", "a@b@c"));
        assert!(m(r"^\d$", "d")); // \d is a literal d in awk
        assert!(m("[]a]", "]"));
        assert!(m("a.b", "a\nb"));
        assert!(m("^[[:digit:]]{3}$", "123"));
        assert!(m("x[z-a]y", "xy"));
        assert!(!m("x[^z-a]y", "xy"));
        assert!(m("[a-c-e]", "d"));
        assert!(!m(r"[a\-z]", "m"));
        assert!(m("[a-]", "-"));
    }
}
