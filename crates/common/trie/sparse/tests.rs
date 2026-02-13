use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::Trie;
use crate::db::{InMemoryTrieDB, TrieDB as _};
use crate::nibbles::Nibbles;

/// A null provider that returns None for all lookups.
struct NullProvider;

impl SparseTrieProvider for NullProvider {
    fn get_node(&self, _path: &Nibbles) -> Result<Option<Vec<u8>>, crate::error::TrieError> {
        Ok(None)
    }
}

#[test]
fn empty_trie_root() {
    let mut trie = SparseTrie::new();
    trie.upper.nodes.insert(Vec::new(), SparseNode::Empty);
    let root = trie.root().expect("should compute root");
    assert_eq!(root, *EMPTY_TRIE_HASH);
}

#[test]
fn single_leaf_hash_direct() {
    // Directly construct a sparse trie with a leaf (bypassing update_leaf)
    // to verify hash computation alone.
    use crate::node::LeafNode;
    use crate::node_hash::NodeHash;

    let path = Nibbles::from_bytes(&[0x01, 0x02]);
    let value = vec![0xAB, 0xCD];

    // Old trie: compute leaf hash directly
    let leaf = LeafNode::new(path.clone(), value.clone());
    let old_hash = leaf.compute_hash();
    let old_root = old_hash.finalize();

    // Sparse trie: manually set up the leaf
    let mut sparse = SparseTrie::new();
    let path_data = path.as_ref().to_vec();
    sparse.upper.nodes.insert(
        Vec::new(),
        SparseNode::Leaf {
            key: Nibbles::from_hex(path_data.clone()),
            hash: None,
        },
    );
    sparse.upper.values.insert(path_data, value.clone());
    sparse.prefix_set.insert(&path);

    let sparse_root = sparse.root().expect("should compute root");

    // Also compute by hand
    let compact = path.encode_compact();
    let mut buf = Vec::new();
    ethrex_rlp::structs::Encoder::new(&mut buf)
        .encode_bytes(&compact)
        .encode_bytes(&value)
        .finish();
    let manual_hash = NodeHash::from_encoded(&buf);
    let manual_root = manual_hash.finalize();

    assert_eq!(
        manual_root, old_root,
        "manual hash should match old trie root"
    );
    assert_eq!(
        sparse_root, old_root,
        "sparse hash should match old trie root"
    );
}

#[test]
fn single_leaf_insert() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    let path = Nibbles::from_bytes(&[0x01, 0x02]);
    let value = vec![0xAB, 0xCD];
    sparse
        .update_leaf(path, value.clone(), &NullProvider)
        .expect("insert should succeed");

    let sparse_root = sparse.root().expect("should compute root");

    // Compare with old trie
    let mut old_trie = Trie::new_temp();
    old_trie
        .insert(vec![0x01, 0x02], value)
        .expect("insert should succeed");
    let old_root = old_trie.hash_no_commit();

    assert_eq!(sparse_root, old_root, "roots should match for single leaf");
}

#[test]
fn two_leaves_same_prefix() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    let path1 = Nibbles::from_bytes(&[0x01]);
    let path2 = Nibbles::from_bytes(&[0x02]);
    let val1 = vec![0x10];
    let val2 = vec![0x20];

    sparse
        .update_leaf(path1, val1.clone(), &NullProvider)
        .expect("insert 1");
    sparse
        .update_leaf(path2, val2.clone(), &NullProvider)
        .expect("insert 2");

    let sparse_root = sparse.root().expect("root");

    let mut old_trie = Trie::new_temp();
    old_trie.insert(vec![0x01], val1).expect("insert 1");
    old_trie.insert(vec![0x02], val2).expect("insert 2");
    let old_root = old_trie.hash_no_commit();

    assert_eq!(sparse_root, old_root, "roots should match for two leaves");
}

#[test]
fn three_leaves_different_branches() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    let entries = vec![
        (vec![0x10], vec![0xAA]),
        (vec![0x20], vec![0xBB]),
        (vec![0x30], vec![0xCC]),
    ];

    for (path, value) in &entries {
        sparse
            .update_leaf(Nibbles::from_bytes(path), value.clone(), &NullProvider)
            .expect("insert");
    }

    let sparse_root = sparse.root().expect("root");

    let mut old_trie = Trie::new_temp();
    for (path, value) in &entries {
        old_trie
            .insert(path.clone(), value.clone())
            .expect("insert");
    }
    let old_root = old_trie.hash_no_commit();

    assert_eq!(sparse_root, old_root);
}

#[test]
fn update_existing_leaf() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    let path = Nibbles::from_bytes(&[0x01, 0x02]);
    sparse
        .update_leaf(path.clone(), vec![0x10], &NullProvider)
        .expect("insert");
    sparse
        .update_leaf(path, vec![0x20], &NullProvider)
        .expect("update");

    let sparse_root = sparse.root().expect("root");

    let mut old_trie = Trie::new_temp();
    old_trie
        .insert(vec![0x01, 0x02], vec![0x10])
        .expect("insert");
    old_trie
        .insert(vec![0x01, 0x02], vec![0x20])
        .expect("update");
    let old_root = old_trie.hash_no_commit();

    assert_eq!(sparse_root, old_root);
}

#[test]
fn leaves_with_shared_prefix() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    // These share the first nibble pair (0x1_)
    let entries = vec![
        (vec![0x10], vec![0xAA]),
        (vec![0x11], vec![0xBB]),
        (vec![0x12], vec![0xCC]),
    ];

    for (path, value) in &entries {
        sparse
            .update_leaf(Nibbles::from_bytes(path), value.clone(), &NullProvider)
            .expect("insert");
    }

    let sparse_root = sparse.root().expect("root");

    let mut old_trie = Trie::new_temp();
    for (path, value) in &entries {
        old_trie
            .insert(path.clone(), value.clone())
            .expect("insert");
    }
    let old_root = old_trie.hash_no_commit();

    assert_eq!(sparse_root, old_root);
}

#[test]
fn remove_single_leaf() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    let path = Nibbles::from_bytes(&[0x01, 0x02]);
    sparse
        .update_leaf(path.clone(), vec![0xAB], &NullProvider)
        .expect("insert");
    sparse.remove_leaf(path, &NullProvider).expect("remove");

    let sparse_root = sparse.root().expect("root");

    // After removing the only leaf, should be empty trie hash
    assert_eq!(sparse_root, *EMPTY_TRIE_HASH);
}

#[test]
fn remove_one_of_two_leaves() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    let path1 = Nibbles::from_bytes(&[0x01]);
    let path2 = Nibbles::from_bytes(&[0x02]);

    sparse
        .update_leaf(path1.clone(), vec![0x10], &NullProvider)
        .expect("insert 1");
    sparse
        .update_leaf(path2, vec![0x20], &NullProvider)
        .expect("insert 2");
    sparse.remove_leaf(path1, &NullProvider).expect("remove 1");

    let sparse_root = sparse.root().expect("root");

    // Compare with old trie that only has path2
    let mut old_trie = Trie::new_temp();
    old_trie.insert(vec![0x02], vec![0x20]).expect("insert");
    let old_root = old_trie.hash_no_commit();

    assert_eq!(sparse_root, old_root);
}

#[test]
fn multiple_operations_match_old_trie() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    let mut old_trie = Trie::new_temp();

    // Insert several entries
    let entries = vec![
        (vec![0x00], vec![0x00]),
        (vec![0x01], vec![0x01]),
        (vec![0x10], vec![0x10]),
        (vec![0x11], vec![0x11]),
        (vec![0xFF], vec![0xFF]),
    ];

    for (path, value) in &entries {
        sparse
            .update_leaf(Nibbles::from_bytes(path), value.clone(), &NullProvider)
            .expect("insert");
        old_trie
            .insert(path.clone(), value.clone())
            .expect("insert");
    }

    let sparse_root = sparse.root().expect("root");
    let old_root = old_trie.hash_no_commit();
    assert_eq!(sparse_root, old_root, "roots should match after inserts");

    // Update some entries
    sparse
        .update_leaf(Nibbles::from_bytes(&[0x01]), vec![0x99], &NullProvider)
        .expect("update");
    old_trie.insert(vec![0x01], vec![0x99]).expect("update");

    let sparse_root = sparse.root().expect("root");
    let old_root = old_trie.hash_no_commit();
    assert_eq!(sparse_root, old_root, "roots should match after update");
}

#[test]
fn prefix_set_basic() {
    let mut ps = PrefixSet::new();
    ps.insert(&Nibbles::from_hex(vec![1, 2, 3]));
    ps.insert(&Nibbles::from_hex(vec![4, 5, 6]));
    ps.ensure_sorted();

    // Exact match
    assert!(ps.contains(&[1, 2, 3]));
    // Prefix match (path is prefix of stored)
    assert!(ps.contains(&[1, 2]));
    // Stored is prefix of path
    assert!(ps.contains(&[1, 2, 3, 4]));
    // No match
    assert!(!ps.contains(&[7, 8]));
}

/// Round-trip test: SparseTrie → collect_updates → InMemoryTrieDB → Trie::get
/// Verifies that nodes emitted by collect_updates can be read back by Trie::get.
#[test]
fn collect_updates_roundtrip_small() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    // Insert a few entries with short keys
    let entries = vec![
        (vec![0x01], vec![0xAA]),
        (vec![0x02], vec![0xBB]),
        (vec![0x10], vec![0xCC]),
    ];

    for (path, value) in &entries {
        sparse
            .update_leaf(Nibbles::from_bytes(path), value.clone(), &NullProvider)
            .expect("insert");
    }

    let root = sparse.root().expect("should compute root");
    let updates = sparse.collect_updates();

    // Store updates in InMemoryTrieDB
    let db = InMemoryTrieDB::new_empty();
    db.put_batch(updates).expect("put_batch");

    // Open a Trie with the same DB and root
    let trie = Trie::open(Box::new(db), root);

    // Verify all entries can be read back
    for (path, expected_value) in &entries {
        let result = trie
            .get(path)
            .unwrap_or_else(|e| panic!("get failed for path {path:?}: {e:?}"));
        assert_eq!(
            result.as_deref(),
            Some(expected_value.as_slice()),
            "mismatch for path {path:?}"
        );
    }
}

/// Round-trip test with 32-byte keys (like hashed addresses).
#[test]
fn collect_updates_roundtrip_32byte_keys() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    // Generate some 32-byte keys (simulating hashed addresses)
    let keys: Vec<[u8; 32]> = (0..5u8)
        .map(|i| {
            let mut key = [0u8; 32];
            key[0] = i;
            key[31] = 0xFF - i;
            key
        })
        .collect();

    let values: Vec<Vec<u8>> = (0..5u8).map(|i| vec![0xAA + i; 4]).collect();

    for (key, value) in keys.iter().zip(values.iter()) {
        sparse
            .update_leaf(Nibbles::from_bytes(key), value.clone(), &NullProvider)
            .expect("insert");
    }

    let root = sparse.root().expect("should compute root");
    let updates = sparse.collect_updates();

    // Store updates in InMemoryTrieDB
    let db = InMemoryTrieDB::new_empty();
    db.put_batch(updates).expect("put_batch");

    // Open a Trie with the same DB and root
    let trie = Trie::open(Box::new(db), root);

    // Verify all entries can be read back
    for (key, expected_value) in keys.iter().zip(values.iter()) {
        let result = trie
            .get(key)
            .unwrap_or_else(|e| panic!("get failed for key {:02x?}: {e:?}", &key[..4]));
        assert_eq!(
            result.as_deref(),
            Some(expected_value.as_slice()),
            "mismatch for key {:02x?}",
            &key[..4]
        );
    }
}

/// Round-trip test comparing SparseTrie collect_updates with old Trie commit.
/// Both should produce the same DB entries.
#[test]
fn collect_updates_matches_old_trie_commit() {
    // Build the same trie with both old and new approaches
    let entries = vec![
        (vec![0x01], vec![0xAA]),
        (vec![0x02], vec![0xBB]),
        (vec![0x10], vec![0xCC]),
    ];

    // Old trie: commit_without_storing both hashes and collects entries
    let mut old_trie = Trie::new_temp();
    for (path, value) in &entries {
        old_trie
            .insert(path.clone(), value.clone())
            .expect("insert");
    }
    let old_updates = old_trie.commit_without_storing();
    let old_root = old_trie.hash_no_commit();

    // New SparseTrie
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);
    for (path, value) in &entries {
        sparse
            .update_leaf(Nibbles::from_bytes(path), value.clone(), &NullProvider)
            .expect("insert");
    }
    let new_root = sparse.root().expect("root");
    let new_updates = sparse.collect_updates();

    assert_eq!(old_root, new_root, "roots should match");

    // Compare update sets - sort by path for comparison
    let mut old_sorted: Vec<(Vec<u8>, Vec<u8>)> = old_updates
        .into_iter()
        .map(|(k, v)| (k.into_vec(), v))
        .collect();
    old_sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut new_sorted: Vec<(Vec<u8>, Vec<u8>)> = new_updates
        .into_iter()
        .map(|(k, v)| (k.into_vec(), v))
        .collect();
    new_sorted.sort_by(|a, b| a.0.cmp(&b.0));

    // Print both for debugging
    eprintln!("=== OLD updates ({}) ===", old_sorted.len());
    for (path, value) in &old_sorted {
        eprintln!("  path={path:?} value={value:02x?}");
    }
    eprintln!("=== NEW updates ({}) ===", new_sorted.len());
    for (path, value) in &new_sorted {
        eprintln!("  path={path:?} value={value:02x?}");
    }

    assert_eq!(old_sorted.len(), new_sorted.len(), "update count mismatch");
    for (old, new) in old_sorted.iter().zip(new_sorted.iter()) {
        assert_eq!(old.0, new.0, "path mismatch");
        assert_eq!(old.1, new.1, "value mismatch for path {:?}", old.0);
    }
}

/// Realistic round-trip: existing state in DB, then apply updates via SparseTrie.
/// This simulates the actual blockchain pipeline: genesis state → apply block → read back.
#[test]
fn roundtrip_existing_state_plus_update() {
    use crate::db::NodeMap;

    // Step 1: Create initial state using old Trie and commit to shared InMemoryTrieDB
    let initial_entries = vec![
        (vec![0x01], vec![0xAA]),
        (vec![0x02], vec![0xBB]),
        (vec![0x10], vec![0xCC]),
        (vec![0x20], vec![0xDD]),
    ];

    let db_inner: NodeMap = Default::default();
    let db = InMemoryTrieDB::new(db_inner.clone());
    let mut old_trie = Trie::new(Box::new(InMemoryTrieDB::new(db_inner.clone())));
    for (path, value) in &initial_entries {
        old_trie
            .insert(path.clone(), value.clone())
            .expect("insert");
    }
    old_trie.commit().expect("commit initial state");
    let initial_root = old_trie.hash_no_commit();

    // Verify initial state is readable
    let trie = Trie::open(
        Box::new(InMemoryTrieDB::new(db_inner.clone())),
        initial_root,
    );
    for (path, expected) in &initial_entries {
        let val = trie.get(path).expect("get").expect("should find");
        assert_eq!(&val, expected, "initial state mismatch for {:?}", path);
    }

    // Step 2: Create a SparseTrie, reveal root from DB, modify one entry, add one new
    let mut sparse = SparseTrie::new();
    let provider = TrieDBProvider(&db as &dyn crate::db::TrieDB);
    sparse.reveal_root(initial_root, &provider).expect("reveal");

    // Update existing entry
    sparse
        .update_leaf(Nibbles::from_bytes(&[0x01]), vec![0xFF], &provider)
        .expect("update");
    // Add new entry
    sparse
        .update_leaf(Nibbles::from_bytes(&[0x30]), vec![0xEE], &provider)
        .expect("insert new");

    let new_root = sparse.root().expect("root");
    let updates = sparse.collect_updates();

    // Step 3: Apply updates to the same DB
    db.put_batch(updates).expect("put_batch");

    // Step 4: Read back ALL entries (including unmodified ones)
    let trie = Trie::open(Box::new(InMemoryTrieDB::new(db_inner)), new_root);

    // Modified entry should have new value
    let val = trie
        .get(&[0x01])
        .expect("get 0x01")
        .expect("should find 0x01");
    assert_eq!(val, vec![0xFF], "updated entry mismatch");

    // Unmodified entries should still be readable
    let val = trie
        .get(&[0x02])
        .expect("get 0x02")
        .expect("should find 0x02");
    assert_eq!(val, vec![0xBB], "unmodified entry 0x02 mismatch");

    let val = trie
        .get(&[0x10])
        .expect("get 0x10")
        .expect("should find 0x10");
    assert_eq!(val, vec![0xCC], "unmodified entry 0x10 mismatch");

    let val = trie
        .get(&[0x20])
        .expect("get 0x20")
        .expect("should find 0x20");
    assert_eq!(val, vec![0xDD], "unmodified entry 0x20 mismatch");

    // New entry should be readable
    let val = trie
        .get(&[0x30])
        .expect("get 0x30")
        .expect("should find 0x30");
    assert_eq!(val, vec![0xEE], "new entry 0x30 mismatch");
}

/// Like roundtrip_existing_state_plus_update but with 32-byte keys (hashed addresses).
#[test]
fn roundtrip_existing_state_32byte_keys() {
    use crate::db::NodeMap;

    // Create deterministic 32-byte keys (simulating hashed addresses)
    let keys: Vec<[u8; 32]> = (0..10u8)
        .map(|i| {
            let mut key = [0u8; 32];
            // Spread the first nibbles to exercise different subtries
            key[0] = i * 0x11; // 0x00, 0x11, 0x22, ...
            key[1] = 0xFF - i;
            key[31] = i;
            key
        })
        .collect();

    // Step 1: Create initial state using old Trie
    let db_inner: NodeMap = Default::default();
    let db = InMemoryTrieDB::new(db_inner.clone());
    let mut old_trie = Trie::new(Box::new(InMemoryTrieDB::new(db_inner.clone())));
    for (i, k) in keys.iter().enumerate() {
        old_trie
            .insert(k.to_vec(), vec![i as u8; 8])
            .expect("insert");
    }
    old_trie.commit().expect("commit");
    let initial_root = old_trie.hash_no_commit();

    // Step 2: SparseTrie - reveal root, modify a few entries
    let mut sparse = SparseTrie::new();
    let provider = TrieDBProvider(&db as &dyn crate::db::TrieDB);
    sparse.reveal_root(initial_root, &provider).expect("reveal");

    // Update key 0
    sparse
        .update_leaf(Nibbles::from_bytes(&keys[0]), vec![0xFF; 8], &provider)
        .expect("update key 0");

    // Remove key 1
    sparse
        .remove_leaf(Nibbles::from_bytes(&keys[1]), &provider)
        .expect("remove key 1");

    let new_root = sparse.root().expect("root");
    let updates = sparse.collect_updates();

    // Step 3: Apply updates
    db.put_batch(updates).expect("put_batch");

    // Step 4: Read back
    let trie = Trie::open(Box::new(InMemoryTrieDB::new(db_inner)), new_root);

    // Key 0: updated
    let val = trie
        .get(&keys[0])
        .expect("get key 0")
        .expect("should find key 0");
    assert_eq!(val, vec![0xFF; 8], "key 0 should be updated");

    // Key 1: removed
    let val = trie.get(&keys[1]).expect("get key 1");
    assert!(val.is_none(), "key 1 should be removed");

    // Keys 2-9: unchanged
    for i in 2..10usize {
        let val = trie
            .get(&keys[i])
            .unwrap_or_else(|e| panic!("get key {i} failed: {e:?}"));
        assert_eq!(val, Some(vec![i as u8; 8]), "key {i} should be unchanged");
    }
}

/// Many-entry roundtrip test simulating a contract storage trie (like fib test).
/// Uses 30 entries with pseudo-random 32-byte keys to create complex trie structure.
#[test]
fn collect_updates_roundtrip_many_entries() {
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    // Generate 30 pseudo-random 32-byte keys (simulating keccak hashes of storage slots)
    let entries: Vec<([u8; 32], Vec<u8>)> = (0..30u8)
        .map(|i| {
            let mut key = [0u8; 32];
            // Distribute first nibbles to create branches at various levels
            key[0] = i.wrapping_mul(37) ^ 0xA5;
            key[1] = i.wrapping_mul(53) ^ 0x3C;
            key[15] = i;
            key[31] = 0xFF - i;
            // Value: RLP-encoded U256
            let value = vec![i + 1; (i as usize % 4) + 1];
            (key, value)
        })
        .collect();

    for (key, value) in &entries {
        sparse
            .update_leaf(Nibbles::from_bytes(key), value.clone(), &NullProvider)
            .expect("insert");
    }

    let root = sparse.root().expect("root");
    let updates = sparse.collect_updates();

    // Verify root matches old trie
    let mut old_trie = Trie::new_temp();
    for (key, value) in &entries {
        old_trie
            .insert(key.to_vec(), value.clone())
            .expect("insert");
    }
    let old_root = old_trie.hash_no_commit();
    assert_eq!(root, old_root, "roots should match");

    // Store in InMemoryTrieDB and verify retrieval
    let db = InMemoryTrieDB::new_empty();
    db.put_batch(updates).expect("put_batch");
    let trie = Trie::open(Box::new(db), root);

    for (key, expected_value) in &entries {
        let result = trie
            .get(key)
            .unwrap_or_else(|e| panic!("get failed for key {:02x?}: {e:?}", &key[..4]));
        assert_eq!(
            result.as_deref(),
            Some(expected_value.as_slice()),
            "mismatch for key {:02x?}",
            &key[..4]
        );
    }
}

/// Test that extensions spanning the upper/lower subtrie boundary work correctly.
/// A storage trie with only 2 entries whose hashed keys share a common prefix
/// of >= 2 nibbles will have an extension at the root that crosses the boundary.
#[test]
fn extension_spanning_subtrie_boundary() {
    // Create two keys that share the first 3 nibbles (0xa, 0x3, 0x5)
    // but differ at nibble 4. This creates:
    //   Root: Extension(key=[0xa, 0x3, 0x5]) -> Branch -> two leaves
    // The extension is in the upper subtrie (depth 0),
    // but its child (the branch at [0xa, 0x3, 0x5]) is in the lower subtrie (depth 3).
    let mut key1 = [0u8; 32];
    let mut key2 = [0u8; 32];
    // First byte = 0xa3 → nibbles [0xa, 0x3]
    key1[0] = 0xa3;
    key2[0] = 0xa3;
    // Second byte: key1=0x5x, key2=0x5y where x != y
    key1[1] = 0x50; // nibbles [0x5, 0x0]
    key2[1] = 0x51; // nibbles [0x5, 0x1]
    // Rest differs
    key1[31] = 0x01;
    key2[31] = 0x02;

    let value1 = vec![0xAA; 4];
    let value2 = vec![0xBB; 4];

    // Build with SparseTrie
    let mut sparse = SparseTrie::new();
    sparse.upper.nodes.insert(Vec::new(), SparseNode::Empty);
    sparse
        .update_leaf(Nibbles::from_bytes(&key1), value1.clone(), &NullProvider)
        .expect("insert key1");
    sparse
        .update_leaf(Nibbles::from_bytes(&key2), value2.clone(), &NullProvider)
        .expect("insert key2");

    let root = sparse.root().expect("root");

    // Verify root matches old trie
    let mut old_trie = Trie::new_temp();
    old_trie
        .insert(key1.to_vec(), value1.clone())
        .expect("insert");
    old_trie
        .insert(key2.to_vec(), value2.clone())
        .expect("insert");
    let old_root = old_trie.hash_no_commit();
    assert_eq!(root, old_root, "roots must match");

    // Collect updates and verify roundtrip via Trie::get
    let updates = sparse.collect_updates();
    let db = InMemoryTrieDB::new_empty();
    db.put_batch(updates).expect("put");
    let trie = Trie::open(Box::new(db), root);

    let v1 = trie.get(&key1).expect("get key1").expect("key1 must exist");
    assert_eq!(v1, value1);
    let v2 = trie.get(&key2).expect("get key2").expect("key2 must exist");
    assert_eq!(v2, value2);
}

/// Test with encoding comparison: verify that re-encoding revealed nodes
/// produces the exact same bytes as the original.
#[test]
fn encode_roundtrip_matches_original() {
    use crate::db::NodeMap;

    // Step 1: Create a trie with the old code and collect its committed entries
    let entries: Vec<([u8; 32], Vec<u8>)> = (0..15u8)
        .map(|i| {
            let mut key = [0u8; 32];
            key[0] = i.wrapping_mul(37) ^ 0xA5;
            key[1] = i.wrapping_mul(53) ^ 0x3C;
            key[15] = i;
            key[31] = 0xFF - i;
            let value = vec![i + 1; 4];
            (key, value)
        })
        .collect();

    let db_inner: NodeMap = Arc::new(Mutex::new(BTreeMap::new()));
    let db = InMemoryTrieDB::new(db_inner.clone());
    let mut old_trie = Trie::new(Box::new(db));
    for (key, value) in &entries {
        old_trie
            .insert(key.to_vec(), value.clone())
            .expect("insert");
    }
    let old_updates = old_trie.commit_without_storing();
    let old_root = old_trie.hash_no_commit();

    // Store in DB
    let db2 = InMemoryTrieDB::new(db_inner.clone());
    db2.put_batch(old_updates.clone()).expect("put old");

    // Step 2: Create SparseTrie, reveal from same DB, and collect_updates
    let provider = InMemoryTrieDB::new(db_inner.clone());
    let mut sparse = SparseTrie::new();
    sparse.reveal_root(old_root, &provider).expect("reveal");

    // Just re-root (no modifications) and collect
    let sparse_root = sparse.root().expect("root");
    assert_eq!(old_root, sparse_root, "roots must match");
    let sparse_updates = sparse.collect_updates();

    // Step 3: Apply sparse updates ON TOP of the original DB (like TrieLayerCache)
    // collect_updates skips Hash nodes (already in DB), so we layer updates over old data
    let db2_inner: NodeMap = db_inner.clone();
    let db3 = InMemoryTrieDB::new(db2_inner);
    db3.put_batch(sparse_updates.clone()).expect("put sparse");
    let trie = Trie::open(Box::new(db3), sparse_root);

    for (key, expected_value) in &entries {
        let result = trie
            .get(key)
            .unwrap_or_else(|e| panic!("get failed for key {:02x?}: {e:?}", &key[..4]));
        assert_eq!(
            result.as_deref(),
            Some(expected_value.as_slice()),
            "mismatch for key {:02x?}",
            &key[..4]
        );
    }
}

/// Simulate multi-block behavior: block 1 inserts some entries, block 2 reads and modifies.
/// This tests that collect_updates produces a trie that can be re-opened and further modified.
#[test]
fn multi_block_simulation() {
    use crate::db::NodeMap;

    // Block 1: Insert 5 entries
    let entries1: Vec<([u8; 32], Vec<u8>)> = (0..5u8)
        .map(|i| {
            let mut key = [0u8; 32];
            key[0] = i.wrapping_mul(37) ^ 0xA5;
            key[1] = i.wrapping_mul(53) ^ 0x3C;
            key[15] = i;
            key[31] = 0xFF - i;
            let value = vec![i + 1; 4];
            (key, value)
        })
        .collect();

    let mut sparse1 = SparseTrie::new();
    sparse1.upper.nodes.insert(Vec::new(), SparseNode::Empty);

    for (key, value) in &entries1 {
        sparse1
            .update_leaf(Nibbles::from_bytes(key), value.clone(), &NullProvider)
            .expect("insert");
    }

    let root1 = sparse1.root().expect("root1");
    let updates1 = sparse1.collect_updates();

    // Verify block 1 root matches old trie
    let mut old_trie1 = Trie::new_temp();
    for (key, value) in &entries1 {
        old_trie1
            .insert(key.to_vec(), value.clone())
            .expect("insert");
    }
    assert_eq!(
        root1,
        old_trie1.hash_no_commit(),
        "block 1 roots must match"
    );

    // Store block 1 updates in a DB
    let db_inner: NodeMap = Arc::new(Mutex::new(BTreeMap::new()));
    let db1 = InMemoryTrieDB::new(db_inner.clone());
    db1.put_batch(updates1).expect("put block 1");

    // Verify all entries readable from block 1 state
    let trie1 = Trie::open(Box::new(InMemoryTrieDB::new(db_inner.clone())), root1);
    for (key, value) in &entries1 {
        let result = trie1
            .get(key)
            .unwrap_or_else(|e| panic!("block1 get failed for key {:02x?}: {e:?}", &key[..4]));
        assert_eq!(
            result.as_deref(),
            Some(value.as_slice()),
            "block1 mismatch for key {:02x?}",
            &key[..4]
        );
    }

    // Block 2: Open from block 1 state, modify some entries, add new ones
    let provider2 = InMemoryTrieDB::new(db_inner.clone());
    let mut sparse2 = SparseTrie::new();
    sparse2
        .reveal_root(root1, &provider2)
        .expect("reveal root1");

    // Modify entry 0
    let modified_value = vec![0xFF; 4];
    sparse2
        .update_leaf(
            Nibbles::from_bytes(&entries1[0].0),
            modified_value.clone(),
            &provider2,
        )
        .expect("modify entry 0");

    // Add a new entry
    let mut new_key = [0u8; 32];
    new_key[0] = 0xDE;
    new_key[1] = 0xAD;
    new_key[31] = 0x42;
    let new_value = vec![0x42; 4];
    sparse2
        .update_leaf(Nibbles::from_bytes(&new_key), new_value.clone(), &provider2)
        .expect("add new entry");

    // Remove entry 1
    sparse2
        .remove_leaf(Nibbles::from_bytes(&entries1[1].0), &provider2)
        .expect("remove entry 1");

    let root2 = sparse2.root().expect("root2");
    let updates2 = sparse2.collect_updates();

    // Verify block 2 root matches old trie
    let mut old_trie2 = Trie::new_temp();
    for (i, (key, value)) in entries1.iter().enumerate() {
        if i == 1 {
            continue; // removed
        }
        if i == 0 {
            old_trie2
                .insert(key.to_vec(), modified_value.clone())
                .expect("insert");
        } else {
            old_trie2
                .insert(key.to_vec(), value.clone())
                .expect("insert");
        }
    }
    old_trie2
        .insert(new_key.to_vec(), new_value.clone())
        .expect("insert new");
    assert_eq!(
        root2,
        old_trie2.hash_no_commit(),
        "block 2 roots must match"
    );

    // Store block 2 updates LAYERED on top of block 1 DB
    let db2 = InMemoryTrieDB::new(db_inner.clone());
    db2.put_batch(updates2).expect("put block 2");

    // Verify all expected entries readable from block 2 state
    let trie2 = Trie::open(Box::new(InMemoryTrieDB::new(db_inner.clone())), root2);

    // Entry 0: modified
    let result = trie2
        .get(&entries1[0].0)
        .unwrap_or_else(|e| panic!("block2 get entry0 failed: {e:?}"));
    assert_eq!(
        result.as_deref(),
        Some(modified_value.as_slice()),
        "entry0 should be modified"
    );

    // Entry 1: removed
    let result = trie2.get(&entries1[1].0).expect("block2 get entry1");
    assert!(result.is_none(), "entry1 should be removed");

    // Entries 2-4: unchanged
    for (key, value) in &entries1[2..] {
        let result = trie2.get(key).unwrap_or_else(|e| {
            panic!("block2 get unchanged key {:02x?} failed: {e:?}", &key[..4])
        });
        assert_eq!(
            result.as_deref(),
            Some(value.as_slice()),
            "block2 unchanged mismatch for key {:02x?}",
            &key[..4]
        );
    }

    // New entry
    let result = trie2
        .get(&new_key)
        .unwrap_or_else(|e| panic!("block2 get new key failed: {e:?}"));
    assert_eq!(
        result.as_deref(),
        Some(new_value.as_slice()),
        "new entry should exist"
    );
}
