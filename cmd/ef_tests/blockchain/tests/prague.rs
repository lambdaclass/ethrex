use std::path::Path;

use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;

// TODO: enable these tests once the evm is updated.
#[cfg(not(feature = "levm"))]
const SKIPPED_TESTS_REVM: [&str; 13] = [
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_non_empty_storage[fork_Prague-blockchain_test-zero_nonce]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_contract_deployment.py::test_system_contract_deployment[fork_CancunToPragueAtTime15k-blockchain_test-deploy_after_fork-nonzero_balance]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_contract_deployment.py::test_system_contract_deployment[fork_CancunToPragueAtTime15k-blockchain_test-deploy_after_fork-zero_balance]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_out_of_gas-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reverts-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reaches_gas_limit-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_throws-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7251_consolidations/test_contract_deployment.py::test_system_contract_deployment[fork_CancunToPragueAtTime15k-blockchain_test-deploy_after_fork-zero_balance]",
    "tests/prague/eip7251_consolidations/test_contract_deployment.py::test_system_contract_deployment[fork_CancunToPragueAtTime15k-blockchain_test-deploy_after_fork-nonzero_balance]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reaches_gas_limit-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_out_of_gas-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_throws-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reverts-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
];

#[cfg(feature = "levm")]
const SKIPPED_TESTS_LEVM: [&str; 42] = [
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_out_of_gas-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reverts-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reaches_gas_limit-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7002_el_triggerable_withdrawals/test_modified_withdrawal_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_throws-system_contract_0x00000961ef480eb55e80d19ad83579a64c007002]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reaches_gas_limit-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_out_of_gas-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_throws-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
    "tests/prague/eip7251_consolidations/test_modified_consolidation_contract.py::test_system_contract_errors[fork_Prague-blockchain_test-system_contract_reverts-system_contract_0x0000bbddc7ce488642fb579f8b00f3a590007251]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000006-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000011-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000009-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x000000000000000000000000000000000000000b-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x000000000000000000000000000000000000000e-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000004-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000009-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x000000000000000000000000000000000000000c-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x000000000000000000000000000000000000000c-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x000000000000000000000000000000000000000a-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000008-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000001-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000006-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x000000000000000000000000000000000000000f-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000010-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x000000000000000000000000000000000000000f-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000011-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000002-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x000000000000000000000000000000000000000b-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x000000000000000000000000000000000000000e-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000005-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x000000000000000000000000000000000000000d-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000001-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000004-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000007-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000003-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000005-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000010-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000007-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x000000000000000000000000000000000000000d-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000008-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x0000000000000000000000000000000000000002-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile_not_enough_gas_for_precompile_execution[fork_Prague-precompile_0x000000000000000000000000000000000000000a-blockchain_test_from_state_test]",
    "tests/prague/eip7702_set_code_tx/test_set_code_txs.py::test_set_code_to_precompile[fork_Prague-precompile_0x0000000000000000000000000000000000000003-call_opcode_CALL-evm_code_type_LEGACY-blockchain_test_from_state_test]",
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
