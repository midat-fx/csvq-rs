//! Differential fuzzer: generate random and edge-case inputs, run the same
//! csvq command through two implementations (by default the legacy Bash+awk
//! script under BWK awk, and the Rust binary), and compare stdout, stderr,
//! exit status and every file left in the working directory, byte for byte.
//! A mismatch is shrunk (lines, then bytes) and saved as a reproducer.
//!
//!   difffuzz [--cases N] [--seed S] [--jobs J] [--a IMPL] [--b IMPL]
//!            [--out DIR] [--json FILE] [--max-fail N] [--no-shrink]
//!
//! IMPL is `rust:PATH` or `legacy:AWK` (runs legacy/csvq.sh with CSVQ_AWK=AWK).
//! Defaults: --a legacy:/usr/bin/awk, --b rust:<csvq next to this binary>.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- random

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

// ---------------------------------------------------------------- case model

#[derive(Clone)]
struct Case {
    cmd: &'static str,
    files: Vec<(String, Vec<u8>)>,
    args: Vec<Vec<u8>>,
}

const NAMES: &[&str] = &[
    "id", "name", "email", "age", "score", "city", "amount", "active", "date", "note", "", "a b",
    "Id", "x,y", "\"q\"", "naïve", "@FILE",
];

const VALUES: &[&[u8]] = &[
    b"",
    b"",
    b"0",
    b"1",
    b"2",
    b"3",
    b"17",
    b"18",
    b"42",
    b"-5",
    b"-0",
    b"007",
    b"99999999999999999999",
    b"1.5",
    b"-.5",
    b"5.",
    b".",
    b"1e5",
    b"1E-3",
    b"1.5e3",
    b"2.675",
    b"0.125",
    b"inf",
    b"nan",
    b"-inf",
    b"0x1A",
    b" 12",
    b"12 ",
    b"+3",
    b"1e400",
    b"true",
    b"false",
    b"TRUE",
    b"True",
    b"2024-01-31",
    b"2024-13-99",
    b"2024-1-1",
    b"Almaty",
    b"Astana",
    b"\xd0\xa8\xd1\x8b\xd0\xbc\xd0\xba\xd0\xb5\xd0\xbd\xd1\x82",
    b"na\xc3\xafve",
    b"\xff\xfe",
    b"\xef\xbb\xbfbom",
    b"a@b.c",
    b"bad-email",
    b"a@b@c",
    b"x\x1cy",
    b"ctl\x01",
    b"tab\there",
    b"two words",
    b"#hash",
    b"@EOB",
    b"A",
    b"B",
    b"A",
    b"x",
    b"colour",
    b"ab",
    b"abab",
    b"\\",
    b"a.b",
    b"a\nb",
    b"-1e400",
    b"1e-400",
    b"4e-320",
];

const DELIMS: &[u8] = b",,,,;\t|";

const MINMAX: &[&str] = &[
    "0",
    "1",
    "18",
    "-5",
    "1.5",
    "1e3",
    "0x10",
    "abc",
    "",
    "inf",
    "-inf",
    "nan",
    "1e400",
    "0.1",
    "100000000000000000000",
    "3.14159265",
    ".5",
    "5.",
    "1e-7",
    "-0",
    "120",
];

const REGEXES: &[&str] = &[
    "^[^@]+@[^@]+$",
    "^[0-9]+$",
    "^[A-Z]",
    "[a-z]",
    "^(true|false)$",
    "^[0-9]{4}-[0-9]{2}-[0-9]{2}$",
    "a.b",
    "^$",
    "x*",
    r"^\d$",
    r"\.",
    "[[:alpha:]]+",
    "[[:digit:][:space:]]",
    "^[^,]*$",
    "[]a]",
    "[a-]",
    "[z-a]",
    "colou?r",
    "^.{3,5}$",
    "(ab)+",
    r"\t",
    r"[\]]",
    "^[[:upper:]][[:lower:]]*$",
    "\u{e9}",
    "[^[:print:]]",
    r"^-?[0-9]+(\.[0-9]+)?$",
    "^.$",
    r"\x41",
    r"\101",
];

const DARGS: &[&str] = &[
    ",", ";", "\\t", "|", "", "ab", "\\054", "x", "\\", "\\q", " ", "\\073", "\\n", "\"",
];

fn value(r: &mut Rng) -> Vec<u8> {
    if r.chance(0.01) {
        // a huge field
        // The legacy parser concatenates one character at a time, which is
        // quadratic: 100k is about a second in BWK awk, 300k about 30.
        let n = *r.pick(&[1_000, 20_000, 100_000]);
        return (0..n)
            .map(|i| b"abcdefgh,\"\n"[if i % 97 == 0 { r.below(11) } else { i % 8 }])
            .collect();
    }
    if r.chance(0.1) {
        let n = r.below(8);
        return (0..n).map(|_| *r.pick(b"ab ,;\t|\"\r\n\x1c.-0")).collect();
    }
    r.pick(VALUES).to_vec()
}

fn cell(r: &mut Rng, v: &[u8], delim: u8) -> Vec<u8> {
    let special = v
        .iter()
        .any(|&b| b == delim || b == b'"' || b == b'\n' || b == b'\r');
    let quote = if special {
        !r.chance(0.08)
    } else {
        r.chance(0.1)
    };
    let mut c = if quote {
        let mut q = vec![b'"'];
        for &b in v {
            if b == b'"' && !r.chance(0.05) {
                q.push(b'"');
            }
            q.push(b);
        }
        if !r.chance(0.03) {
            q.push(b'"');
        }
        q
    } else {
        v.to_vec()
    };
    if r.chance(0.02) {
        let at = r.below(c.len() + 1);
        c.insert(at, b'"');
    }
    c
}

fn header(r: &mut Rng) -> Vec<String> {
    let max = if r.chance(0.1) { 12 } else { 5 };
    let n = 1 + r.below(max);
    (0..n).map(|_| r.pick(NAMES).to_string()).collect()
}

fn csv(r: &mut Rng, hdr: &[String]) -> Vec<u8> {
    if r.chance(0.04) {
        let n = r.below(200);
        return (0..n).map(|_| *r.pick(b"a1,;\t|\"\r\n \x1c")).collect();
    }
    let delim = *r.pick(DELIMS);
    let crlf = r.below(4); // 0 LF, 1 CRLF, 2 mixed, 3 LF
    let mut rows: Vec<Vec<Vec<u8>>> = Vec::new();
    rows.push(hdr.iter().map(|h| h.as_bytes().to_vec()).collect());
    let nrows = if r.chance(0.05) {
        150 + r.below(200)
    } else {
        r.below(25)
    };
    let wide = r.chance(0.05);
    for i in 0..nrows {
        if i > 0 && r.chance(0.15) {
            let j = 1 + r.below(rows.len() - 1).min(rows.len() - 1);
            let dup = rows[j.min(rows.len() - 1)].clone();
            rows.push(dup);
            continue;
        }
        let mut n = hdr.len();
        if r.chance(0.08) {
            n = r.below(hdr.len() + 3);
        }
        let row = (0..n)
            .map(|c| {
                if wide && c == 0 {
                    // high-cardinality first column
                    format!("{}", i * 7 + r.below(5)).into_bytes()
                } else {
                    value(r)
                }
            })
            .collect();
        rows.push(row);
    }
    let mut out = Vec::new();
    if r.chance(0.05) {
        out.extend_from_slice(b"\xef\xbb\xbf");
    }
    for (i, row) in rows.iter().enumerate() {
        let cells: Vec<Vec<u8>> = row.iter().map(|v| cell(r, v, delim)).collect();
        out.extend_from_slice(&cells.join(&delim));
        if r.chance(0.03) {
            out.extend_from_slice(b" \t");
        }
        let last = i + 1 == rows.len();
        if last && r.chance(0.2) {
            break;
        }
        if crlf == 1 || (crlf == 2 && r.chance(0.5)) {
            out.push(b'\r');
        }
        out.push(b'\n');
        if r.chance(0.02) {
            out.push(b'\n');
        }
    }
    out
}

fn schema(r: &mut Rng, hdr: &[String]) -> Vec<u8> {
    let mut s = Vec::new();
    for _ in 0..1 + r.below(6) {
        let roll = r.below(20);
        if roll == 0 {
            s.extend_from_slice(b"# comment\n");
            continue;
        }
        if roll == 1 {
            s.extend_from_slice(b"   \n");
            continue;
        }
        let mut toks: Vec<String> = Vec::new();
        if roll != 2 {
            let name = if r.chance(0.85) && !hdr.is_empty() {
                r.pick(hdr).clone()
            } else {
                "missing".into()
            };
            if !name.is_empty() && !name.contains(' ') {
                toks.push(format!("col:{name}"));
            }
        }
        toks.push(format!(
            "type:{}",
            r.pick(&["int", "float", "bool", "date", "string", "string", "integer", ""])
        ));
        if r.chance(0.4) {
            toks.push("required".into());
        }
        if r.chance(0.25) {
            toks.push("unique".into());
        }
        if r.chance(0.3) {
            toks.push(format!("min:{}", r.pick(MINMAX)));
        }
        if r.chance(0.3) {
            toks.push(format!("max:{}", r.pick(MINMAX)));
        }
        if r.chance(0.3) {
            toks.push(format!("regex:{}", r.pick(REGEXES)));
        }
        for i in (1..toks.len()).rev() {
            let j = r.below(i + 1);
            toks.swap(i, j);
        }
        s.extend_from_slice(
            toks.join(if r.chance(0.2) { " \t " } else { " " })
                .as_bytes(),
        );
        if r.chance(0.1) {
            s.push(b'\r');
        }
        s.push(b'\n');
    }
    s
}

/// NEW derived from OLD: change, drop, add and repeat rows.
fn mutate(r: &mut Rng, old: &[u8]) -> Vec<u8> {
    let mut lines: Vec<Vec<u8>> = old.split(|&b| b == b'\n').map(<[u8]>::to_vec).collect();
    for _ in 0..r.below(6) {
        if lines.len() < 2 {
            break;
        }
        let i = 1 + r.below(lines.len() - 1);
        match r.below(5) {
            0 => {
                lines.remove(i);
            }
            4 => {
                // same number, different spelling: exercises strnum compares
                let alt: &[&[u8]] = &[
                    b"1", b"1.0", b" 1", b"+1", b"1e0", b"0x1", b"nan", b"-inf", b"-0", b"0",
                ];
                lines[i] = r.pick(alt).to_vec();
            }
            1 => {
                let l = lines[i].clone();
                lines.insert(i, l);
            }
            2 => lines[i].extend_from_slice(b"9"),
            _ => {
                let mut l = r.pick(VALUES).to_vec();
                l.extend_from_slice(b",x,1");
                lines.insert(i, l);
            }
        }
    }
    if r.chance(0.1) {
        lines.reverse();
    }
    lines.join(&b'\n')
}

fn maybe_delim(r: &mut Rng, args: &mut Vec<Vec<u8>>) {
    if r.chance(0.15) {
        args.push(if r.chance(0.5) {
            b"-d".to_vec()
        } else {
            b"--delim".to_vec()
        });
        args.push(r.pick(DARGS).as_bytes().to_vec());
    }
}

fn keyarg(r: &mut Rng, hdr: &[String]) -> Vec<u8> {
    let n = 1 + r.below(2);
    let mut k: Vec<String> = (0..n)
        .map(|_| {
            if r.chance(0.9) {
                r.pick(hdr).clone()
            } else {
                "nope".into()
            }
        })
        .collect();
    k.dedup();
    k.join(",").into_bytes()
}

fn gen_case(r: &mut Rng) -> Case {
    let hdr = header(r);
    let s = |x: &str| x.as_bytes().to_vec();
    let mut c = Case {
        cmd: "",
        files: Vec::new(),
        args: Vec::new(),
    };
    match r.below(13) {
        0..=2 => {
            c.cmd = "stats";
            c.files.push(("in.csv".into(), csv(r, &hdr)));
            c.args = vec![s("stats"), s("in.csv")];
            maybe_delim(r, &mut c.args);
        }
        3..=4 => {
            c.cmd = "audit";
            c.args = vec![s("audit")];
            for i in 0..1 + r.below(3) {
                let h = header(r);
                let name = format!("f{i}.csv");
                c.files.push((name.clone(), csv(r, &h)));
                c.args.push(name.into_bytes());
            }
            if r.chance(0.2) {
                c.files.push(("d/b.csv".into(), csv(r, &hdr)));
                c.files.push(("d/a.csv".into(), csv(r, &hdr)));
                c.files.push(("d/skip.txt".into(), b"x\n".to_vec()));
                c.args.push(s(r.pick::<&str>(&["d", "d/"])));
            }
            if r.chance(0.2) {
                c.files.push(("empty.csv".into(), Vec::new()));
                c.args.push(s("empty.csv"));
            }
            c.args.extend([s("-o"), s("out")]);
            if r.chance(0.3) {
                c.args.push(s("--strict"));
            }
            maybe_delim(r, &mut c.args);
        }
        5..=6 => {
            c.cmd = "validate";
            c.files.push(("in.csv".into(), csv(r, &hdr)));
            c.files.push(("s.schema".into(), schema(r, &hdr)));
            c.args = vec![s("validate"), s("in.csv"), s("--schema"), s("s.schema")];
            maybe_delim(r, &mut c.args);
        }
        7..=8 => {
            c.cmd = "diff";
            // diff is line-oriented: keep it to one record per line mostly
            let old = csv(r, &hdr);
            let new = mutate(r, &old);
            c.files.push((
                "old.csv".into(),
                if r.chance(0.05) { Vec::new() } else { old },
            ));
            c.files.push(("new.csv".into(), new));
            c.args = vec![
                s("diff"),
                s("old.csv"),
                s("new.csv"),
                s("--key"),
                keyarg(r, &hdr),
            ];
            maybe_delim(r, &mut c.args);
        }
        9..=10 => {
            c.cmd = "dedupe";
            c.files.push(("in.csv".into(), csv(r, &hdr)));
            c.args = vec![s("dedupe"), s("in.csv")];
            if r.chance(0.5) {
                c.args.extend([s("--key"), keyarg(r, &hdr)]);
            }
            maybe_delim(r, &mut c.args);
        }
        11 => {
            c.cmd = "normalize";
            c.files.push(("in.csv".into(), csv(r, &hdr)));
            c.args = vec![s("normalize"), s("in.csv")];
            if r.chance(0.3) {
                c.args.push(s("--in-place"));
            }
        }
        _ => {
            // command-line handling: bad options, missing values and files
            c.cmd = "cli";
            c.files.push(("in.csv".into(), csv(r, &hdr)));
            c.files.push(("s.schema".into(), schema(r, &hdr)));
            let pool: &[&str] = &[
                "stats",
                "audit",
                "validate",
                "diff",
                "dedupe",
                "normalize",
                "version",
                "help",
                "-h",
                "--version",
                "frob",
                "in.csv",
                "in.csv",
                "missing.csv",
                "s.schema",
                "--schema",
                "--key",
                "-d",
                "--delim",
                "-o",
                "--strict",
                "--in-place",
                "-x",
                "-",
                "",
                "id",
                ",",
                "outdir",
            ];
            let n = r.below(6);
            c.args = (0..n).map(|_| s(r.pick::<&str>(pool))).collect();
            if let Some(first) = c.args.first() {
                if first.as_slice() == b"normalize"
                    && c.args.iter().any(|a| a.as_slice() == b"--in-place")
                {
                    // fine: runs in a scratch copy
                }
            }
        }
    }
    c
}

// ---------------------------------------------------------------- running

#[derive(Clone)]
enum Impl {
    Rust(PathBuf),
    Legacy { script: PathBuf, awk: String },
}

impl Impl {
    fn parse(s: &str, root: &Path) -> Impl {
        if let Some(p) = s.strip_prefix("rust:") {
            Impl::Rust(fs::canonicalize(p).unwrap_or_else(|_| panic!("no such binary: {p}")))
        } else if let Some(a) = s.strip_prefix("legacy:") {
            Impl::Legacy {
                script: root.join("legacy/csvq.sh"),
                awk: a.to_string(),
            }
        } else {
            panic!("IMPL must be rust:PATH or legacy:AWK, got {s}")
        }
    }
    fn label(&self) -> String {
        match self {
            Impl::Rust(_) => "rust".into(),
            Impl::Legacy { awk, .. } => format!("legacy({awk})"),
        }
    }
}

#[derive(PartialEq, Eq, Clone)]
struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: Option<i32>,
    files: BTreeMap<String, Vec<u8>>,
    timed_out: bool,
}

fn snapshot(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
        if p.is_dir() {
            out.insert(format!("{rel}/"), Vec::new());
            snapshot(&p, base, out);
        } else {
            out.insert(rel, fs::read(&p).unwrap_or_default());
        }
    }
}

const TIMEOUT: Duration = Duration::from_secs(30);

fn run(imp: &Impl, case: &Case, dir: &Path) -> Outcome {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
    for (name, data) in &case.files {
        let p = dir.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }
    let mut cmd = match imp {
        Impl::Rust(bin) => Command::new(bin),
        Impl::Legacy { script, awk } => {
            let mut c = Command::new("bash");
            c.arg(script).env("CSVQ_AWK", awk);
            c
        }
    };
    let args: Vec<&std::ffi::OsStr> = case
        .args
        .iter()
        .map(|a| std::ffi::OsStr::from_bytes(a))
        .collect();
    let mut child = cmd
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let t1 = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.read_to_end(&mut b);
        b
    });
    let t2 = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = se.read_to_end(&mut b);
        b
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break Some(s);
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            timed_out = true;
            break child.wait().ok();
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let stdout = t1.join().unwrap();
    let stderr = t2.join().unwrap();
    let mut files = BTreeMap::new();
    snapshot(dir, dir, &mut files);
    Outcome {
        stdout,
        stderr,
        code: status.and_then(|s| s.code()),
        files,
        timed_out,
    }
}

fn compare(a: &Outcome, b: &Outcome) -> Option<String> {
    if a.timed_out || b.timed_out {
        return Some(format!("timeout ({})", if a.timed_out { "a" } else { "b" }));
    }
    if a.code != b.code {
        return Some(format!("exit code {:?} vs {:?}", a.code, b.code));
    }
    if a.stdout != b.stdout {
        return Some("stdout".into());
    }
    if a.stderr != b.stderr {
        return Some("stderr".into());
    }
    if a.files != b.files {
        let k = a
            .files
            .iter()
            .find(|(k, v)| b.files.get(*k) != Some(v))
            .map(|(k, _)| k.clone())
            .or_else(|| b.files.keys().find(|k| !a.files.contains_key(*k)).cloned())
            .unwrap_or_default();
        return Some(format!("file {k}"));
    }
    None
}

struct Ctx {
    a: Impl,
    b: Impl,
    work: PathBuf,
}

impl Ctx {
    fn check(&self, case: &Case, tag: &str) -> (Option<String>, Outcome, Outcome) {
        let da = self.work.join(format!("{tag}-a"));
        let db = self.work.join(format!("{tag}-b"));
        let oa = run(&self.a, case, &da);
        let ob = run(&self.b, case, &db);
        let _ = fs::remove_dir_all(&da);
        let _ = fs::remove_dir_all(&db);
        (compare(&oa, &ob), oa, ob)
    }

    /// Greedy delta debugging over each file: drop chunks of lines, then
    /// single bytes, while the case still diverges in the same way.
    fn shrink(&self, mut case: Case, kind: &str, tag: &str) -> Case {
        let mut budget = 400;
        let still = |c: &Case, budget: &mut i32| {
            *budget -= 1;
            matches!(self.check(c, tag).0, Some(k) if k == kind)
        };
        for fi in 0..case.files.len() {
            for unit_bytes in [false, true] {
                let mut chunk = case.files[fi].1.len().max(1);
                while chunk >= 1 && budget > 0 {
                    let parts: Vec<Vec<u8>> = if unit_bytes {
                        case.files[fi].1.iter().map(|&b| vec![b]).collect()
                    } else {
                        case.files[fi]
                            .1
                            .split_inclusive(|&b| b == b'\n')
                            .map(<[u8]>::to_vec)
                            .collect()
                    };
                    if unit_bytes && parts.len() > 4000 {
                        break;
                    }
                    let mut changed = false;
                    let mut i = 0;
                    let mut cur = parts;
                    while i < cur.len() && budget > 0 {
                        let end = (i + chunk).min(cur.len());
                        let mut trial = case.clone();
                        trial.files[fi].1 = [&cur[..i], &cur[end..]].concat().concat();
                        if still(&trial, &mut budget) {
                            cur.drain(i..end);
                            case = trial;
                            changed = true;
                        } else {
                            i = end;
                        }
                    }
                    if !changed {
                        if chunk == 1 {
                            break;
                        }
                        chunk = chunk.div_ceil(2);
                    }
                }
            }
        }
        case
    }
}

fn shell_quote(a: &[u8]) -> String {
    let s = String::from_utf8_lossy(a);
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./,:=".contains(&b))
    {
        s.into_owned()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

fn save(dir: &Path, case: &Case, kind: &str, ctx: &Ctx, oa: &Outcome, ob: &Outcome) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir.join("input")).unwrap();
    for (name, data) in &case.files {
        let p = dir.join("input").join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }
    let args: Vec<String> = case.args.iter().map(|a| shell_quote(a)).collect();
    let mut info = format!("divergence: {kind}\ncommand: csvq {}\n", args.join(" "));
    for (lab, o) in [(ctx.a.label(), oa), (ctx.b.label(), ob)] {
        let _ = write!(
            info,
            "\n== {lab}: exit {:?}\n-- stdout\n{}\n-- stderr\n{}\n",
            o.code,
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        for (k, v) in &o.files {
            if !case.files.iter().any(|(n, d)| n == k && d == v) {
                let _ = write!(info, "-- file {k}\n{}\n", String::from_utf8_lossy(v));
            }
        }
    }
    fs::write(dir.join("README.txt"), info).unwrap();
}

fn js(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn main() {
    let mut cases = 2000usize;
    let mut seed = 1u64;
    let mut jobs = 4usize;
    let mut max_fail = 20usize;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let me = std::env::current_exe().unwrap();
    let mut a = "legacy:/usr/bin/awk".to_string();
    let mut b = format!("rust:{}", me.with_file_name("csvq").display());
    let mut out = root.join("fuzz-out");
    let mut json: Option<PathBuf> = None;
    let mut no_shrink = false;
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut it = argv.iter();
    while let Some(k) = it.next() {
        let mut v = || {
            it.next()
                .unwrap_or_else(|| panic!("{k} needs a value"))
                .clone()
        };
        match k.as_str() {
            "--cases" => cases = v().parse().unwrap(),
            "--seed" => seed = v().parse().unwrap(),
            "--jobs" => jobs = v().parse().unwrap(),
            "--max-fail" => max_fail = v().parse().unwrap(),
            "--a" => a = v(),
            "--b" => b = v(),
            "--out" => out = PathBuf::from(v()),
            "--json" => json = Some(PathBuf::from(v())),
            "--no-shrink" => no_shrink = true,
            _ => panic!("unknown argument {k}"),
        }
    }
    let work = std::env::temp_dir().join(format!("csvq-difffuzz-{}", std::process::id()));
    fs::create_dir_all(&work).unwrap();
    let ctx = Ctx {
        a: Impl::parse(&a, &root),
        b: Impl::parse(&b, &root),
        work: work.clone(),
    };

    let next = AtomicUsize::new(0);
    let fails = AtomicUsize::new(0);
    // command -> (cases, diverged, timed out)
    let stats: Mutex<BTreeMap<&'static str, (u64, u64, u64)>> = Mutex::new(BTreeMap::new());
    let findings: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let start = Instant::now();
    std::thread::scope(|s| {
        for w in 0..jobs {
            let (ctx, next, fails, stats, findings, out) =
                (&ctx, &next, &fails, &stats, &findings, &out);
            s.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= cases || fails.load(Ordering::Relaxed) >= max_fail {
                    break;
                }
                let mut r = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ (i as u64));
                r.next();
                let case = gen_case(&mut r);
                let tag = format!("w{w}");
                let (kind, _, _) = ctx.check(&case, &tag);
                // A run that hits the time limit says nothing about
                // equivalence: count it apart and do not shrink it.
                let timeout = kind.as_deref().is_some_and(|k| k.starts_with("timeout"));
                {
                    let mut st = stats.lock().unwrap();
                    let e = st.entry(case.cmd).or_default();
                    e.0 += 1;
                    e.1 += u64::from(kind.is_some() && !timeout);
                    e.2 += u64::from(timeout);
                }
                if timeout {
                    eprintln!("TIMEOUT case {i} [{}] {}", case.cmd, kind.unwrap());
                } else if let Some(kind) = kind {
                    fails.fetch_add(1, Ordering::Relaxed);
                    let small = if no_shrink {
                        case
                    } else {
                        ctx.shrink(case, &kind, &tag)
                    };
                    let (_, oa, ob) = ctx.check(&small, &tag);
                    let dir = out.join(format!("seed{seed}-case{i}"));
                    save(&dir, &small, &kind, ctx, &oa, &ob);
                    let args: Vec<String> = small.args.iter().map(|a| shell_quote(a)).collect();
                    let line = format!(
                        "case {i} [{}] {kind}: csvq {} -> {}",
                        small.cmd,
                        args.join(" "),
                        dir.display()
                    );
                    eprintln!("DIVERGENCE {line}");
                    findings.lock().unwrap().push(line);
                }
                if (i + 1) % 1000 == 0 {
                    eprintln!("... {} cases", i + 1);
                }
            });
        }
    });
    let _ = fs::remove_dir_all(&work);
    let secs = start.elapsed().as_secs_f64();
    let st = stats.into_inner().unwrap();
    let total: u64 = st.values().map(|v| v.0).sum();
    let bad: u64 = st.values().map(|v| v.1).sum();
    let slow: u64 = st.values().map(|v| v.2).sum();
    println!(
        "difffuzz: {} vs {}, seed {seed}, {total} cases in {secs:.1}s",
        ctx.a.label(),
        ctx.b.label()
    );
    println!(
        "{:<10} {:>7} {:>9} {:>9} {:>9}",
        "command", "cases", "identical", "diverged", "timeout"
    );
    let row = |k: &str, v: (u64, u64, u64)| {
        println!(
            "{k:<10} {:>7} {:>9} {:>9} {:>9}",
            v.0,
            v.0 - v.1 - v.2,
            v.1,
            v.2
        );
    };
    for (k, v) in &st {
        row(k, *v);
    }
    row("total", (total, bad, slow));
    if let Some(j) = json {
        let mut s = format!(
            "{{\n  \"a\": {},\n  \"b\": {},\n  \"seed\": {seed},\n  \"cases\": {total},\n  \"identical\": {},\n  \"diverged\": {bad},\n  \"timeouts\": {slow},\n  \"seconds\": {secs:.1},\n  \"by_command\": {{",
            js(&ctx.a.label()),
            js(&ctx.b.label()),
            total - bad - slow
        );
        let parts: Vec<String> = st
            .iter()
            .map(|(k, v)| {
                format!(
                    "\n    \"{k}\": {{\"cases\": {}, \"identical\": {}, \"diverged\": {}, \"timeouts\": {}}}",
                    v.0,
                    v.0 - v.1 - v.2,
                    v.1,
                    v.2
                )
            })
            .collect();
        s.push_str(&parts.join(","));
        s.push_str("\n  },\n  \"findings\": [");
        let f = findings.into_inner().unwrap();
        let fl: Vec<String> = f.iter().map(|l| format!("\n    {}", js(l))).collect();
        s.push_str(&fl.join(","));
        s.push_str("\n  ]\n}\n");
        fs::write(j, s).unwrap();
    }
    if bad > 0 {
        std::process::exit(1);
    }
}
