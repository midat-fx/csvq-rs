//! The legacy golden suite (tests/run_tests.sh of csv-data-quality-cli),
//! replayed against the Rust binary with the original fixtures and goldens.

use std::path::Path;
use std::process::Command;

fn csvq(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_csvq"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap()
}

fn golden(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(name),
    )
    .unwrap()
}

fn assert_golden(name: &str, args: &[&str]) {
    let a = csvq(args);
    let b = csvq(args);
    assert_eq!(a.stdout, b.stdout, "{name}: non-deterministic");
    assert_eq!(
        String::from_utf8_lossy(&a.stdout),
        String::from_utf8_lossy(&golden(name)),
        "{name}: golden mismatch"
    );
}

fn code(args: &[&str]) -> i32 {
    csvq(args).status.code().unwrap()
}

const FX: &str = "tests/fixtures";

#[test]
fn audit_reports_match_goldens() {
    let dir = std::env::temp_dir().join(format!("csvq-golden-{}", std::process::id()));
    let files: Vec<String> = ["clean", "crlf", "dupes", "ragged", "empty", "quoted"]
        .iter()
        .map(|f| format!("{FX}/{f}.csv"))
        .collect();
    let mut args: Vec<&str> = vec!["audit"];
    args.extend(files.iter().map(String::as_str));
    let d = dir.to_str().unwrap().to_string();
    args.extend(["-o", &d]);
    assert_eq!(csvq(&args).status.code(), Some(0));
    for (report, g) in [
        ("analysis_report.txt", "audit_analysis.txt"),
        ("data_summary.txt", "audit_summary.txt"),
        ("duplicates_found.txt", "audit_duplicates.txt"),
    ] {
        let got = std::fs::read(dir.join(report)).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(&golden(g)),
            "{report}"
        );
    }
    assert!(!dir.join(".gate").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn command_goldens() {
    assert_golden("stats_basic", &["stats", &format!("{FX}/stats.csv")]);
    assert_golden(
        "stats_semicolon",
        &["stats", &format!("{FX}/semicolon.csv")],
    );
    let schema = format!("{FX}/people.schema");
    assert_golden(
        "validate_fail",
        &["validate", &format!("{FX}/people.csv"), "--schema", &schema],
    );
    assert_golden(
        "validate_ok",
        &[
            "validate",
            &format!("{FX}/people_ok.csv"),
            "--schema",
            &schema,
        ],
    );
    assert_golden(
        "diff_basic",
        &[
            "diff",
            &format!("{FX}/old.csv"),
            &format!("{FX}/new.csv"),
            "--key",
            "id",
        ],
    );
    assert_golden("dedupe_basic", &["dedupe", &format!("{FX}/dupes.csv")]);
    assert_golden(
        "dedupe_by_key",
        &["dedupe", &format!("{FX}/dupes.csv"), "--key", "id"],
    );
    assert_golden("normalize_crlf", &["normalize", &format!("{FX}/crlf.csv")]);
    assert_golden("help_text", &["help"]);
}

#[test]
fn exit_codes() {
    let schema = format!("{FX}/people.schema");
    assert_eq!(
        code(&["validate", &format!("{FX}/people.csv"), "--schema", &schema]),
        1
    );
    assert_eq!(
        code(&[
            "validate",
            &format!("{FX}/people_ok.csv"),
            "--schema",
            &schema
        ]),
        0
    );
    assert_eq!(code(&["version"]), 0);
    assert_eq!(code(&["frobnicate"]), 2);
    assert_eq!(code(&["stats", &format!("{FX}/does_not_exist.csv")]), 2);
    let tmp = std::env::temp_dir().join(format!("csvq-gate-{}", std::process::id()));
    let t = tmp.to_str().unwrap();
    assert_eq!(
        code(&[
            "audit",
            &format!("{FX}/clean.csv"),
            "-o",
            &format!("{t}/s1"),
            "--strict"
        ]),
        0
    );
    assert_eq!(
        code(&[
            "audit",
            &format!("{FX}/dupes.csv"),
            "-o",
            &format!("{t}/s2"),
            "--strict"
        ]),
        1
    );
    std::fs::remove_dir_all(&tmp).unwrap();
}
