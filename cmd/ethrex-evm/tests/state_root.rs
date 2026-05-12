//! Integration tests for `compute_post_state_root`.
//!
//! The expected H256 in `test_compute_post_state_root_stable_value` is pinned
//! from a first successful run so any future change to
//! `apply_account_updates_batch` or state-trie hashing fails this test visibly.
//! See `EXPECTED_ROOT` below for the actual value and the recapture procedure.

use ethrex_common::{
    Address, H256,
    constants::EMPTY_KECCACK_HASH,
    types::{Account, AccountInfo, AccountUpdate, Code},
};
use ethrex_evm::compute_post_state_root;
use rustc_hash::FxHashMap;

fn addr(byte: u8) -> Address {
    let mut a = [0u8; 20];
    a[19] = byte;
    Address::from(a)
}

fn eoa(balance_wei: u64) -> Account {
    Account {
        info: AccountInfo {
            balance: balance_wei.into(),
            nonce: 0,
            code_hash: *EMPTY_KECCACK_HASH,
        },
        code: Code::from_bytecode(Default::default(), &ethrex_common::NativeCrypto),
        storage: Default::default(),
    }
}

/// Build the pre-state and a set of updates that simulate a transfer of 10 wei
/// from address 0x01 (balance 100) to new address 0x03.
///
/// After the transfer:
///   - 0x01 balance: 90
///   - 0x03 balance: 10  (new account)
fn build_transfer_scenario() -> (FxHashMap<Address, Account>, Vec<AccountUpdate>) {
    // Pre-state: one EOA at 0x01 with 100 wei.
    let mut pre_state = FxHashMap::default();
    pre_state.insert(addr(0x01), eoa(100));

    // Updates: reduce 0x01 to 90, create 0x03 with 10.
    let mut update_01 = AccountUpdate::new(addr(0x01));
    update_01.info = Some(AccountInfo {
        balance: 90u64.into(),
        nonce: 0,
        code_hash: *EMPTY_KECCACK_HASH,
    });

    let mut update_03 = AccountUpdate::new(addr(0x03));
    update_03.info = Some(AccountInfo {
        balance: 10u64.into(),
        nonce: 0,
        code_hash: *EMPTY_KECCACK_HASH,
    });

    let updates = vec![update_01, update_03];
    (pre_state, updates)
}

#[test]
fn test_compute_post_state_root_determinism() {
    let (pre_state, updates) = build_transfer_scenario();

    let root1 = compute_post_state_root(&pre_state, &updates).expect("first call must succeed");
    let root2 = compute_post_state_root(&pre_state, &updates).expect("second call must succeed");

    assert_eq!(
        root1, root2,
        "compute_post_state_root must be deterministic"
    );
}

#[test]
fn test_compute_post_state_root_stable_value() {
    // Expected root pinned from first run.
    // If this value changes, apply_account_updates_batch or the trie hashing has changed.
    //
    // To recapture: comment-out the assert below, run the test with --nocapture,
    // observe the printed root, and paste it here.
    //
    // Pinned value (captured 2026-05-12):
    const EXPECTED_ROOT: &str =
        "0xbd7c0251e2981c57d98315641bda8051736eb8ac16705049fb2ae592af24e1c5";

    let (pre_state, updates) = build_transfer_scenario();
    let root = compute_post_state_root(&pre_state, &updates)
        .expect("compute_post_state_root must succeed");

    // Print for easy recapture if the value changes.
    eprintln!("post-state root = {root:?}");

    let expected = EXPECTED_ROOT
        .parse::<H256>()
        .expect("EXPECTED_ROOT must be a valid 0x-prefixed H256 literal");
    assert_eq!(root, expected, "post-state root must match pinned value");
}

#[test]
fn test_empty_updates_is_deterministic() {
    // Two calls with the same pre-state and no updates must produce the same root.
    // This isn't a pre-state-root identity check (the function still runs the trie
    // pipeline); it just pins idempotency of the empty-updates path.
    let mut pre_state = FxHashMap::default();
    pre_state.insert(addr(0x01), eoa(42));

    let root1 = compute_post_state_root(&pre_state, &[]).expect("empty updates must succeed");
    let root2 = compute_post_state_root(&pre_state, &[]).expect("second call must succeed");

    assert_eq!(root1, root2, "empty-updates root must be stable");
}
