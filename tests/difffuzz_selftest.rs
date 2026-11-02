//! The fuzzer is the main evidence of equivalence, so it gets a test of its
//! own: pointed at two copies of the same binary it must report nothing, and
//! pointed at a planted bug it must report it, shrink it and save it.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CSVQ: &str = env!("CARGO_BIN_EXE_csvq");
const DIFFFUZZ: &str = env!("CARGO_BIN_EXE_difffuzz");

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("csvq-selftest-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn difffuzz(a: &str, out: &Path, extra: &[&str]) -> Output {
    Command::new(DIFFFUZZ)
        .args(["--cases", "300", "--seed", "1", "--jobs", "2"])
        .args(["--a", a, "--out", out.to_str().unwrap()])
        .args(extra)
        .output()
        .unwrap()
}

fn input_files(dir: &Path, out: &mut Vec<Vec<u8>>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_dir() {
            input_files(&e.path(), out);
        } else {
            out.push(fs::read(e.path()).unwrap());
        }
    }
}

#[test]
fn same_binary_never_diverges() {
    let d = scratch("same");
    let o = difffuzz(&format!("rust:{CSVQ}"), &d.join("out"), &[]);
    let report = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "{report}");
    let total = report
        .lines()
        .find(|l| l.starts_with("total"))
        .unwrap_or("");
    // command, cases, identical, diverged, timeout
    assert_eq!(
        total.split_whitespace().collect::<Vec<_>>(),
        ["total", "300", "300", "0", "0"]
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn planted_bug_is_found_shrunk_and_saved() {
    let d = scratch("mutant");
    // A mutant that prints one extra byte when any file in its working
    // directory contains a CR, the kind of input-dependent slip a port makes.
    let mutant = d.join("mutant.sh");
    fs::write(
        &mutant,
        format!(
            "#!/bin/sh\n\"{CSVQ}\" \"$@\"\ns=$?\ngrep -rqF \"$(printf '\\r')\" . && printf X\nexit $s\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&mutant, fs::Permissions::from_mode(0o755)).unwrap();

    let out = d.join("out");
    let o = difffuzz(
        &format!("rust:{}", mutant.display()),
        &out,
        &["--max-fail", "3"],
    );
    let report = String::from_utf8_lossy(&o.stdout);
    assert_eq!(
        o.status.code(),
        Some(1),
        "divergence not reported: {report}"
    );

    let cases: Vec<_> = fs::read_dir(&out).unwrap().flatten().collect();
    assert!(!cases.is_empty(), "no reproducer saved");
    for c in cases {
        let readme = fs::read_to_string(c.path().join("README.txt")).unwrap();
        assert!(readme.contains("divergence: stdout"), "{readme}");
        let mut inputs = Vec::new();
        input_files(&c.path().join("input"), &mut inputs);
        // Shrinking must keep the trigger and drop nearly everything else.
        assert!(
            inputs.iter().any(|f| f.contains(&b'\r')),
            "CR lost: {readme}"
        );
        let total: usize = inputs.iter().map(Vec::len).sum();
        assert!(
            total <= 16,
            "{total} input bytes left after shrinking: {readme}"
        );
    }
    let _ = fs::remove_dir_all(&d);
}
