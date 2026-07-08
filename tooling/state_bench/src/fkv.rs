//! Flat-KV key encoding, shared by `gen-state`'s direct flat-KV writer and its
//! equivalence test.
//!
//! `gen-state` writes the flat-KV index itself (deriving entries straight from
//! the data it inserts into the tries) instead of invoking the store's
//! background `generate_flatkeyvalue()`, which re-scans the whole trie and
//! allocates ~fixture-proportional heap during generation. For that to be
//! correct the entries must be keyed EXACTLY as the store's generator keys them.
//! These helpers capture that encoding in one place; `tests/fkv_equivalence.rs`
//! builds a small state, runs the real generator, and asserts the entries it
//! wrote are retrievable under these keys.

use ethrex_common::H256;
use ethrex_storage::apply_prefix;
use ethrex_trie::Nibbles;

/// Flat-KV key for an account: the account's hashed-address leaf path
/// (`Nibbles::from_bytes` appends the `16` leaf terminator, giving 65 bytes).
pub fn account_fkv_key(hashed_address: &[u8]) -> Vec<u8> {
    Nibbles::from_bytes(hashed_address).into_vec()
}

/// Flat-KV key for a storage slot: the account prefix followed by the slot's
/// hashed-key leaf path, i.e. `apply_prefix(Some(account_hash), leaf_path)`
/// (65-byte account path + `17` separator + 65-byte slot path = 131 bytes).
pub fn storage_fkv_key(account_hash: H256, hashed_key: &[u8]) -> Vec<u8> {
    apply_prefix(Some(account_hash), Nibbles::from_bytes(hashed_key)).into_vec()
}
