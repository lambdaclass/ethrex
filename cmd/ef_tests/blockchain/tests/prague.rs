use std::path::Path;

use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;

// TODO: enable these tests once the evm is updated.
#[cfg(not(feature = "levm"))]
const SKIPPED_TESTS_REVM: [&str; 1] = [
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_non_empty_storage[fork_Prague-blockchain_test-zero_nonce]",
];

// NOTE: These 3 tests fail on LEVM with a stack overflow if we do not increase the stack size by using RUST_MIN_STACK=11000000
//"tests/prague/eip6110_deposits/test_deposits.py::test_deposit[fork_Prague-blockchain_test-single_deposit_from_contract_call_high_depth]",
//"tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_max_depth_call_stack[fork_Prague-blockchain_test]",
//"tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_pointer_contract_pointer_loop[fork_Prague-blockchain_test]",

#[cfg(not(feature = "levm"))]
fn parse_and_execute_with_revm(path: &Path) -> datatest_stable::Result<()> {
    parse_and_execute(path, EvmEngine::REVM, Some(&SKIPPED_TESTS_REVM));
    Ok(())
}

#[cfg(feature = "levm")]
fn parse_and_execute_with_levm(path: &Path) -> datatest_stable::Result<()> {
    parse_and_execute(path, EvmEngine::LEVM, None);
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
