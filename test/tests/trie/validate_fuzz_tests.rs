use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
};

use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::{
    Node, Trie,
    db::{InMemoryTrieDB, NodeMap},
};
use proptest::{
    collection::{btree_set, vec},
    prelude::*,
    proptest,
    test_runner::Config as ProptestConfig,
};

/// Pad a value to at least 32 bytes so every leaf node's RLP encoding is >= 32
/// bytes, ensuring `NodeHash::Hashed` (not `Inline`) for all nodes. This is
/// required because `Trie::open()` always creates `NodeHash::Hashed(root_hash)`,
/// which won't match an `Inline` hash in `get_node_checked`.
fn pad(val: &[u8]) -> Vec<u8> {
    if val.len() >= 32 {
        val.to_vec()
    } else {
        let mut padded = val.to_vec();
        padded.resize(32, 0);
        padded
    }
}

/// Build two independent Trie instances from the same committed data.
///
/// Inserts all `data` entries (padded to 32+ bytes), commits via `hash()`,
/// then clones the underlying `BTreeMap` into two separate `InMemoryTrieDB`
/// backends. Both tries are opened with the same root hash — mimicking the
/// state after snap sync.
fn build_twin_tries(data: &[Vec<u8>]) -> (Trie, Trie) {
    let inner_map: NodeMap = Arc::new(Mutex::new(BTreeMap::new()));
    let db = InMemoryTrieDB::new(inner_map.clone());
    let mut trie = Trie::new(Box::new(db));

    for val in data {
        let v = pad(val);
        trie.insert(v.clone(), v).unwrap();
    }
    let root_hash = trie.hash().unwrap();
    drop(trie);

    let snapshot = inner_map.lock().unwrap().clone();
    let map_a = Arc::new(Mutex::new(snapshot.clone()));
    let map_b = Arc::new(Mutex::new(snapshot));

    let trie_a = Trie::open(Box::new(InMemoryTrieDB::new(map_a)), root_hash);
    let trie_b = Trie::open(Box::new(InMemoryTrieDB::new(map_b)), root_hash);
    (trie_a, trie_b)
}

/// Build twin tries and also return mutable handles to their DB maps so
/// callers can corrupt them before validation.
fn build_twin_tries_with_maps(data: &[Vec<u8>]) -> (Trie, Trie, NodeMap, NodeMap) {
    let inner_map: NodeMap = Arc::new(Mutex::new(BTreeMap::new()));
    let db = InMemoryTrieDB::new(inner_map.clone());
    let mut trie = Trie::new(Box::new(db));

    for val in data {
        let v = pad(val);
        trie.insert(v.clone(), v).unwrap();
    }
    let root_hash = trie.hash().unwrap();
    drop(trie);

    let snapshot = inner_map.lock().unwrap().clone();
    let map_a: NodeMap = Arc::new(Mutex::new(snapshot.clone()));
    let map_b: NodeMap = Arc::new(Mutex::new(snapshot));

    let trie_a = Trie::open(Box::new(InMemoryTrieDB::new(map_a.clone())), root_hash);
    let trie_b = Trie::open(Box::new(InMemoryTrieDB::new(map_b.clone())), root_hash);
    (trie_a, trie_b, map_a, map_b)
}

/// Try to decode a byte slice as a Node, returning None on decode error or panic.
fn try_decode_node(data: &[u8]) -> Option<Node> {
    let data = data.to_vec();
    catch_unwind(AssertUnwindSafe(move || Node::decode(&data).ok()))
        .ok()
        .flatten()
}

/// Collect the keys of all non-root node entries in the DB map.
/// Node entries are distinguished from leaf value entries by attempting
/// `Node::decode` on the stored data.
fn non_root_node_keys(map: &NodeMap) -> Vec<Vec<u8>> {
    let db = map.lock().unwrap();
    db.iter()
        .skip(1) // skip root (first/shortest-path entry)
        .filter(|(_, v)| try_decode_node(v).is_some())
        .map(|(k, _)| k.clone())
        .collect()
}

/// Remove a random non-root node (index determined by `seed`) from the DB map.
/// Returns `false` if there are no non-root nodes to remove.
fn corrupt_db_remove_node(map: &NodeMap, seed: usize) -> bool {
    let node_keys = non_root_node_keys(map);
    if node_keys.is_empty() {
        return false;
    }
    let key = &node_keys[seed % node_keys.len()];
    map.lock().unwrap().remove(key);
    true
}

/// Flip the first byte of a random non-root node's value.
/// Returns `false` if there are no non-root nodes to corrupt.
fn corrupt_db_flip_bytes(map: &NodeMap, seed: usize) -> bool {
    let node_keys = non_root_node_keys(map);
    if node_keys.is_empty() {
        return false;
    }
    let key = &node_keys[seed % node_keys.len()];
    let mut db = map.lock().unwrap();
    if let Some(value) = db.get_mut(key) {
        if !value.is_empty() {
            value[0] ^= 0xFF;
        }
    }
    true
}

/// Truncate a random non-root node's RLP to its first byte (breaks RLP decode).
/// Returns `false` if there are no non-root nodes to corrupt.
fn corrupt_db_truncate_node(map: &NodeMap, seed: usize) -> bool {
    let node_keys = non_root_node_keys(map);
    if node_keys.is_empty() {
        return false;
    }
    let key = &node_keys[seed % node_keys.len()];
    let mut db = map.lock().unwrap();
    if let Some(value) = db.get_mut(key) {
        if !value.is_empty() {
            value.truncate(1);
        }
    }
    true
}

/// Swap two non-root nodes' values (breaks hash verification).
/// Returns `false` if there are fewer than 2 non-root nodes.
fn corrupt_db_swap_nodes(map: &NodeMap, seed: usize) -> bool {
    let node_keys = non_root_node_keys(map);
    if node_keys.len() < 2 {
        return false;
    }
    let idx_a = seed % node_keys.len();
    let idx_b = (seed / node_keys.len() + 1) % node_keys.len();
    if idx_a == idx_b {
        return false;
    }
    let key_a = node_keys[idx_a].clone();
    let key_b = node_keys[idx_b].clone();
    let mut db = map.lock().unwrap();
    let val_a = db.get(&key_a).cloned();
    let val_b = db.get(&key_b).cloned();
    if let (Some(a), Some(b)) = (val_a, val_b) {
        if a == b {
            return false;
        }
        db.insert(key_a, b);
        db.insert(key_b, a);
    }
    true
}

/// Classify all node keys by type: (leaf_keys, branch_keys, extension_keys).
/// Includes the root node.
fn node_keys_by_type(map: &NodeMap) -> (Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let db = map.lock().unwrap();
    let mut leaves = Vec::new();
    let mut branches = Vec::new();
    let mut extensions = Vec::new();
    for (k, v) in db.iter() {
        if let Some(node) = try_decode_node(v) {
            match node {
                Node::Leaf(_) => leaves.push(k.clone()),
                Node::Branch(_) => branches.push(k.clone()),
                Node::Extension(_) => extensions.push(k.clone()),
            }
        }
    }
    (leaves, branches, extensions)
}

/// Run `validate()` catching panics (e.g. H256 length mismatch on corrupted data).
/// Returns `true` if validation succeeded, `false` if it returned Err or panicked.
fn validate_ok(trie: Trie) -> bool {
    catch_unwind(AssertUnwindSafe(move || trie.validate().is_ok())).unwrap_or(false)
}

/// Run `validate_parallel()` catching panics.
/// Returns `true` if validation succeeded, `false` if it returned Err or panicked.
fn validate_parallel_ok(trie: Trie) -> bool {
    catch_unwind(AssertUnwindSafe(move || trie.validate_parallel().is_ok())).unwrap_or(false)
}

// =============================================================================
// Deterministic edge-case tests
// =============================================================================

#[test]
fn validate_empty_trie() {
    let (trie_a, trie_b) = build_twin_tries(&[]);
    assert!(trie_a.validate().is_ok());
    assert!(trie_b.validate_parallel().is_ok());
}

#[test]
fn validate_single_element() {
    // Value is padded to 32 bytes so the leaf RLP encoding is >= 32 bytes
    let (trie_a, trie_b) = build_twin_tries(&[vec![0x01, 0x02, 0x03]]);
    assert!(trie_a.validate().is_ok());
    assert!(trie_b.validate_parallel().is_ok());
}

#[test]
fn validate_two_elements_branch_root() {
    // Keys with different first nibbles -> Branch at root
    let data = vec![vec![0x00], vec![0x10]];
    let (trie_a, trie_b) = build_twin_tries(&data);
    assert!(trie_a.validate().is_ok());
    assert!(trie_b.validate_parallel().is_ok());
}

#[test]
fn validate_extension_root() {
    // Keys sharing a common prefix -> Extension at root
    let data = vec![vec![0xAA, 0x00], vec![0xAA, 0x10], vec![0xAA, 0x20]];
    let (trie_a, trie_b) = build_twin_tries(&data);
    assert!(trie_a.validate().is_ok());
    assert!(trie_b.validate_parallel().is_ok());
}

#[test]
fn validate_deep_tree() {
    let data: Vec<Vec<u8>> = (0u16..256).map(|i| i.to_be_bytes().to_vec()).collect();
    let (trie_a, trie_b) = build_twin_tries(&data);
    assert!(trie_a.validate().is_ok());
    assert!(trie_b.validate_parallel().is_ok());
}

#[test]
fn validate_both_err_on_root_removal() {
    let data = vec![vec![0x00; 32], vec![0x10; 32], vec![0x20; 32]];

    let inner_map: NodeMap = Arc::new(Mutex::new(BTreeMap::new()));
    let db = InMemoryTrieDB::new(inner_map.clone());
    let mut trie = Trie::new(Box::new(db));
    for val in &data {
        trie.insert(val.clone(), val.clone()).unwrap();
    }
    let root_hash = trie.hash().unwrap();
    drop(trie);

    // Remove the root node (first/shortest-path entry)
    {
        let mut db = inner_map.lock().unwrap();
        let root_key = db.keys().next().unwrap().clone();
        db.remove(&root_key);
    }

    let snapshot = inner_map.lock().unwrap().clone();
    let map_a = Arc::new(Mutex::new(snapshot.clone()));
    let map_b = Arc::new(Mutex::new(snapshot));

    let trie_a = Trie::open(Box::new(InMemoryTrieDB::new(map_a)), root_hash);
    let trie_b = Trie::open(Box::new(InMemoryTrieDB::new(map_b)), root_hash);

    assert!(trie_a.validate().is_err());
    assert!(trie_b.validate_parallel().is_err());
}

// =============================================================================
// Proptest differential tests
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn proptest_validate_both_ok(data in btree_set(vec(any::<u8>(), 1..100), 1..100)) {
        let entries: Vec<Vec<u8>> = data.into_iter().collect();
        let (trie_a, trie_b) = build_twin_tries(&entries);
        prop_assert!(trie_a.validate().is_ok());
        prop_assert!(trie_b.validate_parallel().is_ok());
    }

    #[test]
    fn proptest_validate_both_err_removal(
        data in btree_set(vec(any::<u8>(), 1..100), 3..100),
        seed: usize,
    ) {
        let entries: Vec<Vec<u8>> = data.into_iter().collect();
        let (trie_a, trie_b, map_a, map_b) = build_twin_tries_with_maps(&entries);

        let corrupted_a = corrupt_db_remove_node(&map_a, seed);
        let corrupted_b = corrupt_db_remove_node(&map_b, seed);
        prop_assume!(corrupted_a && corrupted_b);

        // Use panic-safe wrappers: corrupted data may cause H256 panics
        prop_assert!(!validate_ok(trie_a));
        prop_assert!(!validate_parallel_ok(trie_b));
    }

    #[test]
    fn proptest_validate_both_err_flip(
        data in btree_set(vec(any::<u8>(), 1..100), 3..100),
        seed: usize,
    ) {
        let entries: Vec<Vec<u8>> = data.into_iter().collect();
        let (trie_a, trie_b, map_a, map_b) = build_twin_tries_with_maps(&entries);

        let corrupted_a = corrupt_db_flip_bytes(&map_a, seed);
        let corrupted_b = corrupt_db_flip_bytes(&map_b, seed);
        prop_assume!(corrupted_a && corrupted_b);

        // Use panic-safe wrappers: corrupted data may cause H256 panics
        prop_assert!(!validate_ok(trie_a));
        prop_assert!(!validate_parallel_ok(trie_b));
    }

    /// Pure differential oracle: asserts validate() and validate_parallel()
    /// always agree on the outcome (Ok vs Err/panic) regardless of corruption type.
    #[test]
    fn proptest_differential_agreement(
        data in btree_set(vec(any::<u8>(), 1..100), 3..100),
        seed: usize,
        corruption_kind in 0u8..4,
    ) {
        let entries: Vec<Vec<u8>> = data.into_iter().collect();
        let (trie_a, trie_b, map_a, map_b) = build_twin_tries_with_maps(&entries);

        let corrupted = match corruption_kind {
            0 => corrupt_db_remove_node(&map_a, seed) && corrupt_db_remove_node(&map_b, seed),
            1 => corrupt_db_flip_bytes(&map_a, seed) && corrupt_db_flip_bytes(&map_b, seed),
            2 => corrupt_db_truncate_node(&map_a, seed) && corrupt_db_truncate_node(&map_b, seed),
            3 => corrupt_db_swap_nodes(&map_a, seed) && corrupt_db_swap_nodes(&map_b, seed),
            _ => unreachable!(),
        };
        prop_assume!(corrupted);

        // Use panic-safe wrappers: corrupted data may cause H256 panics
        let ok_a = validate_ok(trie_a);
        let ok_b = validate_parallel_ok(trie_b);
        prop_assert_eq!(ok_a, ok_b,
            "Disagreement: validate ok={}, validate_parallel ok={}", ok_a, ok_b);
    }
}

// =============================================================================
// Targeted trie shape tests
// =============================================================================

#[test]
fn validate_full_branch_root() {
    // 16 keys with distinct first nibbles -> full 16-child branch at root
    let data: Vec<Vec<u8>> = (0u8..16)
        .map(|i| {
            let mut key = vec![i << 4]; // first nibble = i
            key.resize(32, 0);
            key
        })
        .collect();
    let (trie_a, trie_b) = build_twin_tries(&data);
    assert!(trie_a.validate().is_ok());
    assert!(trie_b.validate_parallel().is_ok());
}

#[test]
fn validate_extension_chain() {
    // Keys sharing prefix 0xAA,0xBB -> Extension→Extension→Branch
    let data = vec![
        {
            let mut k = vec![0xAA, 0xBB, 0x00];
            k.resize(32, 0);
            k
        },
        {
            let mut k = vec![0xAA, 0xBB, 0x10];
            k.resize(32, 0);
            k
        },
        {
            let mut k = vec![0xAA, 0xBB, 0x20];
            k.resize(32, 0);
            k
        },
    ];
    let (trie_a, trie_b) = build_twin_tries(&data);
    assert!(trie_a.validate().is_ok());
    assert!(trie_b.validate_parallel().is_ok());
}

#[test]
fn validate_skewed_tree() {
    // Single-branch root with deep subtree: [0x00, i, 0..30] for i in 0..16
    let data: Vec<Vec<u8>> = (0u8..16)
        .map(|i| {
            let mut key = vec![0x00, i];
            key.resize(32, 0);
            key
        })
        .collect();
    let (trie_a, trie_b) = build_twin_tries(&data);
    assert!(trie_a.validate().is_ok());
    assert!(trie_b.validate_parallel().is_ok());
}

// =============================================================================
// Per-node-type corruption tests
// =============================================================================

#[test]
fn validate_corrupt_leaf_both_err() {
    let data: Vec<Vec<u8>> = (0u8..10).map(|i| pad(&[i])).collect();
    let (trie_a, trie_b, map_a, map_b) = build_twin_tries_with_maps(&data);

    let (leaves_a, _, _) = node_keys_by_type(&map_a);
    let (leaves_b, _, _) = node_keys_by_type(&map_b);
    assert!(!leaves_a.is_empty(), "expected leaf nodes in trie");

    // Corrupt the first leaf in both DBs
    {
        let mut db = map_a.lock().unwrap();
        if let Some(v) = db.get_mut(&leaves_a[0]) {
            v.truncate(1);
        }
    }
    {
        let mut db = map_b.lock().unwrap();
        if let Some(v) = db.get_mut(&leaves_b[0]) {
            v.truncate(1);
        }
    }

    assert!(!validate_ok(trie_a));
    assert!(!validate_parallel_ok(trie_b));
}

#[test]
fn validate_corrupt_branch_both_err() {
    // 20 entries to ensure internal branch nodes exist
    let data: Vec<Vec<u8>> = (0u8..20).map(|i| pad(&[i])).collect();
    let (trie_a, trie_b, map_a, map_b) = build_twin_tries_with_maps(&data);

    let (_, branches_a, _) = node_keys_by_type(&map_a);
    let (_, branches_b, _) = node_keys_by_type(&map_b);
    assert!(!branches_a.is_empty(), "expected branch nodes in trie");

    // Corrupt the first branch in both DBs
    {
        let mut db = map_a.lock().unwrap();
        if let Some(v) = db.get_mut(&branches_a[0]) {
            v.truncate(1);
        }
    }
    {
        let mut db = map_b.lock().unwrap();
        if let Some(v) = db.get_mut(&branches_b[0]) {
            v.truncate(1);
        }
    }

    assert!(!validate_ok(trie_a));
    assert!(!validate_parallel_ok(trie_b));
}

#[test]
fn validate_corrupt_extension_both_err() {
    // Keys sharing prefix -> Extension at root; corrupt the extension node
    let data = vec![
        {
            let mut k = vec![0xAA, 0xBB, 0x00];
            k.resize(32, 0);
            k
        },
        {
            let mut k = vec![0xAA, 0xBB, 0x10];
            k.resize(32, 0);
            k
        },
        {
            let mut k = vec![0xAA, 0xBB, 0x20];
            k.resize(32, 0);
            k
        },
    ];
    let (trie_a, trie_b, map_a, map_b) = build_twin_tries_with_maps(&data);

    let (_, _, extensions_a) = node_keys_by_type(&map_a);
    let (_, _, extensions_b) = node_keys_by_type(&map_b);
    assert!(!extensions_a.is_empty(), "expected extension nodes in trie");

    // Corrupt the first extension in both DBs
    {
        let mut db = map_a.lock().unwrap();
        if let Some(v) = db.get_mut(&extensions_a[0]) {
            v.truncate(1);
        }
    }
    {
        let mut db = map_b.lock().unwrap();
        if let Some(v) = db.get_mut(&extensions_b[0]) {
            v.truncate(1);
        }
    }

    assert!(!validate_ok(trie_a));
    assert!(!validate_parallel_ok(trie_b));
}
