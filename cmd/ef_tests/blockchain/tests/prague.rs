use std::path::Path;

use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;

// TODO: enable these tests once the evm is updated.
#[cfg(not(feature = "levm"))]
const SKIPPED_TESTS_REVM: [&str; 5] = [
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_non_empty_storage[fork_Prague-blockchain_test-zero_nonce]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reaches_gas_limit-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_throws-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reaches_gas_limit-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_throws-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
];

#[cfg(feature = "levm")]
const SKIPPED_TESTS_LEVM: [&str; 4] = [
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reaches_gas_limit-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_throws-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reaches_gas_limit-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_throws-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
];

// NOTE: These 3 tests fail on LEVM with a stack overflow if we do not increase the stack size by using RUST_MIN_STACK=11000000
//"tests/prague/eip6110_deposits/test_deposits.py::test_deposit[fork_Prague-blockchain_test-single_deposit_from_contract_call_high_depth]",
//"tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_max_depth_call_stack[fork_Prague-blockchain_test]",
//"tests/prague/eip7702_set_code_tx/test_set_code_txs_2.py::test_pointer_contract_pointer_loop[fork_Prague-blockchain_test]",

// NOTE: The following test fails because of an OutOfGas error. This happens because it tests a system call to a contract that has a
// code with a cost of +29 million gas that when is being summed to the 21k base intrinsic gas it goes over the 30 million limit.
// "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reaches_gas_limit-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",

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
    // r".*/.*\.json",
    // "eip7702_set_code_tx/set_code_txs/set_code_to_precompile_not_enough_gas_for_precompile_execution.json",
    "eip7702_set_code_tx/set_code_txs_2/pointer_to_precompile.json",
);
