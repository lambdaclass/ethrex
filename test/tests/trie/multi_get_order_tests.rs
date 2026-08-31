//! Real-RocksDB order-preservation gate for [`TrieDB::multi_get`] (batched read
//! primitive backing `Trie::prefetch_sorted`, see `TrieDB::multi_get` at
//! `crates/common/trie/db.rs`).
//!
//! `BackendTrieDB::multi_get` (crates/storage/trie.rs) partitions keys by
//! table (`ACCOUNT_TRIE_NODES`/`ACCOUNT_FLATKEYVALUE`,
//! `STORAGE_TRIE_NODES`/`STORAGE_FLATKEYVALUE`), issues one
//! `read_view.multi_get(table, ...)` per table, then scatters results back to
//! the original input order. `TrieWrapper::multi_get` (crates/storage/layering.rs)
//! layers an overlay probe on top and batches the misses through the same
//! `BackendTrieDB::multi_get`. Neither can be exercised meaningfully against
//! `InMemoryTrieDB` (whose default `multi_get` is just N `get` calls in a
//! loop and can't reorder anything); the only way to catch a reorder/scatter
//! bug is a real RocksDB backend, since `rocksdb::batched_multi_get_cf` with
//! `sorted_input=false` sorts its keys internally before dispatch and the
//! per-table scatter-back logic must reconstruct the caller's original order.
//!
//! This test builds a real committed state trie and a real committed storage
//! trie against an on-disk RocksDB `Store`, reopens both through
//! `Store::open_state_trie`/`open_storage_trie` (the `TrieWrapper`-over-
//! `BackendTrieDB` path production code uses for merkle root recomputation),
//! and asserts that `multi_get` over a large, deliberately-shuffled key batch
//! returns results in exactly the same order (and with exactly the same
//! values) as calling `get` on each key individually.

#![cfg(feature = "rocksdb")]

use ethrex_common::H256;
use ethrex_common::U256;
use ethrex_common::constants::{EMPTY_KECCAK_HASH, EMPTY_TRIE_HASH};
use ethrex_common::types::AccountState;
use ethrex_crypto::NativeCrypto;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};
use ethrex_trie::{Nibbles, Node, TrieDB};
use rand::seq::SliceRandom;
use rand::{SeedableRng, rngs::StdRng};

/// Accounts committed into the real state trie.
const N_ACCOUNTS: u64 = 400;
/// Storage slots committed into the real storage trie of a single account.
const N_SLOTS: u64 = 400;
/// Number of keys sampled per category (existing leaf / absent leaf /
/// existing node / absent node) into each shuffled query batch. Four
/// categories x this constant comfortably clears the "200+" bar needed for
/// RocksDB's internal per-CF sort to actually reorder something relative to
/// the (shuffled) input.
const SAMPLE_PER_CATEGORY: usize = 60;

fn account_hash(i: u64) -> H256 {
    H256(keccak_hash(
        [b"multiget-account".as_ref(), &i.to_be_bytes()].concat(),
    ))
}

fn account_value(i: u64) -> Vec<u8> {
    AccountState {
        nonce: i,
        balance: U256::from(i),
        code_hash: *EMPTY_KECCAK_HASH,
        storage_root: *EMPTY_TRIE_HASH,
    }
    .encode_to_vec()
}

fn slot_hash(i: u64) -> H256 {
    H256(keccak_hash(
        [b"multiget-slot".as_ref(), &i.to_be_bytes()].concat(),
    ))
}

fn slot_value(i: u64) -> Vec<u8> {
    U256::from(i + 1).encode_to_vec()
}

/// A 32-byte path deterministically derived but never inserted as an account
/// or slot key, used as an "absent leaf-length key" query.
fn absent_leaf_path(i: u64) -> H256 {
    H256(keccak_hash(
        [b"multiget-absent-leaf".as_ref(), &i.to_be_bytes()].concat(),
    ))
}

/// Harvests real, on-disk node-table storage keys from a committed trie by
/// walking it and keeping only Branch/Extension entries.
///
/// `TrieIterator` returns, for a `Leaf`, the *full* logical path (parent path
/// extended with the leaf's own partial) rather than the shorter prefix the
/// leaf node is actually stored under (see `NodeRef::commit`, which pushes
/// the leaf node's encoding at the path *before* consuming `leaf.partial`).
/// Branch/Extension entries don't have this discrepancy: the path yielded by
/// the iterator is exactly the key `BackendTrieDB` stored them under.
fn harvest_node_paths(trie: ethrex_trie::Trie) -> Vec<Nibbles> {
    trie.into_iter()
        .filter_map(|(path, node)| (!matches!(node, Node::Leaf(_))).then_some(path))
        .collect()
}

/// Builds a deliberately non-sorted batch mixing:
/// - existing leaf-length keys (real committed leaf values, routes to the
///   flat-key-value table)
/// - absent leaf-length keys (same shape, no value on disk)
/// - existing node-length keys (real committed Branch/Extension nodes,
///   routes to the trie-node table)
/// - absent node-length keys (perturbed real node paths, overwhelmingly
///   unlikely to collide with a real node)
fn build_shuffled_batch(
    existing_leaf_paths: impl Iterator<Item = H256>,
    absent_leaf_paths: impl Iterator<Item = H256>,
    node_paths: &[Nibbles],
    rng: &mut StdRng,
) -> Vec<Nibbles> {
    let mut keys: Vec<Nibbles> = Vec::new();

    keys.extend(
        existing_leaf_paths
            .take(SAMPLE_PER_CATEGORY)
            .map(|h| Nibbles::from_bytes(h.as_bytes())),
    );
    keys.extend(
        absent_leaf_paths
            .take(SAMPLE_PER_CATEGORY)
            .map(|h| Nibbles::from_bytes(h.as_bytes())),
    );
    keys.extend(node_paths.iter().take(SAMPLE_PER_CATEGORY).cloned());
    keys.extend(node_paths.iter().take(SAMPLE_PER_CATEGORY).map(|path| {
        // Flip the last nibble (wrapping) to perturb a real node path into one
        // that, overwhelmingly likely, doesn't exist, while staying in the
        // same (node-table) length class.
        let mut bytes = path.clone().into_vec();
        if let Some(last) = bytes.last_mut() {
            *last ^= 0x0F;
        } else {
            bytes.push(0x0F);
        }
        Nibbles::from_hex(bytes)
    }));

    keys.shuffle(rng);
    keys
}

/// Asserts that `multi_get` preserves input order and values against a real
/// RocksDB-backed `TrieDB`: for every index `i`, `multi_get(keys)[i]` must
/// equal an independent `get(keys[i])`.
fn assert_multi_get_matches_single_get(db: &dyn TrieDB, keys: &[Nibbles]) {
    assert!(
        keys.len() >= 200,
        "batch too small ({}) to reliably trigger RocksDB's internal reordering",
        keys.len()
    );

    // The batch must not already happen to be sorted, or a reorder bug in the
    // scatter-back logic wouldn't be observable.
    let mut sorted = keys.to_vec();
    sorted.sort();
    assert_ne!(
        sorted, keys,
        "key batch is already sorted; this test can't catch an order bug"
    );

    let batched = db.multi_get(keys);
    assert_eq!(batched.len(), keys.len());

    let mut hits = 0usize;
    for (i, (key, batched_result)) in keys.iter().zip(batched).enumerate() {
        let batched_value = batched_result.unwrap_or_else(|e| {
            panic!("multi_get returned an error at index {i} (key {key:?}): {e}")
        });
        let single_value = db
            .get(key.clone())
            .unwrap_or_else(|e| panic!("get returned an error for key {key:?}: {e}"));
        assert_eq!(
            batched_value, single_value,
            "multi_get result at index {i} (key {key:?}) doesn't match a single get for the \
             same key: order was not preserved or the wrong value was scattered back"
        );
        if single_value.is_some() {
            hits += 1;
        }
    }
    assert!(
        hits > 0,
        "no key in the batch resolved to a value; test wouldn't catch a wrong-value scatter bug"
    );
}

#[test]
fn multi_get_preserves_input_order_against_real_rocksdb() {
    let dir = tempfile::tempdir().expect("tmp");
    let store = Store::new(dir.path().to_str().unwrap(), EngineType::RocksDB).expect("store");

    // --- Build and commit a real account (state) trie. ---
    let mut state_direct = store.open_direct_state_trie(*EMPTY_TRIE_HASH).unwrap();
    for i in 0..N_ACCOUNTS {
        state_direct
            .insert(account_hash(i).as_bytes().to_vec(), account_value(i))
            .unwrap();
    }
    let state_root = state_direct.hash(&NativeCrypto).unwrap();

    let state_node_paths = harvest_node_paths(store.open_direct_state_trie(state_root).unwrap());
    assert!(
        state_node_paths.len() >= SAMPLE_PER_CATEGORY,
        "not enough distinct Branch/Extension nodes harvested from the state trie ({})",
        state_node_paths.len()
    );

    // --- Build and commit a real storage trie for one account. ---
    let owner_account = account_hash(0);
    let mut storage_direct = store
        .open_direct_storage_trie(owner_account, *EMPTY_TRIE_HASH)
        .unwrap();
    for i in 0..N_SLOTS {
        storage_direct
            .insert(slot_hash(i).as_bytes().to_vec(), slot_value(i))
            .unwrap();
    }
    let storage_root = storage_direct.hash(&NativeCrypto).unwrap();

    let storage_node_paths = harvest_node_paths(
        store
            .open_direct_storage_trie(owner_account, storage_root)
            .unwrap(),
    );
    assert!(
        storage_node_paths.len() >= SAMPLE_PER_CATEGORY,
        "not enough distinct Branch/Extension nodes harvested from the storage trie ({})",
        storage_node_paths.len()
    );

    // --- Reopen through the layered path (TrieWrapper over BackendTrieDB):
    // the same `db` shape the merkle write path uses, so this exercises both
    // `TrieWrapper::multi_get` (overlay probe + batched miss fill) and
    // `BackendTrieDB::multi_get` (per-table partition/scatter) in one call. ---
    let state_trie = store.open_state_trie(state_root).unwrap();
    let storage_trie = store
        .open_storage_trie(owner_account, state_root, storage_root)
        .unwrap();

    let mut rng = StdRng::seed_from_u64(0xC0FFEE);

    let state_keys = build_shuffled_batch(
        (0..N_ACCOUNTS).map(account_hash),
        (0..).map(absent_leaf_path),
        &state_node_paths,
        &mut rng,
    );
    assert_multi_get_matches_single_get(state_trie.db(), &state_keys);

    let storage_keys = build_shuffled_batch(
        (0..N_SLOTS).map(slot_hash),
        (1_000_000..).map(absent_leaf_path),
        &storage_node_paths,
        &mut rng,
    );
    assert_multi_get_matches_single_get(storage_trie.db(), &storage_keys);
}
