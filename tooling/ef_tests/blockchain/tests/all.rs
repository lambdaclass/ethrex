use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_prover::backend::BackendType;
use std::path::Path;

// Enable only one of `sp1` or `stateless` at a time.
#[cfg(all(feature = "sp1", feature = "stateless"))]
compile_error!("Only one of `sp1` and `stateless` can be enabled at a time.");

// test-levm / test-sp1 read snobal-devnet-6 + legacy from `vectors/`.
// test-stateless reads zkevm@v0.3.3 (the only bundle that ships executionWitness)
// from a separate `vectors_zkevm/` so its older bal@v5.6.1 base never overlays
// the snobal fixtures used by the other suites.
#[cfg(feature = "stateless")]
const TEST_FOLDER: &str = "vectors_zkevm/";
#[cfg(not(feature = "stateless"))]
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
    // EIP-8025 optional-proofs fixtures filled against bal@v5.6.1 (devnets/bal/3),
    // which predates EELS PR #2711 "immutable intrinsic_state_gas for EIP-7702".
    // Expected gas assumes the auth refund still deducts from block-accounted state
    // gas; our devnet-4 (bal@v5.7.0) impl correctly keeps intrinsic_state_gas
    // immutable and routes the refund to the reservoir only. Re-enable once the
    // zkevm@v0.4.x release ships fixtures regenerated against devnet-4.
    "witness_codes_redelegation_old_marker_included_new_marker_excluded",
    "witness_codes_reset_delegation",
    "witness_codes_reverted_transaction",
    "witness_codes_failed_create_includes_factory",
    "witness_codes_reverted_create_same_hash_then_read",
    "witness_codes_create_then_selfdestruct_same_tx",
    // Additional EIP-8025 optional-proofs fixtures whose expected gas magnitudes
    // disagree with bal-devnet-7 (bal@v7.0.0) state-gas accounting. Same root
    // cause as the block above: zkevm@v0.3.3 bundle is pinned at an older bal
    // spec (storage_set / new_account / cpsb constants pre-recalibration plus
    // earlier refund-channel semantics) and the broader fork.py changes from
    // EELS PRs #2815/#2816/#2823/#2827/#2828. Re-enable once the zkevm bundle
    // is regenerated against bal-7.
    "witness_codes_delegation_set_in_same_block",
    "witness_codes_auth_nonce_mismatch",
    "witness_codes_dedup_identical_bytecode",
    "witness_codes_create2_excludes_new_bytecode",
    "witness_codes_reverted_inner_call",
    "witness_codes_create_same_hash_then_read",
    "witness_codes_create_then_call_same_block",
    "witness_codes_create_then_call_same_tx",
    "witness_codes_failed_create_after_initcode_read",
    "witness_codes_initcode_calls_existing_contract",
    "witness_excludes_bytecode_created_in_same_block",
    "witness_keeps_prestate_code_read_even_if_later_created_with_same_hash",
    "witness_codes_selfdestruct_in_initcode",
    "witness_codes_selfdestruct_beneficiary_no_code",
    "witness_state_delete_with_new_dirty_sibling_omits_post_state_node",
    "witness_state_block_diff_delete_insert_before_delete_order",
    "witness_state_delete_then_insert_uses_insert_before_delete_order",
    "witness_state_sstore_into_empty_storage_omits_post_state_nodes",
    "witness_state_sstore_new_slot_omits_post_state_nodes",
    "validation_state_missing_absent_slot_proof_leaf_node",
    "validation_state_missing_storage_proof_node",
    // ---------------------------------------------------------------
    // bal-devnet-6 known-failing fixtures (Amsterdam fork only).
    //
    // All entries below are anchored with `[fork_Amsterdam` so the legacy
    // Prague/Osaka variants of the same EELS test functions still run (those
    // pass). Each bucket maps to one EIP / fixture family; the underlying
    // root cause is that snobal-devnet-6 fixtures expect the
    // bal-devnet-6 spec semantics, but our impl currently runs ahead of
    // that on the EIP-7702 `set_delegation` state-gas accounting (the
    // bal-devnet-7-prep SELFDESTRUCT-style refund subtraction was re-applied
    // in 0976534cf0). To be re-enabled once we either:
    //   (a) bump fixtures to a snobal-devnet-7 release that locks in the
    //       new accounting, or
    //   (b) revert the bal-devnet-7-prep subtraction for bal-devnet-6
    //       compatibility.
    // Tracking via PR #6574.
    // ---------------------------------------------------------------

    // EIP-7702 — for_amsterdam/prague/eip7702_set_code_tx/set_code_txs/*.
    // Prague set-code transaction tests re-run under Amsterdam; expected gas
    // accounting differs from current set_delegation refund handling.
    "test_delegation_clearing[fork_Amsterdam",
    "test_delegation_clearing_and_set[fork_Amsterdam",
    "test_delegation_clearing_failing_tx[fork_Amsterdam",
    "test_delegation_clearing_tx_to[fork_Amsterdam",
    "test_eoa_tx_after_set_code[fork_Amsterdam",
    "test_ext_code_on_chain_delegating_set_code[fork_Amsterdam",
    "test_ext_code_on_self_delegating_set_code[fork_Amsterdam",
    "test_ext_code_on_self_set_code[fork_Amsterdam",
    "test_ext_code_on_set_code[fork_Amsterdam",
    "test_many_delegations[fork_Amsterdam",
    "test_nonce_overflow_after_first_authorization[fork_Amsterdam",
    "test_nonce_validity[fork_Amsterdam",
    "test_reset_code[fork_Amsterdam",
    "test_self_code_on_set_code[fork_Amsterdam",
    "test_self_sponsored_set_code[fork_Amsterdam",
    "test_set_code_multiple_valid_authorization_tuples_same_signer_increasing_nonce[fork_Amsterdam",
    "test_set_code_multiple_valid_authorization_tuples_same_signer_increasing_nonce_self_sponsored[fork_Amsterdam",
    "test_set_code_to_log[fork_Amsterdam",
    "test_set_code_to_non_empty_storage_non_zero_nonce[fork_Amsterdam",
    "test_set_code_to_self_destruct[fork_Amsterdam",
    "test_set_code_to_self_destructing_account_deployed_in_same_tx[fork_Amsterdam",
    "test_set_code_to_sstore[fork_Amsterdam",
    "test_set_code_to_sstore_then_sload[fork_Amsterdam",
    "test_set_code_to_system_contract[fork_Amsterdam",
    // EIP-7702 — for_amsterdam/prague/eip7702_set_code_tx/set_code_txs_2/*.
    // 7702-pointer interaction tests; fail for the same `set_delegation`
    // accounting reason as the set_code_txs bucket above.
    "test_call_pointer_to_created_from_create_after_oog_call_again[fork_Amsterdam",
    "test_call_to_precompile_in_pointer_context[fork_Amsterdam",
    "test_contract_storage_to_pointer_with_storage[fork_Amsterdam",
    "test_delegation_replacement_call_previous_contract[fork_Amsterdam",
    "test_double_auth[fork_Amsterdam",
    "test_pointer_measurements[fork_Amsterdam",
    "test_pointer_normal[fork_Amsterdam",
    "test_pointer_reentry[fork_Amsterdam",
    "test_pointer_resets_an_empty_code_account_with_storage[fork_Amsterdam",
    "test_pointer_reverts[fork_Amsterdam",
    "test_pointer_to_pointer[fork_Amsterdam",
    "test_pointer_to_precompile[fork_Amsterdam",
    "test_pointer_to_static[fork_Amsterdam",
    "test_pointer_to_static_reentry[fork_Amsterdam",
    "test_static_to_pointer[fork_Amsterdam",
    // EIP-7702 — for_amsterdam/prague/eip7702_set_code_tx/gas/*.
    "test_account_warming[fork_Amsterdam",
    // EIP-8037 — for_amsterdam/amsterdam/eip8037_state_creation_gas_cost_increase/state_gas_set_code/*.
    // 2D-gas tests covering the EIP-7702 auth refund path; same root cause
    // as the EIP-7702 buckets above.
    "test_auth_refund_block_gas_accounting[fork_Amsterdam",
    "test_auth_refund_bypasses_one_fifth_cap[fork_Amsterdam",
    "test_auth_with_calldata_and_access_list[fork_Amsterdam",
    "test_auth_with_multiple_sstores[fork_Amsterdam",
    "test_authorization_exact_state_gas_boundary[fork_Amsterdam",
    "test_authorization_to_precompile_address[fork_Amsterdam",
    "test_authorization_with_sstore[fork_Amsterdam",
    "test_duplicate_signer_authorizations[fork_Amsterdam",
    "test_existing_account_auth_header_gas_used_uses_worst_case[fork_Amsterdam",
    "test_existing_account_refund[fork_Amsterdam",
    "test_existing_account_refund_enables_sstore[fork_Amsterdam",
    "test_existing_auth_with_reverted_execution_preserves_intrinsic[fork_Amsterdam",
    "test_many_authorizations_state_gas[fork_Amsterdam",
    "test_mixed_auths_header_gas_used_uses_worst_case[fork_Amsterdam",
    "test_mixed_new_and_existing_auths[fork_Amsterdam",
    "test_mixed_valid_and_invalid_auths[fork_Amsterdam",
    "test_multi_tx_block_auth_refund_and_sstore[fork_Amsterdam",
    // EIP-8037 — for_amsterdam/amsterdam/eip8037_state_creation_gas_cost_increase/state_gas_pricing/*.
    "test_auth_state_gas_scales_with_cpsb[fork_Amsterdam",
    // EIP-8037 — for_amsterdam/amsterdam/eip8037_state_creation_gas_cost_increase/state_gas_sstore/*.
    "test_sstore_state_gas_all_tx_types[fork_Amsterdam",
    // EIP-7928 — for_amsterdam/amsterdam/eip7928_block_level_access_lists/block_access_lists_eip7702/*.
    // BAL coverage of EIP-7702 delegation flows; expected BAL diffs depend
    // on the same set_delegation refund accounting as above.
    "test_bal_7702_delegation_clear[fork_Amsterdam",
    "test_bal_7702_delegation_create[fork_Amsterdam",
    "test_bal_7702_delegation_update[fork_Amsterdam",
    "test_bal_7702_double_auth_reset[fork_Amsterdam",
    "test_bal_7702_double_auth_swap[fork_Amsterdam",
    "test_bal_7702_null_address_delegation_no_code_change[fork_Amsterdam",
    "test_bal_selfdestruct_to_7702_delegation[fork_Amsterdam",
    "test_bal_withdrawal_to_7702_delegation[fork_Amsterdam",
    // EIP-7928 — for_amsterdam/amsterdam/eip7928_block_level_access_lists/block_access_lists/*.
    // Aggregate BAL test exercising every tx type incl. set-code; trips
    // for the same reason as the eip7702 BAL bucket.
    "test_bal_all_transaction_types[fork_Amsterdam",
    // EIP-7778 — for_amsterdam/amsterdam/eip7778_block_gas_accounting_without_refunds/gas_accounting/*.
    // Block-level gas accounting tests that interact with the auth refund
    // path; tracked alongside the EIP-7702 bucket.
    "test_multiple_refund_types_in_one_tx[fork_Amsterdam",
    "test_simple_gas_accounting[fork_Amsterdam",
    "test_varying_calldata_costs[fork_Amsterdam",
    // EIP-7708 — for_amsterdam/amsterdam/eip7708_eth_transfer_logs/transfer_logs/*.
    // ETH-transfer-logs aggregate test; fails on the set-code tx variant.
    "test_transfer_with_all_tx_types[fork_Amsterdam",
    // EIP-7976 — for_amsterdam/amsterdam/eip7976_increase_calldata_floor_cost/refunds/*.
    // Calldata-floor refund accounting; interacts with the same auth-refund
    // accounting changes.
    "test_gas_refunds_from_data_floor[fork_Amsterdam",
    // EIP-1344 — for_amsterdam/istanbul/eip1344_chainid/chainid/*.
    // Istanbul chainid test re-run as an Amsterdam fork-transition fixture;
    // currently trips on the transition-test runner path rather than on the
    // chainid opcode itself.
    "test_chainid[fork_Amsterdam",
];

// Extra skips added only for prover backends.
#[cfg(all(feature = "sp1", not(feature = "stateless")))]
const EXTRA_SKIPS: &[&str] = &[
    // I believe these tests fail because of how much stress they put into the zkVM, they probably cause an OOM though this should be checked
    "static_Call50000",
    "Return50000",
    "static_Call1MB1024Calldepth",
];
#[cfg(feature = "stateless")]
const EXTRA_SKIPS: &[&str] = &[
    // zkevm@v0.3.3 tolerance tests: the fixture's `statelessOutputBytes` declares `valid = 1`
    // because the executed path does not actually consume the malformed/extra/missing witness
    // entry, but our RpcExecutionWitness conversion eagerly validates the full witness and
    // rejects it. Re-enable once the witness conversion is lazy per EIP-8025 §Tolerance.
    "validation_headers_malformed_rlp_header",
    "validation_headers_missing_oldest_blockhash_ancestor",
    "validation_headers_missing_parent_header",
    "validation_state_extra_unused_trie_node",
    // zkevm@v0.3.3 rejection tests: `statelessOutputBytes` declares `valid = 0` so the guest
    // program must reject the deliberately-incomplete witness, but our stateless path runs
    // to completion instead of detecting the missing entry. Re-enable once the witness
    // completeness checks land (missing delegation/external-code bytecodes, non-contiguous
    // header chain detection).
    "validation_codes_missing_delegated_code_on_insufficient_balance_call",
    "validation_codes_missing_external_code_read_target",
    "validation_codes_missing_redelegation_old_marker",
    "validation_codes_missing_sender_delegation_marker",
    "validation_headers_non_contiguous_chain",
    // zkevm@v0.3.3 conversion-time rejection: `statelessOutputBytes` declares `valid = 0` and
    // our `into_execution_witness` correctly rejects the witness because it can't extract the
    // initial state root without the parent header. Since 5a597e67d the runner treats
    // conversion errors as unconditional regressions, so this correct-rejection-at-the-wrong-
    // stage trips the test. Re-enable once conversion is lazy enough to defer the parent-
    // header check to execution.
    "validation_headers_empty_block_missing_mandatory_parent",
];
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
const EXTRA_SKIPS: &[&str] = &[];

// Select backend
#[cfg(feature = "stateless")]
const BACKEND: Option<BackendType> = Some(BackendType::Exec);
#[cfg(all(feature = "sp1", not(feature = "stateless")))]
const BACKEND: Option<BackendType> = Some(BackendType::SP1);
#[cfg(not(any(feature = "sp1", feature = "stateless")))]
const BACKEND: Option<BackendType> = None;

fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    // Compose the final skip list
    let skips: Vec<&'static str> = SKIPPED_BASE
        .iter()
        .copied()
        .chain(EXTRA_SKIPS.iter().copied())
        .collect();

    parse_and_execute(path, Some(&skips), BACKEND)
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");
