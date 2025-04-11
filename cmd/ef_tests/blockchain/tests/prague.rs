use std::path::Path;

use ef_tests_blockchain::test_runner::{parse_and_execute, parse_and_execute_with_filter};
use ethrex_vm::EvmEngine;

// TODO: enable these tests once the evm is updated.
const SKIPPED_TEST: [&str; 1] = [
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_non_empty_storage[fork_Prague-blockchain_test-zero_nonce]",
];

#[allow(dead_code)]
fn parse_and_execute_with_revm(path: &Path) -> datatest_stable::Result<()> {
    // TODO: We may replace this function in favor of `parse_and_execute` once we no longer need to
    // filter more tests
    parse_and_execute_with_filter(path, EvmEngine::REVM, &SKIPPED_TEST);
    Ok(())
}

#[allow(dead_code)]
fn parse_and_execute_with_levm(path: &Path) -> datatest_stable::Result<()> {
    parse_and_execute(path, EvmEngine::LEVM);
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
    // parse_and_execute_with_levm,
    // "vectors/prague/",
    // r"eip2537_bls_12_381_precompiles/.*/.*\.json",
    parse_and_execute_with_levm,
    "vectors/prague/",
    r"eip2935_historical_block_hashes_from_state/.*/.*\.json",
    parse_and_execute_with_levm,
    "vectors/prague/",
    r"eip2935_historical_block_hashes_from_state/.*/.*\.json",
    // parse_and_execute_with_levm,
    // "vectors/prague/",
    // r"eip6110_deposits/.*/.*\.json",
    // parse_and_execute_with_levm,
    // "vectors/prague/",
    // r"eip7002_el_triggerable_withdrawals/.*/.*\.json",
    // parse_and_execute_with_levm,
    // "vectors/prague/",
    // r"eip7251_consolidations/.*/.*\.json",
    // parse_and_execute_with_levm,
    // "vectors/prague/",
    // r"eip7623_increase_calldata_cost/.*/.*\.json",
    parse_and_execute_with_levm,
    "vectors/prague/",
    r"eip7685_general_purpose_el_requests/.*/.*\.json",
    // parse_and_execute_with_levm,
    // "vectors/prague/",
    // r"eip7702_set_code_tx/.*/.*\.json",
);
