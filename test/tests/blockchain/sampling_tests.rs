use ethrex_blockchain::sampling::{is_provider_role, pick_random_extra_column};
use ethrex_common::H256;

// ── is_provider_role distribution ────────────────────────────────────────────

#[test]
fn provider_role_is_roughly_15_pct() {
    // Sample 10 000 hashes with a fixed local node id; expect 13-17% providers.
    let local_node_id = H256::from_low_u64_be(0xdeadbeef);
    let epoch_seed = 42u64;
    let total: usize = 10_000;
    let providers = (0..total)
        .filter(|&i| {
            is_provider_role(
                local_node_id,
                H256::from_low_u64_be(i as u64),
                epoch_seed,
                false,
            )
        })
        .count();
    let pct = providers * 100 / total;
    assert!(
        (13..=17).contains(&pct),
        "provider pct = {pct}% over {total} hashes (expected 13-17%)"
    );
}

#[test]
fn eager_true_always_provider() {
    // eager=true must return true regardless of tx_hash or epoch_seed.
    let local_node_id = H256::from_low_u64_be(0xabcd);
    for i in 0u64..20 {
        assert!(
            is_provider_role(local_node_id, H256::from_low_u64_be(i), i, true),
            "eager=true must always be provider (i={i})"
        );
    }
}

// ── D1: per-node entropy (different local ids give different decisions) ────────

#[test]
fn different_local_node_ids_give_different_decisions() {
    // Two distinct local node ids must not produce identical provider/sampler
    // decisions for ALL tx hashes (that would mean the local id is ignored).
    let node_a = H256::from_low_u64_be(0x1111_1111);
    let node_b = H256::from_low_u64_be(0x2222_2222);
    let epoch_seed = 7u64;
    let total = 200usize;

    let mut differ = false;
    for i in 0..total {
        let tx = H256::from_low_u64_be(i as u64);
        let a = is_provider_role(node_a, tx, epoch_seed, false);
        let b = is_provider_role(node_b, tx, epoch_seed, false);
        if a != b {
            differ = true;
            break;
        }
    }
    assert!(
        differ,
        "node A and node B must disagree on at least one tx decision over {total} hashes"
    );
}

// ── pick_random_extra_column ──────────────────────────────────────────────────

#[test]
fn extra_column_never_returns_a_custody_bit() {
    // The returned column must NOT be set in custody_mask.
    let custody = 0b1010_1010u128;
    let hash = H256::from_low_u64_be(1);
    if let Some(col) = pick_random_extra_column(custody, hash) {
        assert_eq!(
            (custody >> col) & 1,
            0,
            "column {col} must not be in custody mask"
        );
    }
}

#[test]
fn extra_column_returns_none_when_all_128_set() {
    // When all columns are in custody, there is no extra column.
    assert_eq!(pick_random_extra_column(u128::MAX, H256::zero()), None);
}

#[test]
fn extra_column_returns_value_when_one_slot_free() {
    // Exactly one free column: pick_random_extra_column must return that column.
    let custody = u128::MAX ^ 1; // all set except column 0
    let hash = H256::from_low_u64_be(7);
    let col = pick_random_extra_column(custody, hash).expect("should find free column 0");
    assert_eq!(col, 0);
}
