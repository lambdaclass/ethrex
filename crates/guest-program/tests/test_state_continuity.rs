/// Test state hash continuity across consecutive batches.
/// batch N's final_state_hash must equal batch N+1's initial_state_hash.
/// This ensures the prover is correctly chaining state transitions.
mod fixture_types;

use fixture_types::{hex_to_h256, load_all_fixtures};

#[test]
fn state_continuity_zk_dex() {
    let mut fixtures = load_all_fixtures("zk-dex");
    if fixtures.len() < 2 {
        eprintln!(
            "Skipping state_continuity: need at least 2 fixtures, found {}",
            fixtures.len()
        );
        return;
    }

    // Sort by batch number to ensure correct order.
    fixtures.sort_by_key(|f| f.batch_number);

    for window in fixtures.windows(2) {
        let prev = &window[0];
        let next = &window[1];

        let prev_final = hex_to_h256(&prev.prover.final_state_hash);
        let next_initial = hex_to_h256(&next.prover.initial_state_hash);

        assert_eq!(
            prev_final, next_initial,
            "State continuity broken: batch {} final_state ({}) != batch {} initial_state ({})",
            prev.batch_number,
            prev.prover.final_state_hash,
            next.batch_number,
            next.prover.initial_state_hash,
        );
    }
}

/// Verify that chain_id is consistent across all fixtures for the same app.
#[test]
fn chain_id_consistent_zk_dex() {
    let fixtures = load_all_fixtures("zk-dex");
    assert!(!fixtures.is_empty());

    let expected_chain_id = fixtures[0].chain_id;
    for f in &fixtures {
        assert_eq!(
            f.chain_id, expected_chain_id,
            "batch {}: chain_id {} != expected {}",
            f.batch_number, f.chain_id, expected_chain_id
        );
    }
}

/// Verify that program_type_id is consistent across all fixtures for the same app.
#[test]
fn program_type_id_consistent_zk_dex() {
    let fixtures = load_all_fixtures("zk-dex");
    assert!(!fixtures.is_empty());

    let expected = fixtures[0].program_type_id;
    for f in &fixtures {
        assert_eq!(
            f.program_type_id, expected,
            "batch {}: program_type_id {} != expected {}",
            f.batch_number, f.program_type_id, expected
        );
    }
}
