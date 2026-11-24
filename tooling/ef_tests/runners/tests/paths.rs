//! Every fixture path given runs, in one process with one set of results, and a
//! path the runner cannot use stops the run before any fixture runs.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Map, Value};

/// A fresh directory holding `fixtures/a.json` and `b.json`, copies of the
/// one-fixture file `source` whose fixture is renamed `a` and `b`.
fn fixtures(source: &str, test: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    if root.exists() {
        set_mode(&root.join("fixtures"), 0o755);
        set_mode(&root.join("fixtures/a.json"), 0o644);
        std::fs::remove_dir_all(&root).unwrap();
    }
    std::fs::create_dir_all(root.join("fixtures")).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(source);
    let fixtures: Map<String, Value> =
        serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
    let (_, fixture) = fixtures.into_iter().next().unwrap();
    for (name, path) in [("a", "fixtures/a.json"), ("b", "b.json")] {
        let file = Value::Object(Map::from_iter([(name.to_string(), fixture.clone())]));
        std::fs::write(root.join(path), file.to_string()).unwrap();
    }
    root
}

fn set_mode(path: &Path, mode: u32) {
    if path.exists() {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
}

fn run(bin: &str, args: &[&Path]) -> Output {
    Command::new(bin)
        .arg("--json")
        .args(args)
        .output()
        .expect("failed to run the binary")
}

/// The fixture names in the one JSON array on stdout.
fn names(output: &Output) -> Vec<String> {
    let results: Vec<Value> =
        serde_json::from_slice(&output.stdout).expect("stdout is one JSON array");
    results
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect()
}

fn runs_every_path(bin: &str, source: &str, test: &str) {
    let root = fixtures(source, test);
    let (a, b) = (root.join("fixtures/a.json"), root.join("b.json"));

    let output = run(bin, &[&a, &b]);
    assert_eq!(names(&output), ["a", "b"]);
    assert_eq!(output.status.code(), Some(0));

    // A directory and a file, and a file also found under that directory.
    assert_eq!(
        names(&run(bin, &[&root.join("fixtures"), &b, &a])),
        ["a", "b"]
    );
}

/// Exit 2, nothing on stdout, and stderr naming `bad`.
fn assert_stopped_before_running(output: &Output, bad: &Path, message: &str) {
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "a fixture ran");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("{message} {}", bad.display())),
        "{stderr}"
    );
}

fn rejects_a_missing_path(bin: &str, source: &str, test: &str) {
    let root = fixtures(source, test);
    let missing = root.join("missing");
    let output = run(bin, &[&root.join("b.json"), &missing]);
    assert_stopped_before_running(&output, &missing, "no such fixture file or directory:");
}

/// A file that cannot be read, named or found under a directory, and a directory
/// that cannot be read.
fn rejects_an_unreadable_path(bin: &str, source: &str, test: &str) {
    let root = fixtures(source, test);
    let (dir, a, b) = (
        root.join("fixtures"),
        root.join("fixtures/a.json"),
        root.join("b.json"),
    );
    set_mode(&a, 0o000);
    if std::fs::File::open(&a).is_ok() {
        // Permissions do not stop this user (root).
        eprintln!("skipped: {} is still readable", a.display());
        return;
    }
    assert_stopped_before_running(&run(bin, &[&b, &a]), &a, "cannot read");
    assert_stopped_before_running(&run(bin, &[&b, &dir]), &a, "cannot read");

    set_mode(&a, 0o644);
    set_mode(&dir, 0o000);
    assert_stopped_before_running(&run(bin, &[&b, &dir]), &dir, "cannot read directory");
    set_mode(&dir, 0o755);
}

#[test]
fn blocktest_runs_every_path() {
    runs_every_path(
        env!("CARGO_BIN_EXE_blocktest"),
        "blocktest_bal_report.json",
        "blocktest_runs_every_path",
    );
}

#[test]
fn enginetest_runs_every_path() {
    runs_every_path(
        env!("CARGO_BIN_EXE_enginetest"),
        "enginetest_bal_report.json",
        "enginetest_runs_every_path",
    );
}

#[test]
fn blocktest_rejects_a_missing_path() {
    rejects_a_missing_path(
        env!("CARGO_BIN_EXE_blocktest"),
        "blocktest_bal_report.json",
        "blocktest_missing",
    );
}

#[test]
fn enginetest_rejects_a_missing_path() {
    rejects_a_missing_path(
        env!("CARGO_BIN_EXE_enginetest"),
        "enginetest_bal_report.json",
        "enginetest_missing",
    );
}

#[test]
fn blocktest_rejects_an_unreadable_path() {
    rejects_an_unreadable_path(
        env!("CARGO_BIN_EXE_blocktest"),
        "blocktest_bal_report.json",
        "blocktest_unreadable",
    );
}

#[test]
fn enginetest_rejects_an_unreadable_path() {
    rejects_an_unreadable_path(
        env!("CARGO_BIN_EXE_enginetest"),
        "enginetest_bal_report.json",
        "enginetest_unreadable",
    );
}
