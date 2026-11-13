//! `ef_tests-statev2 statetest` is ethrex's statetest runner; harnesses identify
//! it by the first line of its `--version` output.

use std::process::Command;

#[test]
fn statetest_version_names_client_and_tool() {
    let output = Command::new(env!("CARGO_BIN_EXE_ef_tests-statev2"))
        .args(["statetest", "--version"])
        .output()
        .expect("failed to run the binary");
    assert!(
        output.status.success(),
        "--version exited with {}",
        output.status
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is not UTF-8"),
        format!("ethrex-statetest {}\n", env!("CARGO_PKG_VERSION"))
    );
}
