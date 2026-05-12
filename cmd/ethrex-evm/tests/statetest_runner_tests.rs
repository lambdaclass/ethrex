//! Integration tests for the `statetest` subcommand.
//!
//! These tests invoke the `ethrex-evm` binary via `std::process::Command` and
//! verify the streaming output conforms to the EIP-3155 / goevmlab shape.

use std::path::PathBuf;
use std::process::Command;

/// Returns the path to the compiled `ethrex-evm` binary.
fn binary() -> PathBuf {
    // Cargo puts binaries in target/debug or target/release. Use the env var
    // that `cargo test` sets when running integration tests.
    let mut path = std::env::current_exe()
        .expect("could not get test binary path")
        .parent()
        .expect("no parent dir")
        .to_path_buf();
    // Walk up through `deps/` if needed.
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("ethrex-evm");
    path
}

/// Path to the simple fixture JSON.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("statetest_simple.json")
}

/// Asserts that every line in `lines` (except the last) parses as valid JSON.
fn assert_lines_are_json(lines: &[&str]) {
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(line);
        assert!(
            parsed.is_ok(),
            "expected valid JSON line, got: {line:?}\nerror: {}",
            parsed.unwrap_err()
        );
    }
}

/// Extracts the `{"stateRoot": "0x..."}` from the last non-empty line of stderr.
fn extract_state_root(stderr: &str) -> String {
    let last = stderr
        .lines()
        .filter(|l| !l.trim().is_empty())
        .next_back()
        .expect("stderr was empty");

    let v: serde_json::Value =
        serde_json::from_str(last).unwrap_or_else(|_| panic!("last line not JSON: {last:?}"));

    v.get("stateRoot")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("last line has no stateRoot: {last:?}"))
        .to_owned()
}

/// Test 5a: positional path mode, with `--trace` enabled.
///
/// Acceptance criteria:
/// - exit code 0
/// - all stderr lines parse as valid JSON
/// - last line contains `"stateRoot"` matching the fixture's `post.hash`
#[test]
fn test_5a_positional_path_trace() {
    let bin = binary();
    let fixture = fixture_path();

    // Pinned state root matches the fixture's `post.Prague[0].hash`.
    const EXPECTED_ROOT: &str =
        "0xd985fdb5e9d3040e79c17f7245be8498c0f77d3de9778bbc8fd3906108505daf";

    let out = Command::new(&bin)
        .args([
            "statetest",
            "--trace",
            "--trace.format=json",
            "--trace.nomemory=true",
            "--trace.noreturndata=true",
            fixture.to_str().expect("fixture path to str"),
        ])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin:?}: {e}"));

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr.lines().collect();

    assert!(!lines.is_empty(), "stderr was empty");

    assert_lines_are_json(&lines);

    let root = extract_state_root(&stderr);
    assert_eq!(
        root, EXPECTED_ROOT,
        "state root mismatch: expected {EXPECTED_ROOT}, got {root}"
    );

    // The fixture's tx is rejected at intrinsic-gas validation; the summary
    // line must carry the geth-compatible string so goevmlab byte-diff matches.
    let summary = lines
        .iter()
        .rfind(|l| l.contains("\"gasUsed\""))
        .expect("no summary line found in stderr");
    assert!(
        summary.contains("\"error\":\"intrinsic gas too low\""),
        "summary error must use the geth-compatible string, got: {summary}"
    );
}

/// Test 5b: stdin batch mode — one path fed via stdin, no positional args.
///
/// Same acceptance criteria as 5a.
#[test]
fn test_5b_stdin_batch_mode() {
    let bin = binary();
    let fixture = fixture_path();

    const EXPECTED_ROOT: &str =
        "0xd985fdb5e9d3040e79c17f7245be8498c0f77d3de9778bbc8fd3906108505daf";

    let fixture_str = fixture.to_str().expect("fixture path to str").to_owned();

    let out = Command::new(&bin)
        .args([
            "statetest",
            "--trace",
            "--trace.format=json",
            "--trace.nomemory=true",
            "--trace.noreturndata=true",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            if let Some(stdin) = child.stdin.take() {
                let mut stdin = stdin;
                // Send one path followed by a newline. goevmlab sends an empty
                // line to terminate batch mode; we close stdin instead.
                writeln!(stdin, "{fixture_str}")?;
            }
            child.wait_with_output()
        })
        .unwrap_or_else(|e| panic!("failed to run {bin:?}: {e}"));

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr.lines().collect();

    assert!(!lines.is_empty(), "stderr was empty in batch mode");

    assert_lines_are_json(&lines);

    let root = extract_state_root(&stderr);
    assert_eq!(
        root, EXPECTED_ROOT,
        "state root mismatch in batch mode: expected {EXPECTED_ROOT}, got {root}"
    );
}

/// Test: unsupported trace format exits with code 1.
#[test]
fn test_unsupported_trace_format_exits_1() {
    let bin = binary();
    let fixture = fixture_path();

    let out = Command::new(&bin)
        .args([
            "statetest",
            "--trace",
            "--trace.format=xyz",
            fixture.to_str().expect("fixture path to str"),
        ])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin:?}: {e}"));

    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit code 1 for unsupported format, got {:?}",
        out.status
    );
}

/// Test: `run` subcommand stub returns a non-zero exit and the expected message.
#[test]
fn test_run_stub_exits_nonzero() {
    let bin = binary();

    let out = Command::new(&bin)
        .args(["run"])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin:?}: {e}"));

    assert!(
        !out.status.success(),
        "run stub should fail, but exited with {:?}",
        out.status
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Phase 5"),
        "expected 'Phase 5' in run stub error message: {stderr}"
    );
}
