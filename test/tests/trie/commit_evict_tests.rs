//! Proves that [`Trie::commit_evict_below_root`] (the bulk-load arena-eviction
//! helper) is transparent to the result: building the same key/value set with
//! periodic plain `commit` and with periodic `commit_evict_below_root` yields
//! the same root hash and the same persisted node set.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ethrex_crypto::NativeCrypto;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_trie::db::{InMemoryTrieDB, NodeMap};
use ethrex_trie::{Node, Trie};

/// Number of key/value pairs inserted per build. Kept modest so the test stays
/// CI-fast while still spreading keys across all 16 root branches.
const N: u64 = 200_000;
/// Commit/eviction chunk. Small enough that eviction happens many times.
const CHUNK: u64 = 10_000;

/// 32-byte, well-spread key derived deterministically from the index.
fn key(index: u64) -> Vec<u8> {
    keccak_hash(index.to_be_bytes()).to_vec()
}

/// Non-empty value derived deterministically from the index.
fn value(index: u64) -> Vec<u8> {
    keccak_hash([b"val".as_ref(), &index.to_be_bytes()].concat()).to_vec()
}

/// Build a trie over a fresh in-memory direct backend, committing every
/// `CHUNK` inserts. When `evict` is set, the committed subtree is evicted from
/// memory after every commit. Returns the persisted node map (so two builds can
/// be compared byte-for-byte) alongside the trie.
fn build(evict: bool) -> (NodeMap, Trie) {
    let map: NodeMap = Arc::new(Mutex::new(BTreeMap::new()));
    let mut trie = Trie::new(Box::new(InMemoryTrieDB::new(map.clone())));

    for k in 0..N {
        trie.insert(key(k), value(k)).unwrap();
        if (k + 1) % CHUNK == 0 {
            if evict {
                trie.commit_evict_below_root(&NativeCrypto).unwrap();
            } else {
                trie.commit(&NativeCrypto).unwrap();
            }
        }
    }
    // Final flush of any remainder.
    if evict {
        trie.commit_evict_below_root(&NativeCrypto).unwrap();
    } else {
        trie.commit(&NativeCrypto).unwrap();
    }

    (map, trie)
}

/// Eviction must not change the resulting root hash nor the persisted node set.
#[test]
fn commit_evict_below_root_is_transparent() {
    let (plain_map, mut plain_trie) = build(false);
    let (evict_map, mut evict_trie) = build(true);

    let plain_root = plain_trie.hash(&NativeCrypto).unwrap();
    let evict_root = evict_trie.hash(&NativeCrypto).unwrap();

    assert_eq!(
        plain_root, evict_root,
        "eviction changed the root hash: plain={plain_root:#x} evict={evict_root:#x}"
    );

    let plain_nodes = plain_map.lock().unwrap();
    let evict_nodes = evict_map.lock().unwrap();
    assert_eq!(
        *plain_nodes,
        *evict_nodes,
        "eviction changed the persisted node set (len plain={} evict={})",
        plain_nodes.len(),
        evict_nodes.len()
    );
}

/// Keys sharing a long common prefix so the trie root is an `Extension` node
/// (not a `Branch`), exercising `commit_evict_below_root`'s Extension arm which
/// the random-key build above never hits.
fn shared_prefix_key(index: u64) -> Vec<u8> {
    // 24-byte fixed prefix + 8 varying bytes → all keys share the first 48
    // nibbles, forcing an extension at the root.
    let mut k = vec![0xAB; 24];
    k.extend_from_slice(&index.to_be_bytes());
    k
}

fn build_shared_prefix(evict: bool) -> (NodeMap, Trie) {
    let map: NodeMap = Arc::new(Mutex::new(BTreeMap::new()));
    let mut trie = Trie::new(Box::new(InMemoryTrieDB::new(map.clone())));
    let n: u64 = 5_000;
    let chunk: u64 = 500;
    for k in 0..n {
        trie.insert(shared_prefix_key(k), value(k)).unwrap();
        if (k + 1) % chunk == 0 {
            if evict {
                trie.commit_evict_below_root(&NativeCrypto).unwrap();
            } else {
                trie.commit(&NativeCrypto).unwrap();
            }
        }
    }
    (map, trie)
}

/// Same transparency guarantee for an Extension-rooted trie (covers the
/// `Node::Extension` demotion arm).
#[test]
fn commit_evict_below_root_extension_root_is_transparent() {
    let (plain_map, mut plain_trie) = build_shared_prefix(false);
    let (evict_map, mut evict_trie) = build_shared_prefix(true);

    // Canary: confirm we actually built an Extension root, so this test keeps
    // exercising the arm it targets even if the trie shape changes later.
    assert!(
        matches!(
            plain_trie.root_node().unwrap().as_deref(),
            Some(Node::Extension(_))
        ),
        "expected an Extension root; the test no longer covers the Extension arm"
    );

    assert_eq!(
        plain_trie.hash(&NativeCrypto).unwrap(),
        evict_trie.hash(&NativeCrypto).unwrap(),
        "eviction changed the root hash for an Extension-rooted trie"
    );
    assert_eq!(
        *plain_map.lock().unwrap(),
        *evict_map.lock().unwrap(),
        "eviction changed the persisted node set for an Extension-rooted trie"
    );
}

/// After eviction the values must still be reachable through the trie, i.e. the
/// demoted subtrees are correctly re-read from the direct backend on lookup.
#[test]
fn commit_evict_below_root_preserves_reads() {
    let (_, trie) = build(true);
    for k in (0..N).step_by((N / 50) as usize) {
        let got = trie.get(&key(k)).unwrap();
        assert_eq!(
            got,
            Some(value(k)),
            "value for key {k} not readable after eviction"
        );
    }
}
