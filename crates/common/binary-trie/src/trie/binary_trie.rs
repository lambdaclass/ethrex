//! Incremental insertion-based binary radix trie.
//!
//! The trie retains its node structure across insertions, splitting
//! nodes on descent, and hashes that structure on [`BinaryTrie::root`].
//! Canonical-form invariant: a branch's prefix is exactly the bits its
//! two subtrees share beyond the parent split, so the structure — and
//! therefore the root — depends only on the key/value set, never on
//! insertion order.
//!
//! Deliberate simplifications, deferred to the storage-integration
//! plan: no hash caching (every `root()` call rehashes the whole
//! tree), no deletion, and no `TrieDB` backing (all nodes live in
//! memory). Recursion depth is bounded by the key length in bits, so
//! max-length keys could theoretically overflow the stack; embedding
//! keys are at most 66 bytes, and the storage-integration rework is
//! the place to go iterative if ever needed.

use ethereum_types::H256;

use crate::error::BinaryTrieError;

use super::MAX_KEY_LENGTH;
use super::bits::bytes_to_bits;
use super::node::{EMPTY_TRIE_ROOT, branch_hash, leaf_hash};

enum Node {
    Leaf {
        key: Vec<u8>,
        value: [u8; 32],
    },
    Branch {
        /// Bits (0/1 per element) shared by every key below, relative
        /// to the parent's split point.
        prefix: Vec<u8>,
        left: Box<Node>,
        right: Box<Node>,
    },
}

/// Compressed binary radix trie over prefix-free byte keys and
/// 32-byte values, committing to its contents with a BLAKE3 root.
#[derive(Default)]
pub struct BinaryTrie {
    root: Option<Node>,
}

impl BinaryTrie {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `key` with `value`, overwriting any existing value for
    /// the same key.
    ///
    /// # Errors
    ///
    /// The trie is left unchanged on every error:
    /// - [`BinaryTrieError::EmptyKey`] if `key` is empty.
    /// - [`BinaryTrieError::KeyTooLong`] if `key` exceeds
    ///   [`MAX_KEY_LENGTH`] bytes.
    /// - [`BinaryTrieError::PrefixViolation`] if inserting `key` would
    ///   make some key a bit-prefix of another.
    pub fn insert(&mut self, key: Vec<u8>, value: [u8; 32]) -> Result<(), BinaryTrieError> {
        if key.is_empty() {
            return Err(BinaryTrieError::EmptyKey);
        }
        if key.len() > MAX_KEY_LENGTH {
            return Err(BinaryTrieError::KeyTooLong);
        }
        let bits = bytes_to_bits(&key);
        match &mut self.root {
            None => {
                self.root = Some(Node::Leaf { key, value });
                Ok(())
            }
            Some(node) => Self::insert_at(node, &bits, 0, key, value),
        }
    }

    /// Length of the run where `bits` from `depth` agrees with `other`.
    ///
    /// Errors when `bits` runs out first: the new key would then be a
    /// bit-prefix of what already lives below, which the tree cannot
    /// represent.
    fn shared_len(bits: &[u8], depth: usize, other: &[u8]) -> Result<usize, BinaryTrieError> {
        for (i, expected) in other.iter().enumerate() {
            match bits.get(depth + i) {
                None => return Err(BinaryTrieError::PrefixViolation),
                Some(bit) if bit != expected => return Ok(i),
                Some(_) => {}
            }
        }
        Ok(other.len())
    }

    /// Insert into the subtree rooted at `node`, whose path from the
    /// trie root has consumed the first `depth` bits of every key below
    /// it.
    ///
    /// Every failure is detected before anything is mutated, so an
    /// error leaves the subtree — and therefore the trie — untouched.
    fn insert_at(
        node: &mut Node,
        bits: &[u8],
        depth: usize,
        key: Vec<u8>,
        value: [u8; 32],
    ) -> Result<(), BinaryTrieError> {
        // Read-only phase: find the bit position to split at, or return
        // having changed nothing.
        let split = match node {
            Node::Leaf {
                key: leaf_key,
                value: leaf_value,
            } => {
                if *leaf_key == key {
                    *leaf_value = value;
                    return Ok(());
                }
                // A leaf reached at `depth` consumed that many of its
                // own bits, so slicing from `depth` is always in range.
                let other = bytes_to_bits(leaf_key);
                let shared = Self::shared_len(bits, depth, &other[depth..])?;
                if depth + shared >= other.len() {
                    // The stored key ran out first: it is a prefix of
                    // the new one.
                    return Err(BinaryTrieError::PrefixViolation);
                }
                depth + shared
            }
            Node::Branch {
                prefix,
                left,
                right,
            } => {
                let shared = Self::shared_len(bits, depth, prefix)?;
                if shared == prefix.len() {
                    // Full prefix match: descend on the bit at the split.
                    let split = depth + prefix.len();
                    let Some(&bit) = bits.get(split) else {
                        return Err(BinaryTrieError::PrefixViolation);
                    };
                    let child = if bit == 0 { left } else { right };
                    return Self::insert_at(child, bits, split + 1, key, value);
                }
                depth + shared
            }
        };

        // Mutating phase: split `node` in two, with the leaf and the
        // displaced subtree ordered by the key's bit at `split`.
        let displaced = std::mem::replace(
            node,
            Node::Leaf {
                key: Vec::new(),
                value: [0; 32],
            },
        );
        let (prefix, displaced) = match displaced {
            Node::Leaf { .. } => (bits[depth..split].to_vec(), displaced),
            Node::Branch {
                mut prefix,
                left,
                right,
            } => {
                // The bit at `split` becomes the new branch's split bit,
                // so it belongs to neither side's prefix.
                let tail = prefix.split_off(split - depth + 1);
                prefix.truncate(split - depth);
                (
                    prefix,
                    Node::Branch {
                        prefix: tail,
                        left,
                        right,
                    },
                )
            }
        };
        let new_leaf = Box::new(Node::Leaf { key, value });
        let displaced = Box::new(displaced);
        let (left, right) = if bits[split] == 0 {
            (new_leaf, displaced)
        } else {
            (displaced, new_leaf)
        };
        *node = Node::Branch {
            prefix,
            left,
            right,
        };
        Ok(())
    }

    /// Value stored under `key`, or `None` if absent.
    pub fn get(&self, key: &[u8]) -> Option<[u8; 32]> {
        let bits = bytes_to_bits(key);
        let mut node = self.root.as_ref()?;
        let mut depth = 0;
        loop {
            match node {
                Node::Leaf {
                    key: leaf_key,
                    value,
                } => return (leaf_key.as_slice() == key).then_some(*value),
                Node::Branch {
                    prefix,
                    left,
                    right,
                } => {
                    let split = depth + prefix.len();
                    if split >= bits.len() || bits[depth..split] != prefix[..] {
                        return None;
                    }
                    node = if bits[split] == 0 { left } else { right };
                    depth = split + 1;
                }
            }
        }
    }

    /// Root hash: [`EMPTY_TRIE_ROOT`] for the empty trie, otherwise
    /// the recursive tagged BLAKE3 commitment of the retained node
    /// structure.
    pub fn root(&self) -> H256 {
        match &self.root {
            None => EMPTY_TRIE_ROOT,
            Some(node) => Self::merkleize(node),
        }
    }

    fn merkleize(node: &Node) -> H256 {
        match node {
            Node::Leaf { key, value } => leaf_hash(key, value),
            Node::Branch {
                prefix,
                left,
                right,
            } => branch_hash(prefix, Self::merkleize(left), Self::merkleize(right)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::BinaryTrieError;
    use crate::trie::node::EMPTY_TRIE_ROOT;
    use hex_literal::hex;

    #[test]
    fn empty_trie_root_is_sentinel() {
        assert_eq!(BinaryTrie::new().root(), EMPTY_TRIE_ROOT);
    }

    #[test]
    fn single_leaf_matches_spec_vector() {
        let mut trie = BinaryTrie::new();
        trie.insert(vec![0u8; 34], [0x01; 32]).unwrap();
        assert_eq!(
            trie.root().0,
            hex!("4b60a28dce9f3529d103a26e00fadb98514cbd16ce03b7df752426addef9bbc7")
        );
    }

    #[test]
    fn get_returns_inserted_value_and_none_for_absent() {
        let mut trie = BinaryTrie::new();
        trie.insert(vec![0xab; 34], [7; 32]).unwrap();
        assert_eq!(trie.get(&[0xab; 34]), Some([7; 32]));
        assert_eq!(trie.get(&[0xac; 34]), None);
    }

    #[test]
    fn overwrite_replaces_value() {
        let mut trie = BinaryTrie::new();
        trie.insert(vec![0x42; 34], [1; 32]).unwrap();
        trie.insert(vec![0x42; 34], [2; 32]).unwrap();
        assert_eq!(trie.get(&[0x42; 34]), Some([2; 32]));
    }

    #[test]
    fn rejects_empty_key_and_oversized_key() {
        let mut trie = BinaryTrie::new();
        assert_eq!(trie.insert(vec![], [0; 32]), Err(BinaryTrieError::EmptyKey));
        assert_eq!(
            trie.insert(vec![0; 8193], [0; 32]),
            Err(BinaryTrieError::KeyTooLong)
        );
    }

    #[test]
    fn rejects_prefix_violations_both_directions() {
        let mut trie = BinaryTrie::new();
        trie.insert(vec![0xaa, 0xbb], [1; 32]).unwrap();
        assert_eq!(
            trie.insert(vec![0xaa], [2; 32]),
            Err(BinaryTrieError::PrefixViolation)
        );
        assert_eq!(
            trie.insert(vec![0xaa, 0xbb, 0xcc], [2; 32]),
            Err(BinaryTrieError::PrefixViolation)
        );
    }

    #[test]
    fn failed_insert_leaves_trie_unchanged() {
        let mut trie = BinaryTrie::new();
        trie.insert(vec![0xaa, 0xbb], [1; 32]).unwrap();
        let root_before = trie.root();
        let _ = trie.insert(vec![0xaa], [2; 32]);
        let _ = trie.insert(vec![0xaa, 0xbb, 0xcc], [2; 32]);
        assert_eq!(trie.root(), root_before);
        assert_eq!(trie.get(&[0xaa, 0xbb]), Some([1; 32]));
    }

    #[test]
    fn failed_insert_below_branch_leaves_trie_unchanged() {
        // Two keys sharing their first 9 bits force a root branch with
        // a long prefix, so both Branch-arm error sites are reachable.
        let mut trie = BinaryTrie::new();
        trie.insert(vec![0xaa, 0xbb], [1; 32]).unwrap();
        trie.insert(vec![0xaa, 0xcc], [2; 32]).unwrap();
        let root_before = trie.root();
        // Runs out of bits inside the branch's prefix walk.
        assert_eq!(
            trie.insert(vec![0xaa], [3; 32]),
            Err(BinaryTrieError::PrefixViolation)
        );
        // Fails at the leaf below the branch, exercising error
        // propagation and branch reconstruction on the way back up.
        assert_eq!(
            trie.insert(vec![0xaa, 0xbb, 0xcc], [3; 32]),
            Err(BinaryTrieError::PrefixViolation)
        );
        assert_eq!(trie.root(), root_before);
        assert_eq!(trie.get(&[0xaa, 0xbb]), Some([1; 32]));
        assert_eq!(trie.get(&[0xaa, 0xcc]), Some([2; 32]));
    }

    #[test]
    fn insertion_order_does_not_change_root() {
        let keys: [&[u8]; 3] = [&[0xf0, 0x00], &[0xf1, 0x00], &[0x0f, 0x00]];
        let mut forward = BinaryTrie::new();
        let mut reverse = BinaryTrie::new();
        for k in keys {
            forward.insert(k.to_vec(), [9; 32]).unwrap();
        }
        for k in keys.iter().rev() {
            reverse.insert(k.to_vec(), [9; 32]).unwrap();
        }
        assert_eq!(forward.root(), reverse.root());
    }
}
