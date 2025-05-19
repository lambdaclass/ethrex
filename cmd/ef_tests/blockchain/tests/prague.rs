use std::path::Path;

use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;

#[cfg(not(feature = "levm"))]
const SKIPPED_TESTS_REVM: [&str; 1] = [
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_non_empty_storage[fork_Prague-blockchain_test-zero_nonce]", // Skipped because REVM doesn't support this.
];

#[cfg(feature = "levm")]
const SKIPPED_TESTS_LEVM: [&str; 0] = [];

#[cfg(not(feature = "levm"))]
fn parse_and_execute_with_revm(path: &Path) -> datatest_stable::Result<()> {
    parse_and_execute(path, EvmEngine::REVM, Some(&SKIPPED_TESTS_REVM));
    Ok(())
}

#[cfg(feature = "levm")]
fn parse_and_execute_with_levm(path: &Path) -> datatest_stable::Result<()> {
    parse_and_execute(path, EvmEngine::LEVM, Some(&SKIPPED_TESTS_LEVM));
    Ok(())
}

// REVM execution
#[cfg(not(feature = "levm"))]
datatest_stable::harness!(
    parse_and_execute_with_revm,
    "vectors/prague/",
    r".*/.*\.json",
);

// LEVM execution
#[cfg(feature = "levm")]
datatest_stable::harness!(
    parse_and_execute_with_levm,
    "vectors/prague/",
    r".*/.*\.json",
);
