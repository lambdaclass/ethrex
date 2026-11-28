//! Equivalence gate for [`Trie::prefetch_sorted`] (breadth-first batched
//! arena pre-resolution, design A2): prefetching must be transparent to the
//! resulting root hash and to the persisted node set. `prefetch_sorted` only
//! converts already-on-disk `NodeRef::Hash` nodes into `NodeRef::Node`,
//! preserving their memoized hash, so a serial insert loop run with or
//! without a prior `prefetch_sorted` call over the same batch must produce a
//! byte-identical root and an identical set of collected node bytes.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ethereum_types::H256;
use ethrex_crypto::NativeCrypto;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_trie::db::{InMemoryTrieDB, NodeMap, TrieDB};
use ethrex_trie::{Nibbles, Node, Trie, TrieError};

/// First-batch size: large/varied enough to build real branch+extension
/// structure (not just a single leaf), so prefetch_sorted's Branch/Extension
/// routing arms actually get exercised.
const BASE_SLOTS: u64 = 2_000;
/// Second batch: the slots whose insertion `prefetch_sorted` should
/// accelerate.
const BATCH_SLOTS: u64 = 2_000;

fn base_key(i: u64) -> Vec<u8> {
    keccak_hash([b"base".as_ref(), &i.to_be_bytes()].concat()).to_vec()
}
fn base_value(i: u64) -> Vec<u8> {
    keccak_hash([b"base-val".as_ref(), &i.to_be_bytes()].concat()).to_vec()
}
fn batch_key(i: u64) -> Vec<u8> {
    keccak_hash([b"batch".as_ref(), &i.to_be_bytes()].concat()).to_vec()
}
fn batch_value(i: u64) -> Vec<u8> {
    keccak_hash([b"batch-val".as_ref(), &i.to_be_bytes()].concat()).to_vec()
}

/// Builds and commits the base trie (`BASE_SLOTS` pseudo-random slots),
/// returning its root hash and the committed node map backing it.
fn build_base() -> (H256, NodeMap) {
    let map: NodeMap = Arc::new(Mutex::new(BTreeMap::new()));
    let mut trie = Trie::new(Box::new(InMemoryTrieDB::new(map.clone())));
    for i in 0..BASE_SLOTS {
        trie.insert(base_key(i), base_value(i)).unwrap();
    }

    // Sanity canary: the base trie must have real internal structure (a
    // Branch root), not just a single leaf, or this test wouldn't exercise
    // prefetch_sorted's branch/extension routing at all.
    assert!(
        matches!(trie.root_node().unwrap().as_deref(), Some(Node::Branch(_))),
        "expected a Branch root; the test no longer builds real trie structure"
    );

    let root = trie.hash(&NativeCrypto).unwrap();
    (root, map)
}

/// Deep-clones a `NodeMap`'s contents into a fresh, independent backing map.
fn clone_map(map: &NodeMap) -> NodeMap {
    Arc::new(Mutex::new(map.lock().unwrap().clone()))
}

#[test]
fn prefetch_sorted_is_byte_identical_to_serial_insert() {
    let (base_root, base_map) = build_base();

    let sorted_paths: Vec<Nibbles> = {
        let mut paths: Vec<Nibbles> = (0..BATCH_SLOTS)
            .map(|i| Nibbles::from_bytes(&batch_key(i)))
            .collect();
        paths.sort();
        paths
    };

    // R1: serial insert loop only, no prefetch.
    let mut trie_r1 = Trie::open(
        Box::new(InMemoryTrieDB::new(clone_map(&base_map))),
        base_root,
    );
    for i in 0..BATCH_SLOTS {
        trie_r1.insert(batch_key(i), batch_value(i)).unwrap();
    }
    let (root_r1, mut nodes_r1) = trie_r1.collect_changes_since_last_hash(&NativeCrypto);

    // R2: prefetch_sorted over the same batch, then the identical serial
    // insert loop, against a fresh copy of the identical base DB.
    let mut trie_r2 = Trie::open(
        Box::new(InMemoryTrieDB::new(clone_map(&base_map))),
        base_root,
    );
    trie_r2.prefetch_sorted(&sorted_paths).unwrap();
    for i in 0..BATCH_SLOTS {
        trie_r2.insert(batch_key(i), batch_value(i)).unwrap();
    }
    let (root_r2, mut nodes_r2) = trie_r2.collect_changes_since_last_hash(&NativeCrypto);

    assert_eq!(
        root_r1, root_r2,
        "prefetch_sorted changed the root hash: r1={root_r1:#x} r2={root_r2:#x}"
    );

    nodes_r1.sort();
    nodes_r2.sort();
    assert_eq!(
        nodes_r1, nodes_r2,
        "prefetch_sorted changed the collected node set"
    );
}

/// Counts the keys a batched read asks for, so a test can tell whether
/// `prefetch_sorted` fetched anything.
struct CountingDB {
    inner: InMemoryTrieDB,
    multi_get_keys: Arc<AtomicUsize>,
}

impl TrieDB for CountingDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        self.inner.get(key)
    }
    fn multi_get(&self, keys: &[Nibbles]) -> Vec<Result<Option<Vec<u8>>, TrieError>> {
        self.multi_get_keys.fetch_add(keys.len(), Ordering::Relaxed);
        self.inner.multi_get(keys)
    }
    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        self.inner.put_batch(key_values)
    }
}

/// A root already in memory (here: resolved and modified by a first batch of
/// inserts) must still be prefetched below, and stay transparent to the root
/// hash and to the collected node set.
#[test]
fn prefetch_sorted_descends_from_an_in_memory_root() {
    let (base_root, base_map) = build_base();
    let first_batch = 0..BATCH_SLOTS / 2;
    let second_batch = BATCH_SLOTS / 2..BATCH_SLOTS;

    let sorted_paths: Vec<Nibbles> = {
        let mut paths: Vec<Nibbles> = second_batch
            .clone()
            .map(|i| Nibbles::from_bytes(&batch_key(i)))
            .collect();
        paths.sort();
        paths
    };

    // R1: both batches inserted serially, no prefetch.
    let mut trie_r1 = Trie::open(
        Box::new(InMemoryTrieDB::new(clone_map(&base_map))),
        base_root,
    );
    for i in 0..BATCH_SLOTS {
        trie_r1.insert(batch_key(i), batch_value(i)).unwrap();
    }
    let (root_r1, mut nodes_r1) = trie_r1.collect_changes_since_last_hash(&NativeCrypto);

    // R2: the first batch resolves the root into memory, then prefetch the
    // second batch before inserting it.
    let multi_get_keys = Arc::new(AtomicUsize::new(0));
    let mut trie_r2 = Trie::open(
        Box::new(CountingDB {
            inner: InMemoryTrieDB::new(clone_map(&base_map)),
            multi_get_keys: multi_get_keys.clone(),
        }),
        base_root,
    );
    for i in first_batch {
        trie_r2.insert(batch_key(i), batch_value(i)).unwrap();
    }
    trie_r2.prefetch_sorted(&sorted_paths).unwrap();
    assert!(
        multi_get_keys.load(Ordering::Relaxed) > 0,
        "prefetch_sorted fetched nothing below an in-memory root"
    );
    for i in second_batch {
        trie_r2.insert(batch_key(i), batch_value(i)).unwrap();
    }
    let (root_r2, mut nodes_r2) = trie_r2.collect_changes_since_last_hash(&NativeCrypto);

    assert_eq!(
        root_r1, root_r2,
        "prefetch_sorted changed the root hash: r1={root_r1:#x} r2={root_r2:#x}"
    );

    nodes_r1.sort();
    nodes_r2.sort();
    assert_eq!(
        nodes_r1, nodes_r2,
        "prefetch_sorted changed the collected node set"
    );
}
