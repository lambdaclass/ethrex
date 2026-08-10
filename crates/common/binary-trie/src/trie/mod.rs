//! Raw EIP-8297 binary tree: a compressed binary radix trie mapping
//! prefix-free variable-length bit keys to 32-byte values, committing
//! to its contents with BLAKE3 hashes up to a single root.

mod binary_trie;
mod bits;
#[cfg(test)]
mod commit_cost;
pub mod db;
pub(crate) mod node;
pub mod path;
pub mod prefix;
pub mod proof;
pub mod range;

pub use binary_trie::{BinaryTrie, Committed, LeafBatch, LeafChangelog};
pub use db::{BinaryTrieDB, InMemoryBinaryTrieDB};
pub use node::EMPTY_TRIE_ROOT;
pub use path::BitPath;
pub use prefix::KeyPrefix;
pub use proof::{ProofError, WalkEnd, WalkStep, verify_walk};
pub use range::{RangeProofError, RangeSlice, VerifiedRange, prove_range, verify_range};

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

/// The next key after `key` in this trie's order: bytewise `+1` with carry,
/// `None` when `key` is all-`0xff` and has no successor.
///
/// The successor function of *this* trie's key order, which is why it lives
/// here rather than in a range API. It is only a true successor because the
/// key set is **prefix-free**: with variable-length keys in general,
/// `[0x01, 0x00]` sits strictly between `[0x01]` and `increment_key(&[0x01])
/// == [0x02]`, so `+1` would skip it. This trie's keys are all 32-byte stems
/// plus a one-byte suffix (34 bytes) or a 64-byte prefix plus two (66), and no
/// key is a prefix of another, so nothing can hide in that gap: any key
/// strictly greater than `key` and of any accepted length is at or after the
/// carry result.
///
/// The exclusive-upper-bound converse of the inclusive `origin` that
/// [`BinaryTrie::leaves_from`] takes: a caller resuming a scan past a key it
/// has already served asks for `increment_key(&last)` rather than re-reading
/// and discarding `last`. `None` means "there is nothing after this", which a
/// resumable scan reports as exhaustion rather than as an error.
///
/// The empty key has no successor either, and returns `None` — not `[0x01]`.
/// An empty key is the *start* sentinel in every scan API here (an empty
/// `origin` means "from the beginning"), so treating it as a value to
/// increment would turn "scan everything" into "scan from `[0x01]`". This
/// needs no guard of its own: the loop below runs zero times and falls through
/// to the same `None` the all-`0xff` case reaches, and an explicit early return
/// was removed after a mutation check showed it could not change an answer.
pub fn increment_key(key: &[u8]) -> Option<Vec<u8>> {
    let mut next = key.to_vec();
    for byte in next.iter_mut().rev() {
        let (incremented, carried) = byte.overflowing_add(1);
        *byte = incremented;
        if !carried {
            return Some(next);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::increment_key;

    #[test]
    fn increments_the_last_byte() {
        assert_eq!(increment_key(&[0x00]), Some(vec![0x01]));
        assert_eq!(increment_key(&[0x01, 0x02]), Some(vec![0x01, 0x03]));
        assert_eq!(increment_key(&[0xff, 0x00]), Some(vec![0xff, 0x01]));
    }

    #[test]
    fn carries_across_byte_boundaries() {
        assert_eq!(increment_key(&[0x00, 0xff]), Some(vec![0x01, 0x00]));
        assert_eq!(
            increment_key(&[0x01, 0xff, 0xff]),
            Some(vec![0x02, 0x00, 0x00])
        );
    }

    #[test]
    fn all_ones_has_no_successor() {
        assert_eq!(increment_key(&[0xff]), None);
        assert_eq!(increment_key(&[0xff; 34]), None);
        assert_eq!(increment_key(&[0xff; 66]), None);
    }

    #[test]
    fn the_empty_key_has_no_successor() {
        // Not `[0x01]`: the empty key is the "from the beginning" sentinel of
        // every scan API here, so incrementing it would silently skip the whole
        // `[0x00..]` region.
        assert_eq!(increment_key(&[]), None);
    }

    #[test]
    fn the_result_is_the_least_key_strictly_greater_for_this_trie() {
        // The property that makes `+1` a successor at all: for a fixed-length
        // key space, nothing sorts between `key` and `increment_key(key)`.
        let key = [0x04u8; 34];
        let next = increment_key(&key).expect("34-byte key is not all-0xff");
        assert!(next.as_slice() > key.as_slice());
        // Every 34-byte key strictly between the two would have to differ from
        // `key` only in its last byte, and the last byte of `next` is
        // `key`'s + 1, so there is no room.
        assert_eq!(next[..33], key[..33]);
        assert_eq!(next[33], key[33] + 1);
    }
}
