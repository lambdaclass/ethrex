//! Integration tests for the `run` subcommand.
//!
//! These tests invoke the `ethrex-evm` binary via `std::process::Command` and
//! verify that the `run` subcommand produces EIP-3155 streaming output
//! compatible with geth's `evm run --json` command.

use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

/// Returns the path to the compiled `ethrex-evm` binary.
fn binary() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("could not get test binary path")
        .parent()
        .expect("no parent dir")
        .to_path_buf();
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("ethrex-evm");
    path
}

/// Parses a JSON line and returns the value.
fn parse_json(line: &str) -> serde_json::Value {
    serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("expected valid JSON line, got: {line:?}\nerror: {e}"))
}

/// Test 1: PUSH1 1, PUSH1 1, ADD, STOP — positional bytecode argument.
///
/// Bytecode: 0x6001600101
/// Expected: 4 opcode lines + 1 summary on stderr; exit 0.
/// Expected opcode sequence: PUSH1 (pc=0), PUSH1 (pc=2), ADD (pc=4), STOP (pc=5).
#[test]
fn test1_push_add_stop_positional() {
    let bin = binary();
    let out = Command::new(&bin)
        .args(["run", "--json", "0x6001600101"])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin:?}: {e}"));

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.is_empty()).collect();

    // 4 opcode lines + 1 summary
    assert_eq!(
        lines.len(),
        5,
        "expected 5 lines (4 opcodes + summary), got {}: {stderr}",
        lines.len()
    );

    // Opcode lines: validate pc, op byte, opName, gasCost
    let expected = [
        (0u64, 96u64, "PUSH1", "0x3"),
        (2u64, 96u64, "PUSH1", "0x3"),
        (4u64, 1u64, "ADD", "0x3"),
        (5u64, 0u64, "STOP", "0x0"),
    ];

    for (i, (pc, op, op_name, gas_cost)) in expected.iter().enumerate() {
        let v = parse_json(lines[i]);
        assert_eq!(v["pc"].as_u64().unwrap(), *pc, "opcode {i}: pc mismatch");
        assert_eq!(v["op"].as_u64().unwrap(), *op, "opcode {i}: op mismatch");
        assert_eq!(
            v["opName"].as_str().unwrap(),
            *op_name,
            "opcode {i}: opName mismatch"
        );
        assert_eq!(
            v["gasCost"].as_str().unwrap(),
            *gas_cost,
            "opcode {i}: gasCost mismatch"
        );
    }

    // Summary line
    let summary = parse_json(lines[4]);
    assert_eq!(
        summary["output"].as_str().unwrap(),
        "",
        "summary output should be empty"
    );
    assert_eq!(
        summary["gasUsed"].as_str().unwrap(),
        "0x9",
        "summary gasUsed mismatch"
    );
    assert!(
        summary.get("error").is_none(),
        "summary should have no error field on success"
    );
}

/// Test 2: PUSH1 1, PUSH1 0, REVERT — checks revert output.
///
/// Bytecode: 0x60016000fd
/// REVERT(offset=0, size=1) reads 1 byte from uninitialized memory (0x00).
/// Expected: 3 opcode lines + 1 summary with error; exit 0 (EVM revert is not a process error).
#[test]
fn test2_revert_bytecode() {
    let bin = binary();
    let out = Command::new(&bin)
        .args(["run", "--json", "0x60016000fd"])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin:?}: {e}"));

    // EVM revert does not cause a non-zero process exit.
    assert!(
        out.status.success(),
        "expected exit 0 on EVM revert, got {:?}",
        out.status
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.is_empty()).collect();

    // The last line is always the summary; opcode lines come before it.
    let summary_line = lines.last().expect("stderr was empty");
    let summary = parse_json(summary_line);

    // Summary must contain the error field.
    assert_eq!(
        summary["error"].as_str().unwrap(),
        "execution reverted",
        "summary error mismatch"
    );

    // There should be at least 2 opcode lines (PUSH1, PUSH1) + 1 summary.
    assert!(
        lines.len() >= 3,
        "expected at least 3 lines, got {}: {stderr}",
        lines.len()
    );

    // Verify the opcode lines with "opName" fields.
    let opcode_lines: Vec<&&str> = lines.iter().filter(|l| l.contains("\"opName\"")).collect();
    assert!(
        !opcode_lines.is_empty(),
        "expected at least one opcode line in stderr"
    );

    // The last opcode emitted must be REVERT (op byte 253 = 0xfd).
    let last_op = parse_json(opcode_lines.last().unwrap());
    assert_eq!(
        last_op["opName"].as_str().unwrap(),
        "REVERT",
        "last opcode should be REVERT"
    );
}

/// Test 3: `--codefile -` reads bytecode from stdin.
///
/// Same bytecode as Test 1; same expected output.
#[test]
fn test3_codefile_stdin() {
    let bin = binary();
    let mut child = Command::new(&bin)
        .args(["run", "--json", "--codefile", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {bin:?}: {e}"));

    // Write bytecode to stdin then close it.
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"0x6001600101")
        .expect("write stdin");

    let out = child.wait_with_output().expect("wait_with_output");

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.is_empty()).collect();

    assert_eq!(
        lines.len(),
        5,
        "expected 5 lines, got {}: {stderr}",
        lines.len()
    );

    let summary = parse_json(lines[4]);
    assert_eq!(summary["output"].as_str().unwrap(), "");
    assert_eq!(summary["gasUsed"].as_str().unwrap(), "0x9");
}

/// Test 4: `--statdump` prints gas/time/allocation fields to stderr.
///
/// Bytecode: 0x00 (STOP).
#[test]
fn test4_statdump() {
    let bin = binary();
    let out = Command::new(&bin)
        .args(["run", "--statdump", "0x00"])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin:?}: {e}"));

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("EVM gas used:"),
        "expected 'EVM gas used:' in statdump: {stderr}"
    );
    assert!(
        stderr.contains("execution time:"),
        "expected 'execution time:' in statdump: {stderr}"
    );
    assert!(
        stderr.contains("allocations:"),
        "expected 'allocations:' in statdump: {stderr}"
    );
    assert!(
        stderr.contains("allocated bytes:"),
        "expected 'allocated bytes:' in statdump: {stderr}"
    );
}

/// Test 5: no bytecode source exits with code 1.
#[test]
fn test5_no_bytecode_exits_1() {
    let bin = binary();
    let out = Command::new(&bin)
        .args(["run"])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin:?}: {e}"));

    assert_ne!(
        out.status.code().unwrap_or(0),
        0,
        "expected non-zero exit when no bytecode provided"
    );
}

/// Test 6: byte-exact comparison against captured geth output for PUSH1+PUSH1+ADD+STOP.
///
/// The golden file was captured from geth v1.17.3-stable (see GETH_VERSION.txt).
///
/// If this test fails, the diff is printed line-by-line so you can see which
/// fields diverge. On a real divergence: either fix the binary or, if the
/// fixture is genuinely stale (e.g. geth changed an unrelated field), update
/// `run_push_add.geth.jsonl` and bump `GETH_VERSION.txt`.
#[test]
fn test6_golden_push_add() {
    let bin = binary();
    let out = Command::new(&bin)
        .args(["run", "--json", "0x6001600101"])
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin:?}: {e}"));

    assert!(out.status.success(), "binary exited non-zero");

    let actual_stderr = String::from_utf8_lossy(&out.stderr);
    // Normalize trailing newline to allow minor whitespace differences.
    let actual = actual_stderr.trim_end_matches('\n').to_owned();

    let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("run_push_add.geth.jsonl");
    let golden_raw = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("could not read golden file {golden_path:?}: {e}"));
    let golden = golden_raw.trim_end_matches('\n');

    if actual != golden {
        // Print diff information to assist debugging.
        eprintln!("=== GOLDEN FILE DIVERGENCE ===");
        eprintln!("--- expected (geth) ---");
        for (i, line) in golden.lines().enumerate() {
            eprintln!("{i}: {line}");
        }
        eprintln!("--- actual (ethrex-evm) ---");
        for (i, line) in actual.lines().enumerate() {
            eprintln!("{i}: {line}");
        }
        eprintln!("=== END DIVERGENCE ===");

        // Compare line by line and identify specific field differences.
        let golden_lines: Vec<&str> = golden.lines().collect();
        let actual_lines: Vec<&str> = actual.lines().collect();
        let min_len = golden_lines.len().min(actual_lines.len());
        for i in 0..min_len {
            if golden_lines[i] != actual_lines[i] {
                let gv = serde_json::from_str::<serde_json::Value>(golden_lines[i]);
                let av = serde_json::from_str::<serde_json::Value>(actual_lines[i]);
                if let (Ok(gv), Ok(av)) = (gv, av) {
                    eprintln!("Line {i} differences:");
                    if let (Some(gobj), Some(aobj)) = (gv.as_object(), av.as_object()) {
                        for key in gobj.keys() {
                            if gobj.get(key) != aobj.get(key) {
                                eprintln!(
                                    "  field {key:?}: expected {:?}, got {:?}",
                                    gobj[key],
                                    aobj.get(key)
                                );
                            }
                        }
                    }
                }
            }
        }

        panic!(
            "Output does not match golden file {golden_path:?}.\n\
             If the difference is only in gas values or other expected divergences,\n\
             update the fixture or add #[ignore] to this test with a comment."
        );
    }
}
