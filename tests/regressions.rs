//! Frozen differential cases: every divergence the fuzzer ever found, plus
//! hand-written probes of legacy quirks, with the legacy (BWK awk) output
//! stored next to the input. Replayed against the Rust binary; no awk needed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

fn files(dir: &Path, base: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, base, out);
        } else {
            out.push((
                p.strip_prefix(base).unwrap().to_path_buf(),
                fs::read(&p).unwrap(),
            ));
        }
    }
}

#[test]
fn frozen_cases_match_legacy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let mut names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    names.sort();
    let mut failed = Vec::new();
    for case in &names {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let work = std::env::temp_dir().join(format!("csvq-reg-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        copy_dir(&case.join("input"), &work);
        let args = fs::read_to_string(case.join("args")).unwrap();
        // One argument per line; an empty file means no arguments at all.
        let args: Vec<&str> = if args.is_empty() {
            Vec::new()
        } else {
            args.strip_suffix('\n')
                .unwrap_or(&args)
                .split('\n')
                .collect()
        };
        let out = Command::new(env!("CARGO_BIN_EXE_csvq"))
            .args(&args)
            .current_dir(&work)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let exp = case.join("expected");
        let want_code: i32 = fs::read_to_string(exp.join("code"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut problems = Vec::new();
        if out.status.code() != Some(want_code) {
            problems.push(format!("exit {:?}, want {want_code}", out.status.code()));
        }
        if out.stdout != fs::read(exp.join("stdout")).unwrap() {
            problems.push("stdout".into());
        }
        if out.stderr != fs::read(exp.join("stderr")).unwrap() {
            problems.push("stderr".into());
        }
        let mut want = Vec::new();
        files(&exp.join("files"), &exp.join("files"), &mut want);
        for (rel, data) in want {
            if fs::read(work.join(&rel)).ok().as_ref() != Some(&data) {
                problems.push(format!("file {}", rel.display()));
            }
        }
        if !problems.is_empty() {
            failed.push(format!("{name}: {}", problems.join(", ")));
        }
        let _ = fs::remove_dir_all(&work);
    }
    assert!(
        failed.is_empty(),
        "{} of {} cases differ:\n{}",
        failed.len(),
        names.len(),
        failed.join("\n")
    );
    assert!(
        names.len() >= 10,
        "expected the frozen corpus, found {}",
        names.len()
    );
}
