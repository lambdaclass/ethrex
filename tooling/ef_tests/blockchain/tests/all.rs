use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_prover_lib::backend::BackendType;
use std::path::Path;

// Enable only one of `sp1` or `stateless` at a time.
#[cfg(all(feature = "sp1", feature = "stateless"))]
compile_error!("Only one of `sp1` and `stateless` can be enabled at a time.");

const TEST_FOLDER: &str = "vectors/";

// Base skips shared by all runs.
const SKIPPED_BASE: &[&str] = &[
    // Skip because they take too long to run, but they pass
    "static_Call50000_sha256",
    "CALLBlake2f_MaxRounds",
    "loopMul",
    // Skip because it tries to deserialize number > U256::MAX
    "ValueOverflowParis",
    // Skip because it's a "Create" Blob Transaction, which doesn't actually exist. It never reaches the EVM because we can't even parse it as an actual Transaction.
    "createBlobhashTx",
];

// Extra skips added only for prover backends.
#[cfg(feature = "sp1")]
const EXTRA_SKIPS: &[&str] = &[
    // I believe these tests fail because of how much stress they put into the zkVM, they probably cause an OOM though this should be checked
    "static_Call50000",
    "Return50000",
    "static_Call1MB1024Calldepth",
];
#[cfg(not(feature = "sp1"))]
const EXTRA_SKIPS: &[&str] = &[];

// Amsterdam EIP tests - skipped until each EIP is implemented
// See docs/eip.md for Amsterdam EIP implementation status
//
// STRUCTURE:
// - Section 1: Amsterdam-specific EIP directory skips (can be individually enabled)
// - Section 2: Legacy tests running on Amsterdam fork (skip until ALL Amsterdam EIPs done)
//
// HOW TO TEST AN INDIVIDUAL EIP (e.g., EIP-7708):
// 1. Comment out BOTH:
//    - The EIP's skip pattern (e.g., "eip7708_eth_transfer_logs")
//    - The "fork_Amsterdam" pattern in Section 2
// 2. Run with cargo test filter to only run that EIP's tests:
//      cargo test eip7708 --profile release-with-debug
// 3. Fix any test failures in your EIP implementation
// 4. IMPORTANT: Restore both skip patterns before committing
// 5. Update docs/eip.md to track progress
//
// WHY TWO SKIPS ARE NEEDED:
// - EIP patterns skip by test directory (e.g., "eip7708_eth_transfer_logs")
// - "fork_Amsterdam" skips by fork parameter in test name
// - Tests have BOTH in their full name, so both must be commented to run
//
// HOW TO FULLY ENABLE AMSTERDAM:
// 1. Implement ALL Amsterdam EIPs
// 2. Remove/comment ALL entries in this list (both sections)
// 3. Run: make test-levm
// 4. Fix any remaining failures
const SKIPPED_AMSTERDAM: &[&str] = &[
    // =========================================================================
    // SECTION 1: Amsterdam-specific EIP tests
    // These EIP-specific test directories have failing tests. Once an EIP is
    // fully implemented, remove it from this list to enable its tests.
    // =========================================================================
    //
    // EIP-7928: Block-Level Access Lists (SFI)
    // Directory ENABLED - 20 tests pass, 90 tests skipped below due to other
    // Amsterdam EIP dependencies (EIP-7778 gas accounting, EIP-7708 ETH transfer logs)
    // Passing tests: test_bal_invalid_*, test_bal_4788_empty_block,
    //                test_bal_empty_block_no_coinbase, test_bal_withdrawal_to_coinbase_empty_block,
    //                test_bal_withdrawal_empty_block, test_bal_zero_withdrawal,
    //                test_bal_withdrawal_largest_amount, test_bal_withdrawal_no_evm_execution,
    //                test_bal_withdrawal_to_nonexistent_account, test_bal_withdrawal_to_precompiles,
    //                test_bal_multiple_withdrawals_same_address
    "test_bal_2930_account_listed_but_untouched",
    "test_bal_2930_slot_listed_and_unlisted_reads",
    "test_bal_2930_slot_listed_and_unlisted_writes",
    "test_bal_2930_slot_listed_but_untouched",
    "test_bal_4788_query",
    "test_bal_4788_selfdestruct_to_beacon_root",
    "test_bal_4788_simple",
    "test_bal_7002_clean_sweep",
    "test_bal_7002_no_withdrawal_requests",
    "test_bal_7002_partial_sweep",
    "test_bal_7002_request_from_contract",
    "test_bal_7002_request_invalid",
    "test_bal_7702_delegated_storage_access",
    "test_bal_7702_delegated_via_call_opcode",
    "test_bal_7702_delegation_clear",
    "test_bal_7702_delegation_create",
    "test_bal_7702_delegation_update",
    "test_bal_7702_double_auth_reset",
    "test_bal_7702_double_auth_swap",
    "test_bal_7702_invalid_chain_id_authorization",
    "test_bal_7702_invalid_nonce_authorization",
    "test_bal_7702_null_address_delegation_no_code_change",
    "test_bal_aborted_account_access",
    "test_bal_aborted_storage_access",
    "test_bal_account_access_target",
    "test_bal_all_transaction_types",
    "test_bal_balance_and_oog",
    "test_bal_balance_changes",
    "test_bal_block_rewards",
    "test_bal_call_7702_delegation_and_oog",
    "test_bal_callcode_7702_delegation_and_oog",
    "test_bal_callcode_nested_value_transfer",
    "test_bal_callcode_no_delegation_and_oog_before_target_access",
    "test_bal_call_no_delegation_and_oog_before_target_access",
    "test_bal_call_no_delegation_oog_after_target_access",
    "test_bal_call_revert_insufficient_funds",
    "test_bal_call_with_value_in_static_context",
    "test_bal_code_changes",
    "test_bal_coinbase_zero_tip",
    "test_bal_consolidation_contract_cross_index",
    "test_bal_create2_collision",
    "test_bal_create_contract_init_revert",
    "test_bal_create_early_failure",
    "test_bal_create_oog_code_deposit",
    "test_bal_create_selfdestruct_to_self_with_call",
    "test_bal_create_transaction_empty_code",
    "test_bal_cross_block_ripemd160_state_leak",
    "test_bal_cross_tx_storage_revert_to_zero",
    "test_bal_delegatecall_7702_delegation_and_oog",
    "test_bal_delegatecall_no_delegation_and_oog_before_target_access",
    "test_bal_delegated_storage_reads",
    "test_bal_delegated_storage_writes",
    "test_bal_extcodecopy_and_oog",
    "test_bal_extcodesize_and_oog",
    "test_bal_fully_unmutated_account",
    "test_bal_lexicographic_address_ordering",
    "test_bal_multiple_balance_changes_same_account",
    "test_bal_multiple_storage_writes_same_slot",
    "test_bal_nested_delegatecall_storage_writes_net_zero",
    "test_bal_net_zero_balance_transfer",
    "test_bal_nonce_changes",
    "test_bal_nonexistent_account_access_read_only",
    "test_bal_nonexistent_account_access_value_transfer",
    "test_bal_nonexistent_value_transfer",
    "test_bal_noop_storage_write",
    "test_bal_noop_write_filtering",
    "test_bal_precompile_call",
    "test_bal_precompile_funded",
    "test_bal_pure_contract_call",
    "test_bal_selfdestruct_to_7702_delegation",
    "test_bal_self_transfer",
    "test_bal_sload_and_oog",
    "test_bal_sstore_and_oog",
    "test_bal_sstore_static_context",
    "test_bal_staticcall_7702_delegation_and_oog",
    "test_bal_staticcall_no_delegation_and_oog_before_target_access",
    "test_bal_storage_write_read_cross_frame",
    "test_bal_storage_write_read_same_frame",
    "test_bal_system_contract_noop_filtering",
    "test_bal_system_dequeue_consolidations_eip7251",
    "test_bal_transient_storage_not_tracked",
    "test_bal_withdrawal_and_new_contract",
    "test_bal_withdrawal_and_selfdestruct",
    "test_bal_withdrawal_and_state_access_same_account",
    "test_bal_withdrawal_and_transaction",
    "test_bal_withdrawal_and_value_transfer_same_address",
    "test_bal_withdrawal_contract_cross_index",
    "test_bal_withdrawal_to_7702_delegation",
    "test_bal_withdrawal_to_coinbase",
    "test_bal_zero_value_transfer",
    //
    // EIP-7708: ETH Transfers Emit a Log (CFI) - 38 failing tests
    // Requires LOG emission on ETH value transfers
    "eip7708_eth_transfer_logs",
    //
    // EIP-7778: Block Gas Accounting without Refunds (CFI) - 2 failing tests
    // Requires changes to gas refund calculations at block level
    "eip7778_block_gas_accounting_without_refunds",
    //
    // EIP-7843: SLOTNUM Opcode (CFI) - 2 failing tests
    // New opcode returning current slot number
    "eip7843_slotnum",
    //
    // EIP-8024: DUPN/SWAPN/EXCHANGE (CFI) - 41 failing tests
    // New stack manipulation opcodes (tests fail due to gas cost differences)
    "eip8024_dupn_swapn_exchange",
    //
    // =========================================================================
    // SECTION 2: Legacy tests running on Amsterdam fork - ENABLED
    // These pass (5971 tests) as they don't depend on Amsterdam-specific EIP
    // features beyond what's implemented.
    // =========================================================================
];

// Select backend
#[cfg(feature = "stateless")]
const BACKEND: Option<BackendType> = Some(BackendType::Exec);
#[cfg(feature = "sp1")]
const BACKEND: Option<BackendType> = Some(BackendType::SP1);
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
const BACKEND: Option<BackendType> = None;

fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    // Compose the final skip list
    let skips: Vec<&'static str> = SKIPPED_BASE
        .iter()
        .copied()
        .chain(EXTRA_SKIPS.iter().copied())
        .chain(SKIPPED_AMSTERDAM.iter().copied())
        .collect();

    parse_and_execute(path, Some(&skips), BACKEND)
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");
