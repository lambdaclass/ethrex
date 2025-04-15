use std::path::Path;

use ef_tests_blockchain::test_runner::{parse_and_execute, parse_and_execute_with_filter};
use ethrex_vm::EvmEngine;

// TODO: enable these tests once the evm is updated.
#[cfg(not(feature = "levm"))]
const SKIPPED_TESTS_REVM: [&str; 1] = [
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_non_empty_storage[fork_Prague-blockchain_test-zero_nonce]",
];

// TODO: enable these tests once the they are fixed
#[cfg(feature = "levm")]
const SKIPPED_TESTS_LEVM: [&str; 11] = [
    "tests/prague/eip6110_deposits/test_deposits.py::test_deposit[fork_Prague-blockchain_test-single_deposit_from_contract_call_high_depth]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_max_depth_call_stack[fork_Prague-blockchain_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_pointer_contract_pointer_loop[fork_Prague-blockchain_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_gas_diff_pointer_vs_direct_call[fork_Prague-blockchain_test-access_list_to_AccessListTo.CONTRACT_ADDRESS-pointer_definition_PointerDefinition.SEPARATE-access_list_rule_AccessListCall.IN_NORMAL_TX_ONLY]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_gas_diff_pointer_vs_direct_call[fork_Prague-blockchain_test-access_list_to_AccessListTo.CONTRACT_ADDRESS-pointer_definition_PointerDefinition.SEPARATE-access_list_rule_AccessListCall.IN_BOTH_TX]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_gas_diff_pointer_vs_direct_call[fork_Prague-blockchain_test-access_list_to_AccessListTo.CONTRACT_ADDRESS-pointer_definition_PointerDefinition.SEPARATE-access_list_rule_AccessListCall.NONE]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_gas_diff_pointer_vs_direct_call[fork_Prague-blockchain_test-access_list_to_AccessListTo.CONTRACT_ADDRESS-pointer_definition_PointerDefinition.SEPARATE-access_list_rule_AccessListCall.IN_POINTER_TX_ONLY]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_gas_diff_pointer_vs_direct_call[fork_Prague-blockchain_test-access_list_to_AccessListTo.POINTER_ADDRESS-pointer_definition_PointerDefinition.SEPARATE-access_list_rule_AccessListCall.IN_NORMAL_TX_ONLY]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_gas_diff_pointer_vs_direct_call[fork_Prague-blockchain_test-access_list_to_AccessListTo.POINTER_ADDRESS-pointer_definition_PointerDefinition.SEPARATE-access_list_rule_AccessListCall.IN_POINTER_TX_ONLY]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_gas_diff_pointer_vs_direct_call[fork_Prague-blockchain_test-access_list_to_AccessListTo.POINTER_ADDRESS-pointer_definition_PointerDefinition.SEPARATE-access_list_rule_AccessListCall.NONE]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_gas_diff_pointer_vs_direct_call[fork_Prague-blockchain_test-access_list_to_AccessListTo.POINTER_ADDRESS-pointer_definition_PointerDefinition.SEPARATE-access_list_rule_AccessListCall.IN_BOTH_TX]"
];

#[cfg(not(feature = "levm"))]
fn parse_and_execute_with_revm(path: &Path) -> datatest_stable::Result<()> {
    // TODO: We may replace this function in favor of `parse_and_execute` once we no longer need to
    // filter more tests
    parse_and_execute_with_filter(path, EvmEngine::REVM, &SKIPPED_TESTS_REVM);
    Ok(())
}

#[cfg(feature = "levm")]
fn parse_and_execute_with_levm(path: &Path) -> datatest_stable::Result<()> {
    // TODO: We may replace this function in favor of `parse_and_execute` once we no longer need to
    // filter more tests
    parse_and_execute_with_filter(path, EvmEngine::LEVM, &SKIPPED_TESTS_LEVM);
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
