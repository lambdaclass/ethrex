//! Correctness gate for the parallel mega storage-trie build
//! ([`ethrex_blockchain::build_sharded_storage_trie_streaming`]).
//!
//! - **Equivalence**: the 16-shard streaming build yields the exact same storage
//!   root as the serial `open_direct_storage_trie` + `commit_evict_below_root`
//!   build over the same slots.
//! - **Persistence/reachability**: reopening the trie at the sharded root and
//!   reading every slot back from disk returns the correct value — proving every
//!   node the root references was actually persisted (a wrong-but-consistent root
//!   would pass equivalence yet fail these reads).
//! - **Root-collision race**: with a tiny chunk (many evict cycles → many
//!   interleaved partial-root writes to the empty path) the sharded root stays
//!   correct across repeated runs — the assembler's final root must always win.

use std::path::PathBuf;

use ethrex_blockchain::build_sharded_storage_trie_streaming;
use ethrex_common::{H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::error::StoreError;
use ethrex_storage::{EngineType, Store};
use ethrex_trie::EMPTY_TRIE_HASH;

fn scratch_base() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let cache = PathBuf::from(home).join(".cache").join("tmp");
        if std::fs::create_dir_all(&cache).is_ok() {
            return cache;
        }
    }
    std::env::temp_dir()
}

fn new_store(dir: &tempfile::TempDir) -> Store {
    Store::new(dir.path().to_str().unwrap(), EngineType::RocksDB).unwrap()
}

/// Deterministic, keccak-spread slot: (hashed key, RLP value). keccak keys land
/// across all 16 root nibbles, so the parallel path (>= all-16 occupied) engages.
fn derive(k: u64) -> (H256, Vec<u8>) {
    let hashed = H256(keccak_hash([b"k".as_ref(), &k.to_be_bytes()].concat()));
    let value = U256::from_big_endian(&keccak_hash([b"v".as_ref(), &k.to_be_bytes()].concat()))
        + U256::one();
    (hashed, value.encode_to_vec())
}

/// Serial reference build: the non-sharded path the sharded build must match.
fn serial_root(store: &Store, account: H256, n: u64, chunk: u64) -> H256 {
    let mut trie = store
        .open_direct_storage_trie(account, *EMPTY_TRIE_HASH)
        .unwrap();
    for k in 0..n {
        let (hashed, value_rlp) = derive(k);
        trie.insert(hashed.as_bytes().to_vec(), value_rlp).unwrap();
        if (k + 1) % chunk == 0 {
            trie.commit_evict_below_root(&NativeCrypto).unwrap();
            trie.clear_dirty();
        }
    }
    trie.commit_evict_below_root(&NativeCrypto).unwrap();
    trie.hash_no_commit(&NativeCrypto)
}

fn sharded_root(store: &Store, account: H256, n: u64, chunk: u64) -> H256 {
    let noop = |_: &H256, _: &[u8]| -> Result<(), StoreError> { Ok(()) };
    build_sharded_storage_trie_streaming(store, account, n, chunk, derive, noop).unwrap()
}

#[test]
fn sharded_matches_serial_and_persists() {
    const N: u64 = 50_000;
    const CHUNK: u64 = 1_000;
    let account = H256(keccak_hash(b"mega-account"));

    let dir_a = tempfile::tempdir_in(scratch_base()).unwrap();
    let dir_b = tempfile::tempdir_in(scratch_base()).unwrap();
    let store_a = new_store(&dir_a);
    let store_b = new_store(&dir_b);

    let root_serial = serial_root(&store_a, account, N, CHUNK);
    let root_sharded = sharded_root(&store_b, account, N, CHUNK);

    assert_eq!(
        root_serial, root_sharded,
        "sharded root {root_sharded:#x} != serial root {root_serial:#x}"
    );
    assert_ne!(root_sharded, *EMPTY_TRIE_HASH, "root should be non-empty");

    // Reopen at the sharded root and read every slot from disk: forces traversal
    // through every persisted node, so a missing/wrong node fails here.
    let trie = store_b
        .open_direct_storage_trie(account, root_sharded)
        .unwrap();
    for k in 0..N {
        let (hashed, value_rlp) = derive(k);
        assert_eq!(
            trie.get(hashed.as_bytes()).unwrap(),
            Some(value_rlp),
            "slot {k} not readable from disk after sharded build"
        );
    }
}

#[test]
fn sharded_root_stable_under_tiny_chunk_stress() {
    // Tiny chunk => many evict cycles => many interleaved partial-root writes to
    // the empty path, maximizing exposure to the root-collision race.
    const N: u64 = 20_000;
    const CHUNK: u64 = 200;
    const ITERS: usize = 20;
    let account = H256(keccak_hash(b"stress-account"));

    let ref_dir = tempfile::tempdir_in(scratch_base()).unwrap();
    let expected = serial_root(&new_store(&ref_dir), account, N, CHUNK);

    for i in 0..ITERS {
        let dir = tempfile::tempdir_in(scratch_base()).unwrap();
        let got = sharded_root(&new_store(&dir), account, N, CHUNK);
        assert_eq!(
            got, expected,
            "iteration {i}: sharded root {got:#x} != {expected:#x} (root-collision race?)"
        );
    }
}
