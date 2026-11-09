//! The flat state a snap/2 sync patches with access-list diffs before it
//! rebuilds the tries (devp2p `caps/snap.md`, "Synchronization algorithm").
//!
//! The reconstruction reads back exactly what these writes leave behind, and
//! any disagreement surfaces only as a wrong state root at the very end, so
//! the read/write/iterate contract is pinned here.

use ethrex_common::{H256, U256, types::AccountState};
use ethrex_p2p::sync::snap2::FlatState;

/// A datadir for one test. `FlatState::destroy` clears the stores it owns
/// inside it, not the datadir itself, so each test wipes its own on entry.
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ethrex-snap2-flat-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp datadir");
    dir
}

fn account(nonce: u64, balance: u64) -> AccountState {
    AccountState {
        nonce,
        balance: U256::from(balance),
        ..Default::default()
    }
}

fn at(value: u64) -> H256 {
    use ethrex_common::BigEndianHash;
    H256::from_uint(&U256::from(value))
}

#[tokio::test]
async fn accounts_round_trip() {
    let dir = temp_dir("accounts-round-trip");
    let flat = FlatState::open(&dir).expect("open flat state");

    assert!(
        flat.get_account(at(1)).expect("read").is_none(),
        "an account never written must read back absent"
    );

    flat.put_account(at(1), &account(7, 42)).expect("write");
    let read = flat.get_account(at(1)).expect("read").expect("present");
    assert_eq!(read.nonce, 7);
    assert_eq!(read.balance, U256::from(42));

    // A later write wins, which is what applying a newer block's diff does.
    flat.put_account(at(1), &account(8, 43)).expect("rewrite");
    let read = flat.get_account(at(1)).expect("read").expect("present");
    assert_eq!(read.nonce, 8);

    flat.delete_account(at(1)).expect("delete");
    assert!(flat.get_account(at(1)).expect("read").is_none());

    flat.destroy().await.expect("destroy");
}

#[tokio::test]
async fn slots_round_trip_and_are_scoped_per_account() {
    let dir = temp_dir("slots-round-trip");
    let flat = FlatState::open(&dir).expect("open flat state");

    assert!(flat.get_slot(at(1), at(9)).expect("read").is_none());

    flat.put_slot(at(1), at(9), U256::from(100)).expect("write");
    assert_eq!(
        flat.get_slot(at(1), at(9)).expect("read"),
        Some(U256::from(100))
    );
    // The same slot hash under a different account is a different key.
    assert!(flat.get_slot(at(2), at(9)).expect("read").is_none());

    flat.delete_slot(at(1), at(9)).expect("delete");
    assert!(flat.get_slot(at(1), at(9)).expect("read").is_none());

    flat.destroy().await.expect("destroy");
}

/// The trie build consumes accounts in hash order; anything else produces a
/// different root.
#[tokio::test]
async fn accounts_iterate_in_hash_order() {
    let dir = temp_dir("accounts-order");
    let flat = FlatState::open(&dir).expect("open flat state");

    for value in [5u64, 1, 4, 2, 3] {
        flat.put_account(at(value), &account(value, value))
            .expect("write");
    }

    let hashes: Vec<H256> = flat
        .iter_accounts()
        .map(|entry| entry.expect("iterate").0)
        .collect();
    assert_eq!(hashes, vec![at(1), at(2), at(3), at(4), at(5)]);

    flat.destroy().await.expect("destroy");
}

/// Slot iteration must stop at the account boundary: the storage trie for one
/// contract cannot absorb another's slots.
#[tokio::test]
async fn slot_iteration_stops_at_the_account_boundary() {
    let dir = temp_dir("slot-boundary");
    let flat = FlatState::open(&dir).expect("open flat state");

    for account_hash in [at(1), at(2), at(3)] {
        for slot in [3u64, 1, 2] {
            flat.put_slot(account_hash, at(slot), U256::from(slot * 10))
                .expect("write");
        }
    }

    let slots: Vec<(H256, U256)> = flat
        .iter_slots(at(2))
        .map(|(hash, encoded)| {
            use ethrex_rlp::decode::RLPDecode;
            (hash, U256::decode(&encoded).expect("decode slot"))
        })
        .collect();

    assert_eq!(
        slots,
        vec![
            (at(1), U256::from(10)),
            (at(2), U256::from(20)),
            (at(3), U256::from(30)),
        ]
    );

    // An account with no slots yields nothing rather than running into its
    // neighbour's keys.
    assert_eq!(flat.iter_slots(at(4)).count(), 0);

    flat.destroy().await.expect("destroy");
}

/// A zero slot value is a real value, distinct from an absent slot: access-list
/// application deletes rather than writing zero, and the two must not alias.
#[tokio::test]
async fn zero_is_distinguishable_from_absent() {
    let dir = temp_dir("zero-slot");
    let flat = FlatState::open(&dir).expect("open flat state");

    flat.put_slot(at(1), at(9), U256::zero()).expect("write");
    assert_eq!(
        flat.get_slot(at(1), at(9)).expect("read"),
        Some(U256::zero())
    );
    assert_eq!(flat.iter_slots(at(1)).count(), 1);

    flat.delete_slot(at(1), at(9)).expect("delete");
    assert!(flat.get_slot(at(1), at(9)).expect("read").is_none());
    assert_eq!(flat.iter_slots(at(1)).count(), 0);

    flat.destroy().await.expect("destroy");
}
