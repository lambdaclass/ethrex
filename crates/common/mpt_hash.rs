/// Minimal re-export of MPT hash computation for Ethereum consensus validation.
///
/// Receipt, transaction, and withdrawal roots require MPT hashing per the
/// Ethereum specification. This module delegates to `ethrex-trie` which
/// is kept solely for this purpose. The full MPT state trie has been replaced
/// by the binary trie (EIP-7864).
pub use ethrex_trie::Trie;

use ethereum_types::H256;
use ethrex_crypto::Crypto;

pub type PathRLP = Vec<u8>;
pub type ValueRLP = Vec<u8>;

/// Builds an in-memory MPT from the given key-value iterator and returns its root hash.
///
/// Keys and values must already be RLP-encoded.
pub fn compute_hash_from_unsorted_iter(
    iter: impl Iterator<Item = (PathRLP, ValueRLP)>,
    crypto: &dyn Crypto,
) -> H256 {
    Trie::compute_hash_from_unsorted_iter(iter, crypto)
}
