/// Test that committer calldata fields match prover public values.
/// This is THE critical test that prevents 00e proof verification failures.
///
/// The L1 OnChainProposer reconstructs publicInputs from the committed data.
/// If any field differs between what the committer sent and what the prover
/// computed, SP1Verifier.verifyProof() will revert with "00e".
mod fixture_types;

use fixture_types::{discover_all_apps, hex_to_h256, load_all_fixtures};

/// Verify that all shared fields between committer and prover are identical.
fn assert_committer_matches_prover(fixture: &fixture_types::TestFixture) {
    let p = &fixture.prover;
    let c = &fixture.committer;
    let batch = fixture.batch_number;

    // 1. State root: committer.new_state_root == prover.final_state_hash
    assert_eq!(
        hex_to_h256(&c.new_state_root),
        hex_to_h256(&p.final_state_hash),
        "batch {batch}: new_state_root != final_state_hash"
    );

    // 2. Withdrawals merkle root: committer.withdrawals_merkle_root == prover.l1_out_messages_merkle_root
    assert_eq!(
        hex_to_h256(&c.withdrawals_merkle_root),
        hex_to_h256(&p.l1_out_messages_merkle_root),
        "batch {batch}: withdrawals_merkle_root != l1_out_messages_merkle_root"
    );

    // 3. Privileged tx rolling hash
    assert_eq!(
        hex_to_h256(&c.priv_tx_rolling_hash),
        hex_to_h256(&p.l1_in_messages_rolling_hash),
        "batch {batch}: priv_tx_rolling_hash != l1_in_messages_rolling_hash"
    );

    // 4. Non-privileged transaction count
    assert_eq!(
        c.non_privileged_txs, p.non_privileged_count,
        "batch {batch}: non_privileged_txs mismatch"
    );

    // 5. Balance diffs count
    assert_eq!(
        c.balance_diffs.len(),
        p.balance_diffs.len(),
        "batch {batch}: balance_diffs count mismatch"
    );

    // 6. L2 in message rolling hashes count
    assert_eq!(
        c.l2_in_message_rolling_hashes.len(),
        p.l2_in_message_rolling_hashes.len(),
        "batch {batch}: l2_in_message_rolling_hashes count mismatch"
    );

    // 7. Balance diffs values (if present)
    for (i, (c_bd, p_bd)) in c.balance_diffs.iter().zip(p.balance_diffs.iter()).enumerate() {
        assert_eq!(
            c_bd.chain_id, p_bd.chain_id,
            "batch {batch}: balance_diff[{i}].chain_id mismatch"
        );
        assert_eq!(
            c_bd.value, p_bd.value,
            "batch {batch}: balance_diff[{i}].value mismatch"
        );
        assert_eq!(
            c_bd.message_hashes.len(),
            p_bd.message_hashes.len(),
            "batch {batch}: balance_diff[{i}].message_hashes count mismatch"
        );
        for (j, (ch, ph)) in c_bd
            .message_hashes
            .iter()
            .zip(p_bd.message_hashes.iter())
            .enumerate()
        {
            assert_eq!(
                hex_to_h256(ch),
                hex_to_h256(ph),
                "batch {batch}: balance_diff[{i}].message_hashes[{j}] mismatch"
            );
        }
    }

    // 8. L2 in message rolling hashes (if present)
    for (i, ((c_cid, c_hash), (p_cid, p_hash))) in c
        .l2_in_message_rolling_hashes
        .iter()
        .zip(p.l2_in_message_rolling_hashes.iter())
        .enumerate()
    {
        assert_eq!(
            c_cid, p_cid,
            "batch {batch}: l2_in_msg[{i}].chain_id mismatch"
        );
        assert_eq!(
            hex_to_h256(c_hash),
            hex_to_h256(p_hash),
            "batch {batch}: l2_in_msg[{i}].hash mismatch"
        );
    }
}

/// Auto-discovery: run across ALL fixtures for ALL apps.
#[test]
fn committer_matches_prover_all_apps() {
    let apps = discover_all_apps();
    assert!(!apps.is_empty(), "No fixture apps found in tests/fixtures/");
    for app in &apps {
        let fixtures = load_all_fixtures(app);
        for f in &fixtures {
            assert_committer_matches_prover(f);
        }
    }
}
