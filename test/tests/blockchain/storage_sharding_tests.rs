//! Equivalence tests for `compute_sharded_storage_root`: the sharded
//! per-account storage-root computation MUST produce a bit-identical
//! `(root_hash, node_set)` to the serial reference path for every input,
//! including value==0 removals and empty/degenerate tries (a divergence here
//! would be a consensus failure).

use ethrex_blockchain::compute_sharded_storage_root;
use ethrex_common::{H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};
use ethrex_trie::TrieNode;

// ── helpers ──────────────────────────────────────────────────────────────

/// Build an H256 where the top nibble (high 4 bits of byte 0) equals `nibble`
/// and the remaining bytes are derived from `index`. This lets us place keys
/// deterministically in a known shard bucket.
fn key_in_nibble(nibble: u8, index: u64) -> H256 {
    assert!(nibble < 16, "nibble must be 0-15");
    let mut k = H256::from_low_u64_be(index | 1); // ensure non-zero lower bits
    k.0[0] = (nibble << 4) | (k.0[0] & 0x0f);
    k
}

/// Seed the storage trie for `hashed_address` by inserting `initial_slots`
/// directly into the backing DB and return the resulting storage root.
fn seed_storage(store: &Store, hashed_address: H256, initial_slots: &[(H256, U256)]) -> H256 {
    let mut trie = store
        .open_direct_storage_trie(hashed_address, *EMPTY_TRIE_HASH)
        .expect("open direct storage trie");
    for (k, v) in initial_slots {
        trie.insert(k.as_bytes().to_vec(), v.encode_to_vec())
            .expect("insert");
    }
    trie.commit(&NativeCrypto).expect("commit seeded trie");
    trie.hash_no_commit(&NativeCrypto)
}

/// Serial reference for a single account: open the storage trie, apply the
/// same insert/remove loop the per-account Stage B path uses, and collect the
/// changes. Built from public APIs so it is independent of the function under
/// test.
fn serial_reference(
    store: &Store,
    hashed_address: H256,
    parent_state_root: H256,
    storage_root: H256,
    hashed_storage: &[(H256, U256)],
) -> (H256, Vec<TrieNode>) {
    let mut trie = store
        .open_storage_trie(hashed_address, parent_state_root, storage_root)
        .expect("open storage trie");
    for (k, v) in hashed_storage {
        if v.is_zero() {
            trie.remove(k.as_bytes()).expect("remove");
        } else {
            trie.insert(k.as_bytes().to_vec(), v.encode_to_vec())
                .expect("insert");
        }
    }
    trie.collect_changes_since_last_hash(&NativeCrypto)
}

/// Assert that the sharded `(root_hash, nodes)` equals the serial reference.
/// Nodes are compared as sorted sets (nibble path -> RLP bytes) because shard
/// and serial emission order can differ.
fn assert_equiv(serial: (H256, Vec<TrieNode>), sharded: (H256, Vec<TrieNode>), label: &str) {
    let (serial_root, mut serial_nodes) = serial;
    let (sharded_root, mut sharded_nodes) = sharded;

    assert_eq!(serial_root, sharded_root, "{label}: root hash mismatch");

    serial_nodes.sort_by(|a, b| a.0.cmp(&b.0));
    sharded_nodes.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(serial_nodes, sharded_nodes, "{label}: node set mismatch");
}

// ── test 1 ───────────────────────────────────────────────────────────────
// Brand-new storage (EMPTY_TRIE_HASH), 2048 new slots spread across all 16
// nibbles.
#[test]
fn sharded_vs_serial_new_storage_2048_slots() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let hashed_address = H256::from_low_u64_be(0xdeadbeef);
    let parent_state_root = *EMPTY_TRIE_HASH;
    let storage_root = *EMPTY_TRIE_HASH;

    let slots: Vec<(H256, U256)> = (0u64..2048)
        .map(|i| {
            let nibble = (i / 128) as u8; // 0-15
            (key_in_nibble(nibble, i), U256::from(i + 1))
        })
        .collect();

    let serial = serial_reference(
        &store,
        hashed_address,
        parent_state_root,
        storage_root,
        &slots,
    );
    let sharded = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        &slots,
    )
    .expect("sharded");

    assert_equiv(serial, sharded, "new_storage_2048_slots");
}

// ── test 2 ───────────────────────────────────────────────────────────────
// All slots removed: pre-seed 2048 slots, then call with all values == 0.
#[test]
fn sharded_vs_serial_all_slots_removed() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let hashed_address = H256::from_low_u64_be(0xcafe_0002);
    let parent_state_root = *EMPTY_TRIE_HASH;

    let initial: Vec<(H256, U256)> = (0u64..2048)
        .map(|i| {
            let nibble = (i / 128) as u8;
            (key_in_nibble(nibble, i), U256::from(i + 1))
        })
        .collect();

    let storage_root = seed_storage(&store, hashed_address, &initial);

    let removals: Vec<(H256, U256)> = initial.iter().map(|(k, _)| (*k, U256::zero())).collect();

    let serial = serial_reference(
        &store,
        hashed_address,
        parent_state_root,
        storage_root,
        &removals,
    );
    let sharded = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        &removals,
    )
    .expect("sharded");

    assert_eq!(
        serial.0, *EMPTY_TRIE_HASH,
        "all-removals must yield EMPTY_TRIE_HASH"
    );
    assert_equiv(serial, sharded, "all_slots_removed");
}

// ── test 3 ───────────────────────────────────────────────────────────────
// Mixed: pre-seed 512 slots, then insert new, remove some, update others.
#[test]
fn sharded_vs_serial_mixed_insert_remove_update() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let hashed_address = H256::from_low_u64_be(0xbabe_0003);
    let parent_state_root = *EMPTY_TRIE_HASH;

    let initial: Vec<(H256, U256)> = (0u64..512)
        .map(|i| {
            let nibble = (i / 32) as u8;
            (key_in_nibble(nibble, i), U256::from(i + 100))
        })
        .collect();
    let storage_root = seed_storage(&store, hashed_address, &initial);

    let new_inserts: Vec<(H256, U256)> = (512u64..640)
        .map(|i| {
            let nibble = (i / 8) as u8 % 16;
            (key_in_nibble(nibble, i + 10_000), U256::from(i + 999))
        })
        .collect();

    let removes: Vec<(H256, U256)> = initial[..128]
        .iter()
        .map(|(k, _)| (*k, U256::zero()))
        .collect();

    let updates: Vec<(H256, U256)> = initial[128..256]
        .iter()
        .map(|(k, v)| (*k, *v * U256::from(2)))
        .collect();

    let mut delta: Vec<(H256, U256)> = Vec::new();
    delta.extend_from_slice(&new_inserts);
    delta.extend_from_slice(&removes);
    delta.extend_from_slice(&updates);

    let serial = serial_reference(
        &store,
        hashed_address,
        parent_state_root,
        storage_root,
        &delta,
    );
    let sharded = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        &delta,
    )
    .expect("sharded");

    assert_equiv(serial, sharded, "mixed_insert_remove_update");
}

// ── test 4 ───────────────────────────────────────────────────────────────
// Single-nibble concentration: 2048 keys all with top nibble 0xA.
#[test]
fn sharded_vs_serial_single_nibble_concentration() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let hashed_address = H256::from_low_u64_be(0xface_0004);
    let parent_state_root = *EMPTY_TRIE_HASH;
    let storage_root = *EMPTY_TRIE_HASH;

    let slots: Vec<(H256, U256)> = (0u64..2048)
        .map(|i| (key_in_nibble(0xa, i), U256::from(i + 1)))
        .collect();

    let serial = serial_reference(
        &store,
        hashed_address,
        parent_state_root,
        storage_root,
        &slots,
    );
    let sharded = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        &slots,
    )
    .expect("sharded");

    assert_equiv(serial, sharded, "single_nibble_concentration");
}

// ── test 5 ───────────────────────────────────────────────────────────────
// Exactly one slot remaining: pre-seed several, remove all but one.
#[test]
fn sharded_vs_serial_single_slot_remaining() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let hashed_address = H256::from_low_u64_be(0x1234_0005);
    let parent_state_root = *EMPTY_TRIE_HASH;

    let initial: Vec<(H256, U256)> = (0u64..16)
        .map(|i| (key_in_nibble(i as u8, i + 1), U256::from(i + 1)))
        .collect();
    let storage_root = seed_storage(&store, hashed_address, &initial);

    let survivor_key = initial[5].0;
    let delta: Vec<(H256, U256)> = initial
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != 5)
        .map(|(_, (k, _))| (*k, U256::zero()))
        .collect();

    let serial = serial_reference(
        &store,
        hashed_address,
        parent_state_root,
        storage_root,
        &delta,
    );
    let sharded = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        &delta,
    )
    .expect("sharded");

    assert_ne!(
        serial.0, *EMPTY_TRIE_HASH,
        "one remaining slot must produce a non-empty root"
    );
    assert!(!survivor_key.is_zero(), "survivor key is sane");
    assert_equiv(serial, sharded, "single_slot_remaining");
}

// ── test 6 ───────────────────────────────────────────────────────────────
// Exactly two active buckets: all keys in nibble 0x3 and 0xC only.
#[test]
fn sharded_vs_serial_two_bucket_branch() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let hashed_address = H256::from_low_u64_be(0x5678_0006);
    let parent_state_root = *EMPTY_TRIE_HASH;
    let storage_root = *EMPTY_TRIE_HASH;

    let mut slots: Vec<(H256, U256)> = Vec::new();
    for i in 0u64..512 {
        slots.push((key_in_nibble(0x3, i), U256::from(i + 1)));
    }
    for i in 0u64..512 {
        slots.push((key_in_nibble(0xc, i + 512), U256::from(i + 1)));
    }

    let serial = serial_reference(
        &store,
        hashed_address,
        parent_state_root,
        storage_root,
        &slots,
    );
    let sharded = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        &slots,
    )
    .expect("sharded");

    assert_equiv(serial, sharded, "two_bucket_branch");
}

// ── test 7 ───────────────────────────────────────────────────────────────
// Normal spread: a few hundred slots across all 16 buckets, mixed operations
// against a pre-seeded trie.
#[test]
fn sharded_vs_serial_normal_spread_mixed() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let hashed_address = H256::from_low_u64_be(0x9abc_0007);
    let parent_state_root = *EMPTY_TRIE_HASH;

    let initial: Vec<(H256, U256)> = (0u64..256)
        .map(|i| {
            let nibble = (i / 16) as u8;
            (key_in_nibble(nibble, i + 50_000), U256::from(i + 1))
        })
        .collect();
    let storage_root = seed_storage(&store, hashed_address, &initial);

    let mut delta: Vec<(H256, U256)> = Vec::new();
    for i in 0u64..64 {
        let nibble = (i / 4) as u8;
        delta.push((key_in_nibble(nibble, i + 100_000), U256::from(i + 9999)));
    }
    for (k, _) in &initial[..64] {
        delta.push((*k, U256::zero()));
    }
    for (k, v) in &initial[64..128] {
        delta.push((*k, *v + U256::from(1)));
    }

    let serial = serial_reference(
        &store,
        hashed_address,
        parent_state_root,
        storage_root,
        &delta,
    );
    let sharded = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        &delta,
    )
    .expect("sharded");

    assert_equiv(serial, sharded, "normal_spread_mixed");
}
