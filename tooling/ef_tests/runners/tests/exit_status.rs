//! The exit status: 0 when every fixture ran and passed, 1 when every fixture
//! ran and at least one failed, and 2 when the runner could not run all of its
//! input. A missing path also exits 2; see `paths.rs`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Map, Value};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// A fresh directory for `test`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A copy of the fixture file `name` whose fixtures all fail: each expects a last
/// block hash the chain never has.
fn failing_copy(name: &str, test: &str) -> PathBuf {
    let mut fixtures: Map<String, Value> =
        serde_json::from_slice(&std::fs::read(fixture(name)).unwrap()).unwrap();
    for fixture in fixtures.values_mut() {
        fixture["lastblockhash"] = format!("0x{}", "11".repeat(32)).into();
    }
    let path = scratch(test).join(name);
    std::fs::write(&path, serde_json::to_vec(&fixtures).unwrap()).unwrap();
    path
}

fn run(bin: &str, paths: &[&Path]) -> Output {
    Command::new(bin)
        .arg("--json")
        .args(paths)
        .output()
        .expect("failed to run the binary")
}

/// `pass` of each result in the one JSON array on stdout, failures first.
fn verdicts(output: &Output) -> Vec<bool> {
    let results: Vec<Value> =
        serde_json::from_slice(&output.stdout).expect("stdout is one JSON array");
    let mut verdicts: Vec<bool> = results
        .iter()
        .map(|r| r["pass"].as_bool().unwrap())
        .collect();
    verdicts.sort();
    verdicts
}

fn exits_0_when_every_fixture_passes(bin: &str, name: &str) {
    let output = run(bin, &[&fixture(name)]);
    assert_eq!(verdicts(&output), [true]);
    assert_eq!(output.status.code(), Some(0));
}

fn exits_1_when_a_fixture_fails(bin: &str, name: &str, test: &str) {
    let output = run(bin, &[&fixture(name), &failing_copy(name, test)]);
    assert_eq!(verdicts(&output), [false, true]);
    assert_eq!(output.status.code(), Some(1));
}

fn exits_2_on_a_file_that_does_not_parse(bin: &str, name: &str, test: &str) {
    let unparseable = scratch(test).join("unparseable.json");
    std::fs::write(&unparseable, "not json").unwrap();
    let output = run(bin, &[&fixture(name), &unparseable]);
    // The other file still runs and is reported; the unparseable one is only on
    // stderr.
    assert_eq!(verdicts(&output), [true]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("cannot run {}", unparseable.display())),
        "{stderr}"
    );
}

#[test]
fn blocktest_exits_0_when_every_fixture_passes() {
    exits_0_when_every_fixture_passes(env!("CARGO_BIN_EXE_blocktest"), "blocktest_bal_report.json");
}

#[test]
fn blocktest_exits_1_when_a_fixture_fails() {
    exits_1_when_a_fixture_fails(
        env!("CARGO_BIN_EXE_blocktest"),
        "blocktest_bal_report.json",
        "blocktest_exits_1",
    );
}

#[test]
fn blocktest_exits_2_on_a_file_that_does_not_parse() {
    exits_2_on_a_file_that_does_not_parse(
        env!("CARGO_BIN_EXE_blocktest"),
        "blocktest_bal_report.json",
        "blocktest_exits_2",
    );
}

#[test]
fn enginetest_exits_0_when_every_fixture_passes() {
    exits_0_when_every_fixture_passes(
        env!("CARGO_BIN_EXE_enginetest"),
        "enginetest_bal_report.json",
    );
}

#[test]
fn enginetest_exits_1_when_a_fixture_fails() {
    exits_1_when_a_fixture_fails(
        env!("CARGO_BIN_EXE_enginetest"),
        "enginetest_bal_report.json",
        "enginetest_exits_1",
    );
}

#[test]
fn enginetest_exits_2_on_a_file_that_does_not_parse() {
    exits_2_on_a_file_that_does_not_parse(
        env!("CARGO_BIN_EXE_enginetest"),
        "enginetest_bal_report.json",
        "enginetest_exits_2",
    );
}
