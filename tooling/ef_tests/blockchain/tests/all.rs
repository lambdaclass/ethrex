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
    // disagree with bal-devnet-7 (bal@v7.1.1) state-gas accounting. Same root
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
