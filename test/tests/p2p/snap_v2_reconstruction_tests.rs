//! Rebuilding the tries from the snap/2 flat state, and the retention bound on
//! rolling that state forward (devp2p `caps/snap.md`, "Synchronization
//! algorithm").
//!
//! The root check here is the only one a snap/2 sync performs. snap/1 gets one
//! for free because healing walks down from the pivot root and cannot finish
//! unless the trie reaches it; snap/2 removes healing, so if this accepts a
//! flat state that does not describe the pivot, nothing else will catch it.

use ethrex_common::{
    BigEndianHash, H256, U256,
    types::{AccountState, BlockHeader},
};
use ethrex_p2p::sync::snap2::{
    FlatState, MAX_CATCH_UP_BLOCKS, catch_up_exceeds_retention, reconstruct_and_verify,
};
use ethrex_storage::{EngineType, Store};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ethrex-snap2-recon-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp datadir");
    dir
}

fn store() -> Store {
    Store::new("memory", EngineType::InMemory).expect("in-memory store")
}

fn at(value: u64) -> H256 {
    H256::from_uint(&U256::from(value))
}

fn header(number: u64, state_root: H256) -> BlockHeader {
    BlockHeader {
        number,
        state_root,
        ..Default::default()
    }
}

/// Build the same state through the store's own trie API, to get the root the
/// reconstruction has to arrive at independently.
async fn expected_root(store: &Store, accounts: &[(H256, AccountState)]) -> H256 {
    use ethrex_crypto::NativeCrypto;
    use ethrex_rlp::encode::RLPEncode;
    let mut trie = store
        .open_direct_state_trie(*ethrex_common::constants::EMPTY_TRIE_HASH)
        .expect("open state trie");
    for (account_hash, account) in accounts {
        trie.insert(account_hash.0.to_vec(), account.encode_to_vec())
            .expect("insert account");
    }
    trie.hash(&NativeCrypto).expect("hash state trie")
}

/// An account-only state reconstructs to the root its leaves imply.
#[tokio::test]
async fn reconstructs_accounts_to_the_expected_root() {
    let store = store();
    let flat = FlatState::open(&temp_dir("accounts")).expect("open");

    let accounts: Vec<(H256, AccountState)> = (1u64..=5)
        .map(|n| {
            (
                at(n),
                AccountState {
                    nonce: n,
                    balance: U256::from(n * 100),
                    ..Default::default()
                },
            )
        })
        .collect();
    for (account_hash, account) in &accounts {
        flat.put_account(*account_hash, account).expect("write");
    }

    let root = expected_root(&store, &accounts).await;
    reconstruct_and_verify(&store, &flat, &header(10, root))
        .await
        .expect("reconstruction must match");

    flat.destroy().await.expect("destroy");
}

/// A contract's leaf is written with the root its slots hash to, not the one it
/// was served with. This is what makes a flat state assembled across several
/// pivots come out consistent.
#[tokio::test]
async fn a_stale_served_storage_root_is_replaced() {
    let store = store();
    let flat = FlatState::open(&temp_dir("stale-root")).expect("open");

    let contract = at(1);
    // Seed the leaf with a storage root that has nothing to do with its slots,
    // standing in for one served at an older pivot.
    flat.put_account(
        contract,
        &AccountState {
            nonce: 1,
            balance: U256::from(10),
            storage_root: H256::repeat_byte(0xab),
            ..Default::default()
        },
    )
    .expect("write account");
    for slot in 1u64..=3 {
        flat.put_slot(contract, at(slot), U256::from(slot * 7))
            .expect("write slot");
    }

    // The root the storage actually hashes to.
    let mut storage_trie = store
        .open_direct_storage_trie(contract, *ethrex_common::constants::EMPTY_TRIE_HASH)
        .expect("open storage trie");
    for slot in 1u64..=3 {
        use ethrex_rlp::encode::RLPEncode;
        storage_trie
            .insert(at(slot).0.to_vec(), U256::from(slot * 7).encode_to_vec())
            .expect("insert slot");
    }
    let real_storage_root = storage_trie
        .hash(&ethrex_crypto::NativeCrypto)
        .expect("hash storage trie");
    assert_ne!(real_storage_root, H256::repeat_byte(0xab));

    let root = expected_root(
        &store,
        &[(
            contract,
            AccountState {
                nonce: 1,
                balance: U256::from(10),
                storage_root: real_storage_root,
                ..Default::default()
            },
        )],
    )
    .await;

    reconstruct_and_verify(&store, &flat, &header(10, root))
        .await
        .expect("reconstruction must use the recomputed storage root");

    flat.destroy().await.expect("destroy");
}

/// A flat state that does not describe the pivot is rejected. Nothing
/// downstream would catch it.
#[tokio::test]
async fn a_flat_state_that_misses_the_pivot_is_rejected() {
    let store = store();
    let flat = FlatState::open(&temp_dir("mismatch")).expect("open");

    flat.put_account(
        at(1),
        &AccountState {
            nonce: 1,
            ..Default::default()
        },
    )
    .expect("write");

    let err = reconstruct_and_verify(&store, &flat, &header(10, H256::repeat_byte(0x99)))
        .await
        .expect_err("a wrong root must not be accepted");
    assert!(
        format!("{err}").contains("root"),
        "expected a state root mismatch, got: {err}"
    );

    flat.destroy().await.expect("destroy");
}

/// A single dropped account changes the root, so a partial flat state cannot
/// pass as complete.
#[tokio::test]
async fn a_missing_account_fails_the_root_check() {
    let store = store();
    let flat = FlatState::open(&temp_dir("missing-account")).expect("open");

    let complete: Vec<(H256, AccountState)> = (1u64..=3)
        .map(|n| {
            (
                at(n),
                AccountState {
                    nonce: n,
                    ..Default::default()
                },
            )
        })
        .collect();
    // Everything but the last.
    for (account_hash, account) in &complete[..2] {
        flat.put_account(*account_hash, account).expect("write");
    }

    let root = expected_root(&store, &complete).await;
    reconstruct_and_verify(&store, &flat, &header(10, root))
        .await
        .expect_err("an incomplete flat state must not verify");

    flat.destroy().await.expect("destroy");
}

/// The catch-up bound comes from EIP-7928's retention floor: peers must hold
/// access lists for at least the weak subjectivity period, 3533 epochs.
#[test]
fn the_retention_bound_is_the_weak_subjectivity_period() {
    assert_eq!(MAX_CATCH_UP_BLOCKS, 3533 * 32);
}

/// A gap within the retention window is rolled forward; one past it is not, and
/// caps/snap.md has the node discard its partial state instead.
#[test]
fn only_a_gap_past_retention_is_refused() {
    let from = header(1_000, H256::zero());

    let inside = header(1_000 + MAX_CATCH_UP_BLOCKS, H256::zero());
    assert!(!catch_up_exceeds_retention(&from, &inside));

    let outside = header(1_000 + MAX_CATCH_UP_BLOCKS + 1, H256::zero());
    assert!(catch_up_exceeds_retention(&from, &outside));
}

/// A pivot that has not advanced is not a gap at all.
#[test]
fn a_pivot_that_did_not_move_is_within_retention() {
    let from = header(1_000, H256::zero());
    assert!(!catch_up_exceeds_retention(
        &from,
        &header(1_000, H256::zero())
    ));
    assert!(!catch_up_exceeds_retention(
        &from,
        &header(999, H256::zero())
    ));
}
