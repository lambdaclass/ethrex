//! Harnesses identify a runner by the first line of its `--version` output.

use std::process::Command;

fn version_output(bin: &str) -> String {
    let output = Command::new(bin)
        .arg("--version")
        .output()
        .expect("failed to run the binary");
    assert!(
        output.status.success(),
        "--version exited with {}",
        output.status
    );
    String::from_utf8(output.stdout).expect("stdout is not UTF-8")
}

#[test]
fn blocktest_version_names_client_and_tool() {
    assert_eq!(
        version_output(env!("CARGO_BIN_EXE_blocktest")),
        format!("ethrex-blocktest {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn enginetest_version_names_client_and_tool() {
    assert_eq!(
        version_output(env!("CARGO_BIN_EXE_enginetest")),
        format!("ethrex-enginetest {}\n", env!("CARGO_PKG_VERSION"))
    );
}
