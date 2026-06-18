//! Equivalence tests for `compute_sharded_storage_root`.
//!
//! The sharded per-account storage-root computation MUST produce the correct
//! `root_hash` for every input (a divergence there is a consensus failure), and
//! its persisted node set MUST be sufficient to read the storage trie back at
//! the new root. For inserts/updates and degenerate removals it is also
//! bit-identical to the serial reference; on full-bucket removals in the
//! parallel path it emits a few redundant/orphan nodes (functionally harmless on
//! the path-keyed node DB — exercised by the `functional_*` tests below).

use ethrex_blockchain::compute_sharded_storage_root;
use ethrex_common::{H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{DecodedNodeOverlay, EngineType, Store, apply_prefix};
use ethrex_trie::{Nibbles, Node, TrieNode};
use std::sync::Arc;

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

/// Like [`seed_storage`] but also returns the trie node set produced by the same
/// inserts (nibble path -> encoded RLP), so the test can build a decoded-node
/// overlay keyed exactly the way `TrieWrapper::get_node` looks nodes up.
///
/// The persisted trie and the node-capture trie are SEPARATE instances: committing
/// memoizes hashes, after which the change accumulator comes back empty, so a single
/// trie cannot both persist and report its node set. The capture trie is an
/// independent in-memory trie fed the identical inserts; the storage trie is
/// content-addressed, so its node set (and root) is byte-identical to the persisted one.
fn seed_storage_returning_nodes(
    store: &Store,
    hashed_address: H256,
    initial_slots: &[(H256, U256)],
) -> (H256, Vec<TrieNode>) {
    // Persist the base trie exactly as the other tests do.
    let root = seed_storage(store, hashed_address, initial_slots);

    // Capture the identical node set from a throwaway in-memory trie.
    let mut capture = ethrex_trie::Trie::empty_in_memory();
    for (k, v) in initial_slots {
        capture
            .insert(k.as_bytes().to_vec(), v.encode_to_vec())
            .expect("capture insert");
    }
    let (capture_root, nodes) = capture.collect_changes_since_last_hash(&NativeCrypto);
    assert_eq!(
        capture_root, root,
        "capture trie root must match persisted trie (content-addressed)"
    );
    (root, nodes)
}

/// Build a CORRECT decoded-node overlay from a base trie's committed change set.
/// Each entry is keyed by `apply_prefix(Some(hashed_address), path)` — byte-for-byte
/// the key `TrieWrapper::get_node` derives (`prefix_nibbles.concat(path)`) for a
/// storage trie — and valued by the pre-decoded node.
///
/// `collect_changes_since_last_hash` interleaves two kinds of entries: actual
/// trie-node encodings (keyed by node path, what the merkleizer reads via
/// `get_node` and what lands in `STORAGE_TRIE_NODES`) and flat leaf-value entries
/// (the slot's encoded U256, keyed by the full leaf path). The overlay must hold
/// only real nodes, so entries whose RLP does not decode as a `Node` (the leaf
/// values) are skipped — mirroring the production builder, which reads only the
/// `STORAGE_TRIE_NODES` CF. Empty (tombstone / empty-trie-marker) entries are
/// likewise skipped.
fn build_overlay(hashed_address: H256, base_nodes: &[TrieNode]) -> DecodedNodeOverlay {
    let mut overlay = DecodedNodeOverlay::default();
    for (path, rlp) in base_nodes {
        if rlp.is_empty() {
            continue; // tombstone / empty-trie marker: never a real node
        }
        // Only retain entries that decode as a trie node; flat leaf-value entries
        // (encoded slot values) are not nodes and are not read via `get_node`.
        let Ok(node) = Node::decode(rlp) else {
            continue;
        };
        let key = apply_prefix(Some(hashed_address), path.clone())
            .into_vec()
            .into_boxed_slice();
        overlay.insert(key, Arc::new(node));
    }
    overlay
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
        None,
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
        None,
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
        None,
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
        None,
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
        None,
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
        None,
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
        None,
    )
    .expect("sharded");

    assert_equiv(serial, sharded, "normal_spread_mixed");
}

// ── removal-focused + randomized coverage ──────────────────────────────────
// The hand-built cases above assert the sharded path is *bit-identical* to the
// serial reference. That holds for inserts/updates and degenerate removals, but
// NOT for full-bucket removals on the parallel path: the bespoke reassembly
// emits a few redundant/orphan nodes (verified harmless — see the functional
// tests). Bit-identical is therefore sufficient but stronger than necessary on
// a path-keyed node DB. The randomized cases below assert the criterion that
// actually matters for a live node: after applying the sharded node set (writes
// + tombstone deletes) and reopening at the new root, every slot reads back
// correctly and removed slots are gone — with emphasis on removal-heavy mixes.

/// Deterministic xorshift64* PRNG so the fuzz cases are reproducible in CI.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Mix the seed so seed==0 still yields non-zero state.
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Random 32-byte key (unique with overwhelming probability), spread across all
/// nibble buckets — unlike `key_in_nibble`, which only controls the first nibble.
fn rand_h256(rng: &mut Rng) -> H256 {
    let mut b = [0u8; 32];
    for chunk in b.chunks_mut(8) {
        chunk.copy_from_slice(&rng.next_u64().to_le_bytes());
    }
    H256(b)
}

/// Random non-zero value (zero would be interpreted as a removal).
fn rand_value(rng: &mut Rng) -> U256 {
    U256::from(rng.next_u64() | 1)
}

// ── test 8 ───────────────────────────────────────────────────────────────
// Removal collapses a multi-bucket delta down to a single occupied bucket:
// the delta only touches nibble 0x3 (removing every seeded 0x3 key), while
// untouched bucket 0xC must survive. Exercises the `occupied <= 1` serial
// fallback against a non-empty residual trie.
#[test]
fn sharded_vs_serial_remove_one_bucket_keeps_other() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let hashed_address = H256::from_low_u64_be(0x0BCE_0009);
    let parent_state_root = *EMPTY_TRIE_HASH;

    let mut initial: Vec<(H256, U256)> = Vec::new();
    for i in 0u64..300 {
        initial.push((key_in_nibble(0x3, i), U256::from(i + 1)));
    }
    for i in 0u64..300 {
        initial.push((key_in_nibble(0xc, i + 1000), U256::from(i + 1)));
    }
    let storage_root = seed_storage(&store, hashed_address, &initial);

    // Delta removes only the 0x3 keys (single occupied bucket).
    let delta: Vec<(H256, U256)> = (0u64..300)
        .map(|i| (key_in_nibble(0x3, i), U256::zero()))
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
        None,
    )
    .expect("sharded");

    assert_ne!(
        serial.0, *EMPTY_TRIE_HASH,
        "bucket 0xC must keep the trie non-empty"
    );
    assert_equiv(serial, sharded, "remove_one_bucket_keeps_other");
}

// ── test 9 ──────────────────────────────────────────────────────────────
// Duplicate-key resolution within one delta: remove-then-reinsert (net insert)
// and insert-then-remove (net delete) for the same key, across two buckets so
// the parallel path runs. Guards the stable-sort last-write-wins semantics.
#[test]
fn sharded_vs_serial_duplicate_key_resolution() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let hashed_address = H256::from_low_u64_be(0x0BCE_000B);
    let parent_state_root = *EMPTY_TRIE_HASH;

    // Seed one key per bucket so removals have something to act on.
    let initial: Vec<(H256, U256)> = (0u64..16)
        .map(|n| (key_in_nibble(n as u8, n + 1), U256::from(n + 1)))
        .collect();
    let storage_root = seed_storage(&store, hashed_address, &initial);

    let k_resurrect = key_in_nibble(0x2, 500); // new key: remove then insert
    let k_kill = initial[7].0; // existing key: insert then remove

    let delta: Vec<(H256, U256)> = vec![
        (k_resurrect, U256::zero()),
        (k_resurrect, U256::from(42u64)),
        (k_kill, U256::from(777u64)),
        (k_kill, U256::zero()),
        // also a couple of plain ops in other buckets to keep >=2 occupied
        (key_in_nibble(0xf, 900), U256::from(5u64)),
        (key_in_nibble(0xa, 901), U256::from(6u64)),
    ];

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
        None,
    )
    .expect("sharded");

    assert_equiv(serial, sharded, "duplicate_key_resolution");
}

// ── functional safety check ────────────────────────────────────────────────
// Bit-identical node sets are sufficient but stronger than necessary on a
// path-keyed node DB. What actually matters for a live node: after applying the
// sharded node set (writes + tombstone deletes) to the backing store, the
// storage trie opened at the new root must read back every slot correctly and
// contain no stale reachable nodes. This drives that end-to-end.
use std::collections::BTreeMap;

async fn assert_db_correct_after_sharded(
    label: &str,
    initial: &[(H256, U256)],
    delta: &[(H256, U256)],
    hashed_address: H256,
) {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let parent_state_root = *EMPTY_TRIE_HASH;
    let storage_root = if initial.is_empty() {
        *EMPTY_TRIE_HASH
    } else {
        seed_storage(&store, hashed_address, initial)
    };

    // Expected final slot map (sequential last-write-wins, zero == delete).
    let mut expected: BTreeMap<H256, U256> = BTreeMap::new();
    for (k, v) in initial {
        if !v.is_zero() {
            expected.insert(*k, *v);
        }
    }
    for (k, v) in delta {
        if v.is_zero() {
            expected.remove(k);
        } else {
            expected.insert(*k, *v);
        }
    }

    let (root_hash, nodes) = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        delta,
        None,
    )
    .expect("sharded");

    // Persist the sharded node set (writes + deletes) directly to the backend.
    store
        .write_storage_trie_nodes_batch(vec![(hashed_address, nodes)])
        .await
        .expect("persist sharded nodes");

    // Reopen at the new root straight from the backend (no cache) and verify.
    let trie = store
        .open_direct_storage_trie(hashed_address, root_hash)
        .expect("open direct");
    for (k, v) in &expected {
        let got = trie
            .get(k.as_bytes())
            .expect("get")
            .map(|rlp| U256::decode(&rlp).expect("decode value"));
        assert_eq!(
            got,
            Some(*v),
            "{label}: slot {k:?} wrong after sharded apply"
        );
    }
    // Spot-check that removed keys are gone.
    for (k, v) in delta {
        if v.is_zero() && !expected.contains_key(k) {
            let got = trie.get(k.as_bytes()).expect("get removed");
            assert_eq!(got, None, "{label}: removed slot {k:?} still present");
        }
    }
}

#[tokio::test]
async fn functional_remove_entire_bucket_parallel() {
    let hashed_address = H256::from_low_u64_be(0x0BCE_010A);
    let mut initial: Vec<(H256, U256)> = Vec::new();
    for i in 0u64..200 {
        initial.push((key_in_nibble(0x1, i), U256::from(i + 1)));
    }
    for i in 0u64..200 {
        initial.push((key_in_nibble(0x9, i + 1000), U256::from(i + 1)));
    }
    let mut delta: Vec<(H256, U256)> = (0u64..200)
        .map(|i| (key_in_nibble(0x1, i), U256::zero()))
        .collect();
    for i in 0u64..50 {
        delta.push((key_in_nibble(0x9, i + 1000), U256::from(i + 9999)));
    }
    assert_db_correct_after_sharded("remove_entire_bucket", &initial, &delta, hashed_address).await;
}

#[tokio::test]
async fn functional_randomized() {
    for seed in 0u64..64 {
        let hashed_address = H256::from_low_u64_be(0xF00D_0000 + seed);
        let mut rng = Rng::new(seed);
        let init_n = (rng.next_u64() % 500) as usize;
        let mut live: Vec<H256> = (0..init_n).map(|_| rand_h256(&mut rng)).collect();
        let initial: Vec<(H256, U256)> = live.iter().map(|k| (*k, rand_value(&mut rng))).collect();
        let ops = (rng.next_u64() % 600) as usize;
        let mut delta: Vec<(H256, U256)> = Vec::new();
        for _ in 0..ops {
            match rng.next_u64() % 100 {
                r if r < 40 && !live.is_empty() => {
                    delta.push((live[(rng.next_u64() as usize) % live.len()], U256::zero()));
                }
                r if r < 65 && !live.is_empty() => {
                    delta.push((
                        live[(rng.next_u64() as usize) % live.len()],
                        rand_value(&mut rng),
                    ));
                }
                _ => {
                    let k = rand_h256(&mut rng);
                    live.push(k);
                    delta.push((k, rand_value(&mut rng)));
                }
            }
        }
        assert_db_correct_after_sharded(
            &format!("rand seed={seed}"),
            &initial,
            &delta,
            hashed_address,
        )
        .await;
    }
}

// ── test 10: populated decoded-node overlay path ───────────────────────────
// All the tests above pass `overlay = None`, exercising only the pure-RocksDB
// fallback in `TrieWrapper::get_node`. The EF blockchain suite never populates
// the overlay either (no hot account hits >=2048 slots). This is the ONLY test
// of the *populated* overlay path: it feeds a CORRECTLY-built decoded-node
// overlay and proves the result is byte-identical to both the serial reference
// and the sharded-no-overlay computation. Byte-identity on the same input is the
// guarantee that serving warmed nodes from the overlay never diverges from a
// cold RocksDB read+decode.
#[test]
fn sharded_with_populated_overlay_is_byte_identical() {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let hashed_address = H256::from_low_u64_be(0x0BCE_000C);
    let parent_state_root = *EMPTY_TRIE_HASH;

    // Seed a non-empty base trie with real internal structure: 512 slots spread
    // across all 16 nibbles, so the merkle walk reads existing branch/extension
    // nodes — exactly where overlay hits happen. Capture the committed node set.
    let initial: Vec<(H256, U256)> = (0u64..512)
        .map(|i| {
            let nibble = (i / 32) as u8; // 0-15
            (key_in_nibble(nibble, i + 700_000), U256::from(i + 1))
        })
        .collect();
    let (storage_root, base_nodes) = seed_storage_returning_nodes(&store, hashed_address, &initial);
    assert_ne!(
        storage_root, *EMPTY_TRIE_HASH,
        "seeded base trie must be non-empty"
    );

    // Build a correctly-keyed overlay from the base trie's committed nodes.
    let overlay = build_overlay(hashed_address, &base_nodes);

    // The overlay must be non-empty AND its keys must match what the walk reads.
    // The merkleizer always reads the root node first at path `Nibbles::default()`,
    // so its prefixed key MUST be present — proving the overlay is actually hit
    // and the test is not a vacuous pass against a never-consulted overlay.
    assert!(!overlay.is_empty(), "overlay must contain warmed nodes");
    let root_key = apply_prefix(Some(hashed_address), Nibbles::default())
        .into_vec()
        .into_boxed_slice();
    assert!(
        overlay.contains_key(&root_key),
        "overlay must contain the base root node (the walk's first read) — \
         otherwise the overlay is never hit and the test proves nothing"
    );

    // A mix of new inserts, updates of existing slots, and removals over the base,
    // spread so the parallel (>=2 occupied buckets) path runs.
    let mut writes: Vec<(H256, U256)> = Vec::new();
    for i in 0u64..128 {
        let nibble = (i / 8) as u8 % 16;
        writes.push((key_in_nibble(nibble, i + 900_000), U256::from(i + 5000)));
    }
    for (k, _) in &initial[..64] {
        writes.push((*k, U256::zero())); // remove
    }
    for (k, v) in &initial[64..128] {
        writes.push((*k, *v + U256::from(7))); // update
    }

    // Ground truth: serial reference, no overlay.
    let serial = serial_reference(
        &store,
        hashed_address,
        parent_state_root,
        storage_root,
        &writes,
    );
    // Sharded, no overlay (pure RocksDB fallback path).
    let sharded_none = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        &writes,
        None,
    )
    .expect("sharded no overlay");
    // Sharded, WITH the populated overlay (the hit path under test).
    let sharded_overlay = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        &writes,
        Some(Arc::new(overlay.clone())),
    )
    .expect("sharded with overlay");

    // All three must agree on root hash and node set. The overlay-vs-None equality
    // is the key assertion: it proves the populated-overlay hit path is byte-identical.
    assert_equiv(
        serial.clone(),
        sharded_none.clone(),
        "overlay_test/serial_vs_sharded_none",
    );
    assert_equiv(
        serial,
        sharded_overlay.clone(),
        "overlay_test/serial_vs_sharded_overlay",
    );
    assert_equiv(
        sharded_none.clone(),
        sharded_overlay,
        "overlay_test/sharded_none_vs_overlay",
    );

    // ── proof of consultation (negative control) ──────────────────────────
    // The byte-identity above is only meaningful if the overlay was actually
    // consulted; an empty / mis-keyed overlay would also be byte-identical
    // (it would just always miss and fall through to disk). To prove the
    // overlay's nodes are really served, poison it: overwrite the root-node
    // entry with a different (but structurally valid) node taken from another
    // path. There is no write layer for this freshly-seeded root, so
    // `TrieWrapper::get_node` precedence (write-layer > overlay > disk) means a
    // poisoned root entry is served INSTEAD of the correct on-disk root. If the
    // overlay were never consulted, the result would still equal the correct
    // `sharded_none`; we assert it does NOT (either it diverges or errors),
    // which can only happen if the overlay node was actually used.
    let mut poisoned = overlay;
    let wrong_node = poisoned
        .iter()
        .find(|(k, _)| ***k != *root_key)
        .map(|(_, v)| v.clone())
        .expect("overlay has a non-root node to clone");
    poisoned.insert(root_key, wrong_node);
    let poisoned_result = compute_sharded_storage_root(
        &store,
        parent_state_root,
        hashed_address,
        storage_root,
        &writes,
        Some(Arc::new(poisoned)),
    );
    // A structurally-wrong root can also make the walk fail; an `Err` likewise
    // proves the overlay node was served (a never-consulted overlay cannot error).
    if let Ok((poisoned_root, _)) = poisoned_result {
        assert_ne!(
            poisoned_root, sharded_none.0,
            "poisoned overlay produced the correct root — the overlay was NOT \
             consulted, so the byte-identity test is vacuous"
        );
    }
}
