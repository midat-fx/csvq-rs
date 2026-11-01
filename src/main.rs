//! The `csvq` command line: a faithful port of the legacy csvq.sh wrapper,
//! including its argument quirks and exit codes (2 for usage errors, 1 for
//! failed gates and for an option missing its value).

use csvq_rs::awk::qstring;
use csvq_rs::{audit, dedupe, diff, stats, validate};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufReader, BufWriter, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::exit;

const USAGE: &str = "\
csvq — deterministic CSV data-quality toolkit

Usage:
  csvq audit    <paths...> [-o DIR] [-d DELIM] [--strict]
  csvq stats    <file> [-d DELIM]
  csvq validate <file> --schema FILE [-d DELIM]
  csvq diff     <old> <new> --key COL[,COL] [-d DELIM]
  csvq dedupe   <file> [--key COL[,COL]] [-d DELIM]
  csvq normalize <file> [--in-place]
  csvq version
  csvq help

Commands:
  audit      Write analysis_report.txt, data_summary.txt, duplicates_found.txt
             into DIR (default ./out). --strict exits 1 if any file has
             duplicate or ragged rows (use it as a CI gate).
  stats      Per-column type, null rate, cardinality, min/max/mean, top values.
  validate   Check a file against a schema DSL (docs/schema-dsl.md). Exits 1
             on any violation.
  diff       Keyed row diff between two CSVs (added / changed / removed).
  dedupe     Drop duplicate rows (whole row, or by --key), keeping the first.
  normalize  CRLF->LF, strip trailing whitespace, ensure a final newline.

Global:
  -d, --delim DELIM   Force the delimiter (default: auto-detect from header).
  All output is deterministic and LC_ALL=C sorted.
";

fn die(msg: &[u8]) -> ! {
    let mut e = io::stderr().lock();
    let _ = e.write_all(b"csvq: ");
    let _ = e.write_all(msg);
    let _ = e.write_all(b"\n");
    exit(2)
}

fn path(b: &[u8]) -> &Path {
    Path::new(OsStr::from_bytes(b))
}

/// `[ -f "$1" ] || die "no such file: $1"`
fn require_file(p: &[u8]) {
    if !path(p).is_file() {
        die(&[b"no such file: ", p].concat());
    }
}

/// Failures the wrapper would hit inside awk (unreadable file and the like).
fn io_fail(e: io::Error) -> ! {
    if e.kind() == io::ErrorKind::BrokenPipe {
        exit(141);
    }
    die(e.to_string().as_bytes())
}

/// The wrapper's `while [ $# -gt 0 ]; do case "$1" in ...; esac; shift; done`.
struct Args {
    v: Vec<Vec<u8>>,
    i: usize,
}

impl Args {
    fn next(&mut self) -> Option<Vec<u8>> {
        let a = self.v.get(self.i).cloned();
        self.i += 1;
        a
    }

    /// `shift; x="${1:-}"`, then the loop's own `shift`. When the value is
    /// missing that second `shift` fails and `set -e` ends the script with
    /// status 1 and no message (legacy quirk L9).
    fn value(&mut self) -> Vec<u8> {
        match self.v.get(self.i) {
            Some(x) => {
                self.i += 1;
                x.clone()
            }
            None => exit(1),
        }
    }
}

fn unknown(cmd: &str, opt: &[u8]) -> ! {
    die(&[cmd.as_bytes(), b": unknown option ", opt].concat())
}

fn open(p: &[u8]) -> BufReader<fs::File> {
    BufReader::with_capacity(
        1 << 20,
        fs::File::open(path(p)).unwrap_or_else(|e| io_fail(e)),
    )
}

fn finish(out: &mut impl Write) {
    if let Err(e) = out.flush() {
        io_fail(e);
    }
}

/// `collect_files`: directories become their `*.csv` children (find
/// -maxdepth 1 -type f, C-sorted), files pass through. It runs in a process
/// substitution, so a missing path prints an error and stops collecting but
/// does not stop audit: the files gathered so far are still audited (L8).
fn collect_files(paths: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut files = Vec::new();
    for p in paths {
        if !p.is_empty() && path(p).is_dir() {
            let mut found: Vec<Vec<u8>> = Vec::new();
            if let Ok(rd) = fs::read_dir(path(p)) {
                for e in rd.flatten() {
                    let name = e.file_name().into_vec();
                    if name.ends_with(b".csv") && e.file_type().is_ok_and(|t| t.is_file()) {
                        let sep: &[u8] = if p.ends_with(b"/") { b"" } else { b"/" };
                        found.push([&p[..], sep, &name].concat());
                    }
                }
            }
            found.sort();
            files.extend(found);
        } else if path(p).is_file() {
            files.push(p.clone());
        } else {
            let mut e = io::stderr().lock();
            let _ = e.write_all(&[b"csvq: no such file: ", &p[..], b"\n"].concat());
            break;
        }
    }
    files
}

fn cmd_audit(mut a: Args) {
    let mut outdir = b"out".to_vec();
    let mut delim = Vec::new();
    let mut strict = false;
    let mut paths = Vec::new();
    while let Some(arg) = a.next() {
        match &arg[..] {
            b"-o" => {
                outdir = a.v.get(a.i).cloned().unwrap_or_default();
                if outdir.is_empty() {
                    die(b"audit: -o needs a directory");
                }
                a.i += 1;
            }
            b"-d" | b"--delim" => delim = a.value(),
            b"--strict" => strict = true,
            o if o.starts_with(b"-") => unknown("audit", o),
            _ => paths.push(arg),
        }
    }
    if paths.is_empty() {
        die(b"audit: no input files");
    }
    let files = collect_files(&paths);
    if files.is_empty() {
        die(b"audit: no .csv files found");
    }
    let out = PathBuf::from(OsStr::from_bytes(&outdir));
    if let Err(e) = fs::create_dir_all(&out) {
        let mut err = io::stderr().lock();
        let _ = err.write_all(
            &[
                b"mkdir: ",
                &outdir[..],
                b": ",
                e.to_string().as_bytes(),
                b"\n",
            ]
            .concat(),
        );
        exit(1);
    }
    let sep = qstring(&delim);
    let mut facts = Vec::new();
    for f in &files {
        audit::facts(path(f), &qstring(f), &sep, &mut facts).unwrap_or_else(|e| io_fail(e));
    }
    let gate = audit::build_reports(&facts, &out).unwrap_or_else(|e| io_fail(e));
    let mut err = io::stderr().lock();
    let _ = err.write_all(&[b"csvq audit: wrote 3 report(s) to ", &outdir[..], b"/\n"].concat());
    if strict && (gate.files_dupes > 0 || gate.files_ragged > 0) {
        let _ = writeln!(
            err,
            "csvq audit: --strict gate failed ({} file(s) with dupes, {} with ragged rows)",
            gate.files_dupes, gate.files_ragged
        );
        exit(1);
    }
}

/// Shared shape of stats / dedupe: one file plus -d (and --key for dedupe).
fn one_file(cmd: &str, mut a: Args, with_key: bool) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (mut file, mut delim, mut key) = (Vec::new(), Vec::new(), Vec::new());
    while let Some(arg) = a.next() {
        match &arg[..] {
            b"--key" if with_key => key = a.value(),
            b"-d" | b"--delim" => delim = a.value(),
            o if o.starts_with(b"-") => unknown(cmd, o),
            _ => file = arg,
        }
    }
    if file.is_empty() {
        die(&[cmd.as_bytes(), b": missing file"].concat());
    }
    require_file(&file);
    (file, delim, key)
}

fn cmd_stats(a: Args) {
    let (file, delim, _) = one_file("stats", a, false);
    let mut out = BufWriter::with_capacity(1 << 16, io::stdout().lock());
    stats::run(open(&file), &qstring(&delim), &qstring(&file), &mut out)
        .unwrap_or_else(|e| io_fail(e));
    finish(&mut out);
}

fn cmd_dedupe(a: Args) {
    let (file, delim, key) = one_file("dedupe", a, true);
    let mut out = BufWriter::with_capacity(1 << 16, io::stdout().lock());
    let removed = dedupe::run(open(&file), &qstring(&delim), &qstring(&key), &mut out)
        .unwrap_or_else(|e| io_fail(e));
    finish(&mut out);
    eprintln!("csvq dedupe: removed {removed} duplicate row(s)");
}

fn cmd_validate(mut a: Args) {
    let (mut file, mut schema, mut delim) = (Vec::new(), Vec::new(), Vec::new());
    while let Some(arg) = a.next() {
        match &arg[..] {
            b"--schema" => schema = a.value(),
            b"-d" | b"--delim" => delim = a.value(),
            o if o.starts_with(b"-") => unknown("validate", o),
            _ => file = arg,
        }
    }
    if file.is_empty() {
        die(b"validate: missing file");
    }
    if schema.is_empty() {
        die(b"validate: --schema is required");
    }
    require_file(&file);
    require_file(&schema);
    let text = fs::read(path(&schema)).unwrap_or_else(|e| io_fail(e));
    let rules = validate::load_schema(&text)
        .unwrap_or_else(|e| die(&[b"bad regex in schema: ", e.as_bytes()].concat()));
    let mut out = BufWriter::with_capacity(1 << 16, io::stdout().lock());
    let n = validate::run(
        open(&file),
        &rules,
        &qstring(&delim),
        &qstring(&file),
        &qstring(&schema),
        &mut out,
    )
    .unwrap_or_else(|e| io_fail(e));
    finish(&mut out);
    if n > 0 {
        exit(1);
    }
}

fn cmd_diff(mut a: Args) {
    let (mut old, mut new, mut key, mut delim) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    while let Some(arg) = a.next() {
        match &arg[..] {
            b"--key" => key = a.value(),
            b"-d" | b"--delim" => delim = a.value(),
            o if o.starts_with(b"-") => unknown("diff", o),
            _ if old.is_empty() => old = arg,
            _ if new.is_empty() => new = arg,
            _ => die(b"diff: too many files"),
        }
    }
    if old.is_empty() || new.is_empty() {
        die(b"diff: needs OLD and NEW files");
    }
    if key.is_empty() {
        die(b"diff: --key is required");
    }
    require_file(&old);
    require_file(&new);
    let mut out = BufWriter::with_capacity(1 << 16, io::stdout().lock());
    let head = [b"CSV DIFF (key: ", &key[..], b")\n======================\n"].concat();
    out.write_all(&head)
        .and_then(|()| {
            diff::run(
                open(&old),
                open(&new),
                &qstring(&delim),
                &qstring(&key),
                &mut out,
            )
        })
        .unwrap_or_else(|e| io_fail(e));
    finish(&mut out);
}

/// `{ sub(/\r$/, ""); sub(/[ \t]+$/, ""); print }`
fn normalize_into(input: impl io::BufRead, out: &mut impl Write) -> io::Result<()> {
    let mut lines = csvq_rs::csv::Lines::new(input);
    while let Some((line, _)) = lines.next_line()? {
        let end = line
            .iter()
            .rposition(|&b| b != b' ' && b != b'\t')
            .map_or(0, |p| p + 1);
        out.write_all(&line[..end])?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn cmd_normalize(mut a: Args) {
    let mut file = Vec::new();
    let mut inplace = false;
    while let Some(arg) = a.next() {
        match &arg[..] {
            b"--in-place" => inplace = true,
            o if o.starts_with(b"-") => unknown("normalize", o),
            _ => file = arg,
        }
    }
    if file.is_empty() {
        die(b"normalize: missing file");
    }
    require_file(&file);
    if inplace {
        // Unlike the legacy mktemp + mv, keep the file's permissions (D3).
        let mut buf = Vec::new();
        normalize_into(open(&file), &mut buf).unwrap_or_else(|e| io_fail(e));
        fs::write(path(&file), buf).unwrap_or_else(|e| io_fail(e));
        let mut err = io::stderr().lock();
        let _ = err.write_all(&[b"csvq normalize: rewrote ", &file[..], b"\n"].concat());
    } else {
        let mut out = BufWriter::with_capacity(1 << 16, io::stdout().lock());
        normalize_into(open(&file), &mut out).unwrap_or_else(|e| io_fail(e));
        finish(&mut out);
    }
}

fn main() {
    let mut v: Vec<Vec<u8>> = std::env::args_os().skip(1).map(|a| a.into_vec()).collect();
    if v.is_empty() {
        eprint!("{USAGE}");
        exit(2);
    }
    let cmd = v.remove(0);
    let a = Args { v, i: 0 };
    match &cmd[..] {
        b"audit" => cmd_audit(a),
        b"stats" => cmd_stats(a),
        b"validate" => cmd_validate(a),
        b"diff" => cmd_diff(a),
        b"dedupe" => cmd_dedupe(a),
        b"normalize" => cmd_normalize(a),
        b"version" | b"-V" | b"--version" => println!("csvq 1.0.0"),
        b"help" | b"-h" | b"--help" => print!("{USAGE}"),
        other => die(&[b"unknown command: ", other, b" (try 'csvq help')"].concat()),
    }
}
