use ef_tests_blockchain::test_runner::parse_and_execute;
use std::path::Path;

// test-levm reads snobal-devnet-6 + legacy from `vectors/`; test-stateless reads
// the generated #3248+#3278 conformance vectors from a separate root so the two
// do not overlay each other. See the Makefile for how each is populated.
//
// Only the `blockchain_tests/` subtree: the generator also emits a sibling
// `blockchain_tests_engine/` in the Engine-API fixture format and a `.meta/`
// directory, neither of which this harness can parse.
//
// It must be the generated set, not the downloaded `vectors_zkevm/` one. That
// bundle is tests-zkevm@v0.6.2, which predates execution-specs #3278 while
// reusing schema id 0x1501, so its `statelessOutputBytes` are 69 bytes against
// the 43 this client now produces — every wire-path comparison would fail.
#[cfg(feature = "stateless")]
const TEST_FOLDER: &str = "vectors_stateless_3278/blockchain_tests/";
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
];

// Fixtures that fail in ORDINARY block execution, before any stateless
// validation runs — `add_block_pipeline` rejects them with `GasUsedMismatch` or
// `ReceiptsRootMismatch`. They are gas-schedule skew between the vectors and
// this client, not stateless conformance gaps: the generated set is filled at
// execution-specs `3c3b6f4af`, which carries the glamsterdam-devnet-7 schedule
// (`COLD_STORAGE_ACCESS` 3000, `ACCOUNT_WRITE` 8000, `CALL_VALUE` 10300,
// access-list keys at full cold cost), while this client implements devnet-8
// (2100 / 9000 / 11300, access-list at cold minus `WARM_ACCESS`).
//
// The pin cannot move: `eip8025_optional_proofs` does not exist on the current
// `forks/amsterdam` default branch, and `3c3b6f4af` has diverged from it, so no
// upstream revision carries both these tests and the devnet-8 schedule. 60 of
// the 105 fixtures still run, which is what keeps the witness and SSZ paths
// covered and the empty-suite guard in `parse_and_execute` meaningful.
//
// Two groups, because they lift at different times. See docs/known_issues.md.
#[cfg(feature = "stateless")]
const EXTRA_SKIPS: &[&str] = &[
    // Failed even when this client targeted devnet-7, so these are skew against
    // `3c3b6f4af` itself rather than against the devnet-8 move.
    "test_witness_codes_auth_nonce_mismatch",
    "test_witness_codes_redelegation_old_marker_included_new_marker_excluded",
    "test_witness_codes_reset_delegation",
    "test_witness_codes_failed_create_after_initcode_read",
    "test_validation_codes_missing_redelegation_old_marker",
    // Started failing when the client moved to the devnet-8 gas schedule; these
    // lift as a block once the vectors are filled against devnet-8.
    "test_validation_codes_missing_delegated_code_on_insufficient_balance_call",
    "test_validation_state_extra_unused_trie_node",
    "test_validation_state_missing_absent_slot_proof_leaf_node",
    "test_validation_state_missing_delete_auxiliary_node",
    "test_validation_state_missing_failed_call_target_account_proof_node",
    "test_validation_state_missing_storage_proof_node",
    "test_validation_state_unsorted_but_complete",
    "test_witness_codes_create2_excludes_new_bytecode",
    "test_witness_codes_create_same_hash_then_read",
    "test_witness_codes_create_then_call_same_block",
    "test_witness_codes_create_then_call_same_tx",
    "test_witness_codes_create_then_selfdestruct_same_tx",
    "test_witness_codes_dedup_identical_bytecode",
    "test_witness_codes_delegated_eoa_insufficient_balance",
    "test_witness_codes_delegation_set_in_same_block",
    "test_witness_codes_failed_create_includes_factory",
    "test_witness_codes_initcode_calls_existing_contract",
    "test_witness_codes_reverted_create_same_hash_then_read",
    "test_witness_codes_reverted_inner_call",
    "test_witness_codes_reverted_transaction",
    "test_witness_codes_selfdestruct_beneficiary_no_code",
    "test_witness_codes_selfdestruct_in_initcode",
    "test_witness_excludes_bytecode_created_in_same_block",
    "test_witness_keeps_prestate_code_read_even_if_later_created_with_same_hash",
    "test_witness_state_block_diff_delete_insert_before_delete_order",
    "test_witness_state_delete_then_insert_uses_insert_before_delete_order",
    "test_witness_state_delete_with_modified_dirty_sibling_omits_post",
    "test_witness_state_delete_with_new_dirty_sibling_omits_post_state_node",
    "test_witness_state_failed_call_still_contains_target_account_proof",
    "test_witness_state_reverted_inner_sload_still_contains_storage_proof",
    "test_witness_state_reverted_sload_still_contains_storage_proof",
    "test_witness_state_reverted_sstore_still_contains_storage_proof",
    "test_witness_state_sload_absent_slot_contains_storage_proof",
    "test_witness_state_sload_contains_storage_proof",
    "test_witness_state_sstore_delete_branch_collapse_adds_auxiliary_node",
    "test_witness_state_sstore_delete_only_slot_keeps_proof",
    "test_witness_state_sstore_delete_without_collapse_omits_sibling_nodes",
    "test_witness_state_sstore_into_empty_storage_omits_post_state_nodes",
    "test_witness_state_sstore_new_slot_omits_post_state_nodes",
    "test_witness_state_sstore_without_explicit_read_contains_storage_proof",
];
#[cfg(not(feature = "stateless"))]
const EXTRA_SKIPS: &[&str] = &[];

fn blockchain_runner(path: &Path) -> datatest_stable::Result<()> {
    let skips: Vec<&'static str> = SKIPPED_BASE
        .iter()
        .copied()
        .chain(EXTRA_SKIPS.iter().copied())
        .collect();

    // Whether to run stateless validation after the stateful run. There is no
    // backend choice any more: the in-memory paths call
    // `validate_blocks_statelessly` and the wire path calls the guest entrypoint
    // directly, so nothing dispatches on a prover backend.
    parse_and_execute(path, Some(&skips), cfg!(feature = "stateless"))
}

datatest_stable::harness!(blockchain_runner, TEST_FOLDER, r".*");
