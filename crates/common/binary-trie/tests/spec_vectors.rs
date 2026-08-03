//! Conformance tests against vectors generated from the EELS
//! reference implementation. The fixture is vendored from
//! execution-specs `tests/binary_trie/vectors/`, which owns the
//! generator; see this crate's README for how to refresh it.

use std::collections::BTreeMap;

use ethereum_types::{H160, U256};
use ethrex_binary_trie::embedding;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    trie_roots: Vec<TrieCase>,
    embedding: EmbeddingVectors,
    chunkify_code: Vec<ChunkifyCase>,
    encode_basic_data: Vec<BasicDataCase>,
}

#[derive(Deserialize)]
struct ChunkifyCase {
    name: String,
    code: String,
    chunks: Vec<String>,
}

#[derive(Deserialize)]
struct BasicDataCase {
    code_size: u32,
    nonce: u64,
    /// Hex string; balances can exceed `u64`.
    balance: String,
    encoded: String,
}

#[derive(Deserialize)]
struct EmbeddingVectors {
    address20: String,
    address32: String,
    basic_data_key: String,
    code_hash_key: String,
    header_sub_index_255_key: String,
    /// Keyed by decimal slot number, including values past `u64`.
    storage_slot_keys: BTreeMap<String, String>,
    /// Keyed by decimal chunk id.
    code_chunk_keys: BTreeMap<String, String>,
    code_hash: String,
}

#[derive(Deserialize)]
struct TrieCase {
    name: String,
    entries: Vec<Entry>,
    root: String,
}

#[derive(Deserialize)]
struct Entry {
    key: String,
    value: String,
}

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).expect("fixture hex string")
}

fn load() -> Fixture {
    let fixture: Fixture =
        serde_json::from_str(include_str!("vectors/binary_trie_vectors.json")).unwrap();
    // The fixture is vendored and refreshed from upstream, so its case
    // counts are expected to grow. Assert only that no section arrived
    // empty — an exact count would fail every legitimate refresh.
    assert!(!fixture.trie_roots.is_empty(), "no trie root cases");
    assert!(!fixture.chunkify_code.is_empty(), "no chunkify cases");
    assert!(!fixture.encode_basic_data.is_empty(), "no basic-data cases");
    assert!(
        !fixture.embedding.storage_slot_keys.is_empty(),
        "no storage slot keys"
    );
    assert!(
        !fixture.embedding.code_chunk_keys.is_empty(),
        "no code chunk keys"
    );
    fixture
}

#[test]
fn embedding_keys_match_spec() {
    let vectors = load().embedding;

    let address20 = H160::from_slice(&unhex(&vectors.address20));
    let address32 = embedding::address20_to_address32(address20);
    assert_eq!(address32.as_slice(), unhex(&vectors.address32).as_slice());

    assert_eq!(
        embedding::get_tree_key_for_basic_data(&address32),
        unhex(&vectors.basic_data_key)
    );
    assert_eq!(
        embedding::get_tree_key_for_code_hash(&address32),
        unhex(&vectors.code_hash_key)
    );
    assert_eq!(
        embedding::get_tree_key_for_header(&address32, 255),
        unhex(&vectors.header_sub_index_255_key)
    );

    for (slot, expected) in &vectors.storage_slot_keys {
        let storage_key = U256::from_dec_str(slot).expect("fixture decimal slot");
        assert_eq!(
            embedding::get_tree_key_for_storage_slot(&address32, storage_key),
            unhex(expected),
            "storage slot {slot}"
        );
    }

    let code_hash: [u8; 32] = unhex(&vectors.code_hash)
        .try_into()
        .expect("fixture code hash");
    for (chunk_id, expected) in &vectors.code_chunk_keys {
        let chunk_id: u64 = chunk_id.parse().expect("fixture decimal chunk id");
        assert_eq!(
            embedding::get_tree_key_for_code_chunk(&address32, &code_hash, chunk_id),
            unhex(expected),
            "code chunk {chunk_id}"
        );
    }
}

#[test]
fn chunkify_matches_spec() {
    let cases = load().chunkify_code;
    for case in cases {
        let chunks = embedding::chunkify_code(&unhex(&case.code));
        assert_eq!(
            chunks.len(),
            case.chunks.len(),
            "chunkify case {}",
            case.name
        );
        for (i, (chunk, expected)) in chunks.iter().zip(&case.chunks).enumerate() {
            assert_eq!(
                chunk.as_slice(),
                unhex(expected).as_slice(),
                "chunkify case {} chunk {i}",
                case.name
            );
        }
    }
}

#[test]
fn basic_data_matches_spec() {
    let cases = load().encode_basic_data;
    for case in cases {
        let balance = U256::from_str_radix(case.balance.trim_start_matches("0x"), 16)
            .expect("fixture hex balance");
        assert_eq!(
            embedding::encode_basic_data(case.code_size, case.nonce, balance)
                .unwrap()
                .as_slice(),
            unhex(&case.encoded).as_slice(),
            "basic data case code_size={} nonce={}",
            case.code_size,
            case.nonce
        );
    }
}

#[test]
fn incremental_matches_spec_roots() {
    for case in load().trie_roots {
        let mut trie = ethrex_binary_trie::trie::BinaryTrie::new_temp();
        for e in &case.entries {
            trie.insert(unhex(&e.key), unhex(&e.value).try_into().unwrap())
                .unwrap();
        }
        assert_eq!(
            trie.root().as_bytes(),
            unhex(&case.root).as_slice(),
            "trie case {}",
            case.name
        );
    }
}
