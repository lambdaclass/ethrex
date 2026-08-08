//! Raw EIP-8297 binary tree: a compressed binary radix trie mapping
//! prefix-free variable-length bit keys to 32-byte values, committing
//! to its contents with BLAKE3 hashes up to a single root.

mod binary_trie;
mod bits;
pub mod db;
pub(crate) mod node;
pub mod path;
pub mod prefix;

pub use binary_trie::{BinaryTrie, Committed, LeafChangelog};
pub use db::{BinaryTrieDB, InMemoryBinaryTrieDB};
pub use node::EMPTY_TRIE_ROOT;
pub use path::BitPath;
pub use prefix::KeyPrefix;

/// The hash a stored node commits to.
///
/// A node's stored bytes are exactly its hashing preimage (see
/// `BinaryTrie::commit`), so this re-derives the hash of whatever is at a
/// path. A storage layer uses it to answer "does this database really hold the
/// trie named by `root`" — read the node at [`BitPath::new`] and check it
/// hashes to `root` — which is the binary analogue of re-hashing the MPT root
/// node. Nodes are keyed by path, not by hash, so opening a trie at a root
/// proves nothing on its own.
pub fn hash_stored_node(encoded: &[u8]) -> ethereum_types::H256 {
    node::blake3_hash(encoded)
}

/// Longest accepted key, in bytes. Bounds branch-prefix bit counts
/// below the two-byte limit of `encode_bit_prefix`.
pub const MAX_KEY_LENGTH: usize = 8192;
