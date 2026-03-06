/// Test state hash continuity across consecutive batches.
/// batch N's final_state_hash must equal batch N+1's initial_state_hash.
/// This ensures the prover is correctly chaining state transitions.
mod fixture_types;

use fixture_types::{discover_all_apps, hex_to_h256, load_all_fixtures};

/// Auto-discovery: verify state continuity for ALL apps.
#[test]
fn state_continuity_all_apps() {
    let apps = discover_all_apps();
    assert!(!apps.is_empty(), "No fixture apps found in tests/fixtures/");

    for app in &apps {
        let mut fixtures = load_all_fixtures(app);
        if fixtures.len() < 2 {
            eprintln!("[{app}] Skipping state_continuity: need at least 2 fixtures, found {}", fixtures.len());
            continue;
        }

        fixtures.sort_by_key(|f| f.batch_number);

        for window in fixtures.windows(2) {
            let prev = &window[0];
            let next = &window[1];

            let prev_final = hex_to_h256(&prev.prover.final_state_hash);
            let next_initial = hex_to_h256(&next.prover.initial_state_hash);

            assert_eq!(
                prev_final, next_initial,
                "[{app}] State continuity broken: batch {} final_state ({}) != batch {} initial_state ({})",
                prev.batch_number,
                prev.prover.final_state_hash,
                next.batch_number,
                next.prover.initial_state_hash,
            );
        }
    }
}

/// Auto-discovery: verify chain_id consistency for ALL apps.
#[test]
fn chain_id_consistent_all_apps() {
    let apps = discover_all_apps();
    assert!(!apps.is_empty());

    for app in &apps {
        let fixtures = load_all_fixtures(app);
        if fixtures.is_empty() {
            continue;
        }
        let expected_chain_id = fixtures[0].chain_id;
        for f in &fixtures {
            assert_eq!(
                f.chain_id, expected_chain_id,
                "[{app}] batch {}: chain_id {} != expected {}",
                f.batch_number, f.chain_id, expected_chain_id
            );
        }
    }
}

/// Auto-discovery: verify program_type_id consistency for ALL apps.
#[test]
fn program_type_id_consistent_all_apps() {
    let apps = discover_all_apps();
    assert!(!apps.is_empty());

    for app in &apps {
        let fixtures = load_all_fixtures(app);
        if fixtures.is_empty() {
            continue;
        }
        let expected = fixtures[0].program_type_id;
        for f in &fixtures {
            assert_eq!(
                f.program_type_id, expected,
                "[{app}] batch {}: program_type_id {} != expected {}",
                f.batch_number, f.program_type_id, expected
            );
        }
    }
}
