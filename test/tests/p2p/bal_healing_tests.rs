//! snap/2 BAL replay applier integration tests (EIP-8189).
//!
//! These tests exercise `apply_bal` against an in-memory `Store` and
//! verify post-block state-root convergence plus targeted invariants
//! (creation, destruction, storage diffs, code deployment, delegation
//! clear, bad-state-root detection).

use ethrex_common::{
    Address, H256, U256,
    constants::{EMPTY_KECCAK_HASH, EMPTY_TRIE_HASH},
    types::{
        AccountState, BlockHeader,
        block_access_list::{
            AccountChanges, BalanceChange, BlockAccessList, CodeChange, NonceChange, SlotChange,
            StorageChange,
        },
    },
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_p2p::sync::SyncError;
use ethrex_p2p::sync::bal_healing::{ApplyBalError, apply_bal, try_apply_bal_block};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_storage::{
    EngineType, Store,
    api::tables::{ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES},
    apply_prefix, hash_address, hash_key,
};

fn empty_store() -> Store {
    Store::new("memory", EngineType::InMemory).expect("failed to create in-memory store")
}

fn header_with_root(state_root: H256) -> BlockHeader {
    BlockHeader {
        state_root,
        ..Default::default()
    }
}

fn insert_account_into_store(store: &Store, addr: Address, account: &AccountState) -> H256 {
    let hashed = hash_address(&addr);
    let mut trie = store
        .open_direct_state_trie(*EMPTY_TRIE_HASH)
        .expect("open trie");
    trie.insert(hashed, account.encode_to_vec())
        .expect("insert account");
    let (root, nodes) = trie.collect_changes_since_last_hash(&NativeCrypto);
    let batch: Vec<(Vec<u8>, Vec<u8>)> = nodes
        .into_iter()
        .map(|(path, rlp)| (apply_prefix(None, path).into_vec(), rlp))
        .collect();
    store
        .write_batch(ACCOUNT_TRIE_NODES, batch)
        .expect("write batch");
    root
}

fn insert_storage_slot(
    store: &Store,
    account_hash: H256,
    slot_key: Vec<u8>,
    value: Vec<u8>,
) -> H256 {
    let mut trie = store
        .open_storage_trie(account_hash, *EMPTY_TRIE_HASH, *EMPTY_TRIE_HASH)
        .expect("open storage trie");
    trie.insert(slot_key, value).expect("insert slot");
    let (root, nodes) = trie.collect_changes_since_last_hash(&NativeCrypto);
    let batch: Vec<(Vec<u8>, Vec<u8>)> = nodes
        .into_iter()
        .map(|(path, rlp)| {
            let key = apply_prefix(Some(account_hash), path).into_vec();
            (key, rlp)
        })
        .collect();
    store
        .write_batch(STORAGE_TRIE_NODES, batch)
        .expect("write storage batch");
    root
}

#[test]
fn apply_bal_empty_bal_returns_same_root() {
    let store = empty_store();
    let bal = BlockAccessList::new();
    let root = H256::from([0xABu8; 32]);
    let header = header_with_root(root);
    let result = apply_bal(&store, root, &bal, &header).unwrap();
    assert_eq!(result, root, "empty BAL must return unchanged root");
}

#[test]
fn apply_bal_account_creation() {
    let store = empty_store();
    let addr = Address::from([0x01u8; 20]);

    let mut changes = AccountChanges::new(addr);
    changes.add_balance_change(BalanceChange::new(0, U256::from(100u64)));
    changes.add_nonce_change(NonceChange::new(0, 1));
    let mut bal = BlockAccessList::new();
    bal.add_account_changes(changes);

    let hashed = hash_address(&addr);
    let expected_acct = AccountState {
        balance: U256::from(100u64),
        nonce: 1,
        ..Default::default()
    };
    let mut expected_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
    expected_trie
        .insert(hashed, expected_acct.encode_to_vec())
        .unwrap();
    let (expected_root, _) = expected_trie.collect_changes_since_last_hash(&NativeCrypto);

    let header = header_with_root(expected_root);
    let new_root = apply_bal(&store, *EMPTY_TRIE_HASH, &bal, &header).unwrap();
    assert_eq!(
        new_root, expected_root,
        "creation should produce correct root"
    );

    let trie_after = store.open_state_trie(new_root).unwrap();
    let encoded = trie_after.get(&hash_address(&addr)).unwrap().unwrap();
    let acct = AccountState::decode(&encoded).unwrap();
    assert_eq!(acct.balance, U256::from(100u64));
    assert_eq!(acct.nonce, 1);
}

#[test]
fn apply_bal_account_destruction() {
    let store = empty_store();
    let addr = Address::from([0x02u8; 20]);

    let pre_acct = AccountState {
        balance: U256::from(500u64),
        nonce: 3,
        ..Default::default()
    };
    let pre_root = insert_account_into_store(&store, addr, &pre_acct);

    let mut changes = AccountChanges::new(addr);
    changes.add_balance_change(BalanceChange::new(0, U256::zero()));
    changes.add_nonce_change(NonceChange::new(1, 0));
    let mut bal = BlockAccessList::new();
    bal.add_account_changes(changes);

    let header = header_with_root(*EMPTY_TRIE_HASH);
    let new_root = apply_bal(&store, pre_root, &bal, &header).unwrap();
    assert_eq!(
        new_root, *EMPTY_TRIE_HASH,
        "destroyed account should yield empty root"
    );

    let trie_after = store.open_state_trie(new_root).unwrap();
    assert!(
        trie_after.get(&hash_address(&addr)).unwrap().is_none(),
        "account should be absent after destruction"
    );
}

#[test]
fn apply_bal_storage_slot_deletion() {
    let store = empty_store();
    let addr = Address::from([0x03u8; 20]);
    let slot = U256::from(42u64);

    let hashed_addr = hash_address(&addr);
    let hashed_addr_h256 = H256::from_slice(&hashed_addr);
    let slot_key = hash_key(&H256::from(slot.to_big_endian()));

    let storage_root = insert_storage_slot(
        &store,
        hashed_addr_h256,
        slot_key,
        U256::from(99u64).encode_to_vec(),
    );

    let pre_acct = AccountState {
        balance: U256::from(1u64),
        storage_root,
        ..Default::default()
    };

    let mut pre_state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
    pre_state_trie
        .insert(hashed_addr.clone(), pre_acct.encode_to_vec())
        .unwrap();
    let (pre_root, nodes) = pre_state_trie.collect_changes_since_last_hash(&NativeCrypto);
    let batch: Vec<(Vec<u8>, Vec<u8>)> = nodes
        .into_iter()
        .map(|(path, rlp)| (apply_prefix(None, path).into_vec(), rlp))
        .collect();
    store.write_batch(ACCOUNT_TRIE_NODES, batch).unwrap();

    let mut slot_change = SlotChange::new(slot);
    slot_change.add_change(StorageChange::new(0, U256::zero()));
    let mut changes = AccountChanges::new(addr);
    changes.add_storage_change(slot_change);
    let mut bal = BlockAccessList::new();
    bal.add_account_changes(changes);

    let mut expected_acct = pre_acct;
    expected_acct.storage_root = *EMPTY_TRIE_HASH;
    let mut expected_state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
    expected_state_trie
        .insert(hashed_addr, expected_acct.encode_to_vec())
        .unwrap();
    let (expected_root, _) = expected_state_trie.collect_changes_since_last_hash(&NativeCrypto);

    let header = header_with_root(expected_root);
    let new_root = apply_bal(&store, pre_root, &bal, &header).unwrap();
    assert_eq!(
        new_root, expected_root,
        "slot deletion should produce correct root"
    );
}

#[test]
fn apply_bal_code_deployment() {
    use bytes::Bytes as RawBytes;
    let store = empty_store();
    let addr = Address::from([0x04u8; 20]);
    let bytecode = RawBytes::from(vec![0x60, 0x00, 0x56]);
    let code_hash = keccak(&bytecode);

    let mut changes = AccountChanges::new(addr);
    changes.add_balance_change(BalanceChange::new(0, U256::from(1u64)));
    changes.add_code_change(CodeChange::new(0, bytecode.clone()));
    let mut bal = BlockAccessList::new();
    bal.add_account_changes(changes);

    let hashed = hash_address(&addr);
    let mut expected_state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
    let expected_acct = AccountState {
        balance: U256::from(1u64),
        code_hash,
        ..Default::default()
    };
    expected_state_trie
        .insert(hashed, expected_acct.encode_to_vec())
        .unwrap();
    let (expected_root, _) = expected_state_trie.collect_changes_since_last_hash(&NativeCrypto);

    let header = header_with_root(expected_root);
    let new_root = apply_bal(&store, *EMPTY_TRIE_HASH, &bal, &header).unwrap();
    assert_eq!(
        new_root, expected_root,
        "code deploy should produce correct root"
    );

    let stored_code = store.get_account_code(code_hash).unwrap();
    assert!(stored_code.is_some(), "code should be stored in the store");
    assert_eq!(stored_code.unwrap().bytecode, bytecode);
}

#[test]
fn apply_bal_storage_slot_fresh_creation() {
    let store = empty_store();
    let addr = Address::from([0x06u8; 20]);

    let pre_acct = AccountState {
        balance: U256::from(10u64),
        ..Default::default()
    };
    let pre_root = insert_account_into_store(&store, addr, &pre_acct);

    let slot = U256::from(777u64);
    let post_value = U256::from(42u64);

    let mut slot_change = SlotChange::new(slot);
    slot_change.add_change(StorageChange::new(0, post_value));
    let mut changes = AccountChanges::new(addr);
    changes.add_storage_change(slot_change);
    let mut bal = BlockAccessList::new();
    bal.add_account_changes(changes);

    let hashed_addr = hash_address(&addr);
    let hashed_addr_h256 = H256::from_slice(&hashed_addr);
    let slot_key = hash_key(&H256::from(slot.to_big_endian()));

    let storage_root = insert_storage_slot(
        &store,
        hashed_addr_h256,
        slot_key,
        post_value.encode_to_vec(),
    );
    let mut expected_acct = pre_acct;
    expected_acct.storage_root = storage_root;

    let mut expected_state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
    expected_state_trie
        .insert(hashed_addr, expected_acct.encode_to_vec())
        .unwrap();
    let (expected_root, _) = expected_state_trie.collect_changes_since_last_hash(&NativeCrypto);

    let header = header_with_root(expected_root);
    let new_root = apply_bal(&store, pre_root, &bal, &header).unwrap();
    assert_eq!(
        new_root, expected_root,
        "fresh storage slot write should produce correct root"
    );
}

#[test]
fn apply_bal_detects_bad_state_root() {
    let store = empty_store();
    let addr = Address::from([0xBAu8; 20]);

    let mut changes = AccountChanges::new(addr);
    changes.add_balance_change(BalanceChange::new(0, U256::from(999u64)));
    let mut bal = BlockAccessList::new();
    bal.add_account_changes(changes);

    let bad_root = H256::from([0xFFu8; 32]);
    let header = header_with_root(bad_root);

    let result = apply_bal(&store, *EMPTY_TRIE_HASH, &bal, &header);
    assert!(
        matches!(result, Err(SyncError::StateRootMismatch(_, _))),
        "apply_bal must return StateRootMismatch when header root doesn't match computed root"
    );
}

#[test]
fn apply_bal_delegation_clear() {
    use bytes::Bytes as RawBytes;
    let store = empty_store();
    let addr = Address::from([0x05u8; 20]);
    let old_code = RawBytes::from(vec![0xEF, 0x01, 0x02]);
    let old_code_hash = keccak(&old_code);

    let pre_acct = AccountState {
        balance: U256::from(50u64),
        nonce: 2,
        code_hash: old_code_hash,
        ..Default::default()
    };
    let pre_root = insert_account_into_store(&store, addr, &pre_acct);

    let mut changes = AccountChanges::new(addr);
    changes.add_code_change(CodeChange::new(0, RawBytes::new()));
    let mut bal = BlockAccessList::new();
    bal.add_account_changes(changes);

    let mut expected_acct = pre_acct;
    expected_acct.code_hash = *EMPTY_KECCAK_HASH;
    let mut expected_state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
    expected_state_trie
        .insert(hash_address(&addr), expected_acct.encode_to_vec())
        .unwrap();
    let (expected_root, _) = expected_state_trie.collect_changes_since_last_hash(&NativeCrypto);

    let header = header_with_root(expected_root);
    let new_root = apply_bal(&store, pre_root, &bal, &header).unwrap();
    assert_eq!(
        new_root, expected_root,
        "delegation clear should produce correct root"
    );

    let trie_after = store.open_state_trie(new_root).unwrap();
    let encoded = trie_after.get(&hash_address(&addr)).unwrap().unwrap();
    let acct = AccountState::decode(&encoded).unwrap();
    assert_eq!(
        acct.code_hash, *EMPTY_KECCAK_HASH,
        "code_hash should be EMPTY_KECCAK after delegation clear"
    );
}

#[test]
fn chain_reorg_is_not_recoverable() {
    // SyncError::ChainReorgDetected must be non-recoverable so the outer sync
    // loop falls back to snap/1 healing instead of retrying with the same
    // peer/data, which would re-trigger the same mismatch.
    let err = SyncError::ChainReorgDetected {
        expected_parent: H256::from([1u8; 32]),
        actual_parent: H256::from([2u8; 32]),
    };
    assert!(!err.is_recoverable());
}

// ---------------------------------------------------------------------------
// `try_apply_bal_block` — pure block-level validation + apply.
//
// These tests exercise the per-block validation order (ordering → hash →
// parent → state-root → persist) without touching peers or diagnostics.
// They give Layer A coverage of the BAL replay driver's apply path.
// ---------------------------------------------------------------------------

/// Build a post-Amsterdam header with the given state root + parent hash +
/// `block_access_list_hash` set to the keccak of the empty BAL.
fn post_amsterdam_header_for(
    parent_hash: H256,
    state_root: H256,
    bal: &BlockAccessList,
) -> BlockHeader {
    BlockHeader {
        parent_hash,
        state_root,
        block_access_list_hash: Some(bal.compute_hash()),
        ..Default::default()
    }
}

#[test]
fn try_apply_bal_block_happy_path() {
    let store = empty_store();
    let bal = BlockAccessList::new(); // empty BAL ⇒ post-state = parent state
    let parent_state_root = H256::from([0xABu8; 32]);
    let parent_hash = H256::from([0xCDu8; 32]);
    let header = post_amsterdam_header_for(parent_hash, parent_state_root, &bal);

    let new_root =
        try_apply_bal_block(&store, &header, &bal, parent_state_root, parent_hash).unwrap();
    assert_eq!(
        new_root, parent_state_root,
        "empty BAL preserves state root"
    );

    // BAL must be persisted for serving onward.
    let persisted = store.get_block_access_list(header.hash()).unwrap();
    assert!(persisted.is_some(), "BAL must be persisted to the store");
}

#[test]
fn try_apply_bal_block_rejects_wrong_parent() {
    let store = empty_store();
    let bal = BlockAccessList::new();
    let actual_parent = H256::from([0x11u8; 32]);
    let wrong_expected = H256::from([0x22u8; 32]);
    let header = post_amsterdam_header_for(actual_parent, *EMPTY_TRIE_HASH, &bal);

    let err = try_apply_bal_block(&store, &header, &bal, *EMPTY_TRIE_HASH, wrong_expected)
        .expect_err("parent mismatch must fail");
    assert!(matches!(
        err,
        ApplyBalError::BadParent {
            expected_parent,
            actual_parent: ap,
        } if expected_parent == wrong_expected && ap == actual_parent
    ));
}

#[test]
fn try_apply_bal_block_rejects_bad_bal_hash() {
    let store = empty_store();
    let bal = BlockAccessList::new();
    let parent_hash = H256::from([0xCDu8; 32]);
    // Header claims a BAL hash that does NOT match the actual BAL.
    let header = BlockHeader {
        parent_hash,
        state_root: *EMPTY_TRIE_HASH,
        block_access_list_hash: Some(H256::from([0xDEu8; 32])),
        ..Default::default()
    };

    let err = try_apply_bal_block(&store, &header, &bal, *EMPTY_TRIE_HASH, parent_hash)
        .expect_err("bad BAL hash must fail");
    assert!(matches!(err, ApplyBalError::BadHash { .. }));

    // Failure must NOT have persisted the BAL.
    assert!(
        store
            .get_block_access_list(header.hash())
            .unwrap()
            .is_none()
    );
}

#[test]
fn try_apply_bal_block_rejects_bad_state_root() {
    let store = empty_store();
    let addr = Address::from([0xAAu8; 20]);
    let mut changes = AccountChanges::new(addr);
    changes.add_balance_change(BalanceChange::new(0, U256::from(42u64)));
    let mut bal = BlockAccessList::new();
    bal.add_account_changes(changes);

    let parent_hash = H256::from([0xCDu8; 32]);
    // Header advertises the BAL hash correctly but the WRONG post-state root.
    let header = BlockHeader {
        parent_hash,
        state_root: H256::from([0x99u8; 32]),
        block_access_list_hash: Some(bal.compute_hash()),
        ..Default::default()
    };

    let err = try_apply_bal_block(&store, &header, &bal, *EMPTY_TRIE_HASH, parent_hash)
        .expect_err("bad post-state root must fail");
    assert!(matches!(err, ApplyBalError::BadStateRoot { .. }));
}

#[test]
fn try_apply_bal_block_chain_of_three_advances_state_root() {
    // Layer A end-to-end: apply three consecutive empty BALs and verify the
    // state root threads through correctly. This exercises the exact loop
    // body of `advance_state_via_bals` without needing a `PeerHandler`.
    let store = empty_store();
    let bal = BlockAccessList::new();
    let mut state_root = *EMPTY_TRIE_HASH;
    let mut parent_hash = H256::from([0x00u8; 32]);

    for n in 1u64..=3 {
        let header = BlockHeader {
            number: n,
            parent_hash,
            state_root,
            block_access_list_hash: Some(bal.compute_hash()),
            ..Default::default()
        };
        let new_root = try_apply_bal_block(&store, &header, &bal, state_root, parent_hash).unwrap();
        assert_eq!(new_root, state_root, "empty BAL preserves root each block");
        parent_hash = header.hash();
        state_root = new_root;
    }
}
