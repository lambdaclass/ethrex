use crate::error::BinaryTrieError;
use crate::node::{InternalNode, MAX_DEPTH, Node, StemNode, stem_bit};

/// The binary trie. The root is an owned optional boxed node.
pub struct BinaryTrie {
    pub root: Option<Box<Node>>,
}

/// Split a 32-byte key into a 31-byte stem and a 1-byte sub-index.
pub fn split_key(key: &[u8; 32]) -> ([u8; 31], u8) {
    let mut stem = [0u8; 31];
    stem.copy_from_slice(&key[..31]);
    (stem, key[31])
}

impl BinaryTrie {
    /// Create a new empty trie.
    pub fn new() -> Self {
        Self { root: None }
    }

    /// Insert a key-value pair into the trie.
    pub fn insert(&mut self, key: [u8; 32], value: [u8; 32]) -> Result<(), BinaryTrieError> {
        let (stem, sub_index) = split_key(&key);
        self.root = Some(insert_node(self.root.take(), stem, sub_index, value, 0)?);
        Ok(())
    }

    /// Look up the value for a key, returning None if absent.
    pub fn get(&self, key: [u8; 32]) -> Option<[u8; 32]> {
        let (stem, sub_index) = split_key(&key);
        get_node(self.root.as_deref(), &stem, sub_index)
    }

    /// Remove the value for a key, returning the previous value if it existed.
    pub fn remove(&mut self, key: [u8; 32]) -> Option<[u8; 32]> {
        let (stem, sub_index) = split_key(&key);
        let (new_root, removed) = remove_node(self.root.take(), &stem, sub_index);
        self.root = new_root;
        removed
    }
}

impl Default for BinaryTrie {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Recursive insert
// ---------------------------------------------------------------------------

fn insert_node(
    node: Option<Box<Node>>,
    stem: [u8; 31],
    sub_index: u8,
    value: [u8; 32],
    depth: usize,
) -> Result<Box<Node>, BinaryTrieError> {
    match node {
        // Empty slot — create a new StemNode here.
        None => {
            let mut stem_node = StemNode::new(stem);
            stem_node.set_value(sub_index, value);
            Ok(Box::new(Node::Stem(stem_node)))
        }

        Some(existing) => match *existing {
            // StemNode at this location.
            Node::Stem(mut stem_node) => {
                if stem_node.stem == stem {
                    // Same stem: just update the value in place.
                    stem_node.set_value(sub_index, value);
                    Ok(Box::new(Node::Stem(stem_node)))
                } else {
                    // Different stem: split by creating InternalNodes until the stems diverge.
                    if depth >= MAX_DEPTH {
                        return Err(BinaryTrieError::MaxDepthExceeded);
                    }
                    split_stems(stem_node, stem, sub_index, value, depth)
                }
            }

            // InternalNode: follow the bit for the new stem and recurse.
            Node::Internal(mut internal) => {
                if depth >= MAX_DEPTH {
                    return Err(BinaryTrieError::MaxDepthExceeded);
                }
                let bit = stem_bit(&stem, depth);
                if bit == 0 {
                    internal.left = Some(insert_node(
                        internal.left.take(),
                        stem,
                        sub_index,
                        value,
                        depth + 1,
                    )?);
                } else {
                    internal.right = Some(insert_node(
                        internal.right.take(),
                        stem,
                        sub_index,
                        value,
                        depth + 1,
                    )?);
                }
                // Invalidate this node's cached hash — a descendant was mutated.
                internal.cached_hash = None;
                Ok(Box::new(Node::Internal(internal)))
            }
        },
    }
}

/// Create a chain of InternalNodes until the two stems diverge, then place each
/// StemNode in the appropriate child slot.
fn split_stems(
    existing: StemNode,
    new_stem: [u8; 31],
    new_sub_index: u8,
    new_value: [u8; 32],
    depth: usize,
) -> Result<Box<Node>, BinaryTrieError> {
    if depth >= MAX_DEPTH {
        return Err(BinaryTrieError::MaxDepthExceeded);
    }

    let existing_bit = stem_bit(&existing.stem, depth);
    let new_bit = stem_bit(&new_stem, depth);

    if existing_bit != new_bit {
        // The stems diverge here — place each in the appropriate child.
        let mut new_stem_node = StemNode::new(new_stem);
        new_stem_node.set_value(new_sub_index, new_value);

        let (left, right) = if existing_bit == 0 {
            (
                Some(Box::new(Node::Stem(existing)) as Box<Node>),
                Some(Box::new(Node::Stem(new_stem_node)) as Box<Node>),
            )
        } else {
            (
                Some(Box::new(Node::Stem(new_stem_node)) as Box<Node>),
                Some(Box::new(Node::Stem(existing)) as Box<Node>),
            )
        };

        Ok(Box::new(Node::Internal(InternalNode::new(left, right))))
    } else {
        // Bits agree at this depth — recurse one level deeper.
        let inner = split_stems(existing, new_stem, new_sub_index, new_value, depth + 1)?;
        let (left, right) = if new_bit == 0 {
            (Some(inner), None)
        } else {
            (None, Some(inner))
        };
        Ok(Box::new(Node::Internal(InternalNode::new(left, right))))
    }
}

// ---------------------------------------------------------------------------
// Recursive get
// ---------------------------------------------------------------------------

fn get_node(node: Option<&Node>, stem: &[u8; 31], sub_index: u8) -> Option<[u8; 32]> {
    get_node_at_depth(node, stem, sub_index, 0)
}

fn get_node_at_depth(
    node: Option<&Node>,
    stem: &[u8; 31],
    sub_index: u8,
    depth: usize,
) -> Option<[u8; 32]> {
    match node? {
        Node::Stem(stem_node) => {
            if &stem_node.stem == stem {
                stem_node.get_value(sub_index)
            } else {
                None
            }
        }
        Node::Internal(internal) => {
            if depth >= MAX_DEPTH {
                return None;
            }
            let bit = stem_bit(stem, depth);
            if bit == 0 {
                get_node_at_depth(internal.left.as_deref(), stem, sub_index, depth + 1)
            } else {
                get_node_at_depth(internal.right.as_deref(), stem, sub_index, depth + 1)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Recursive remove
// ---------------------------------------------------------------------------

/// Returns the updated node (None if it should be collapsed) and the removed value.
fn remove_node(
    node: Option<Box<Node>>,
    stem: &[u8; 31],
    sub_index: u8,
) -> (Option<Box<Node>>, Option<[u8; 32]>) {
    remove_node_at_depth(node, stem, sub_index, 0)
}

fn remove_node_at_depth(
    node: Option<Box<Node>>,
    stem: &[u8; 31],
    sub_index: u8,
    depth: usize,
) -> (Option<Box<Node>>, Option<[u8; 32]>) {
    match node {
        None => (None, None),
        Some(boxed) => match *boxed {
            Node::Stem(mut stem_node) => {
                if &stem_node.stem != stem {
                    return (Some(Box::new(Node::Stem(stem_node))), None);
                }
                let removed = stem_node.remove_value(sub_index);
                if stem_node.is_empty() {
                    // StemNode is now empty — remove it entirely.
                    (None, removed)
                } else {
                    (Some(Box::new(Node::Stem(stem_node))), removed)
                }
            }

            Node::Internal(mut internal) => {
                if depth >= MAX_DEPTH {
                    return (Some(Box::new(Node::Internal(internal))), None);
                }
                let bit = stem_bit(stem, depth);
                let removed;
                if bit == 0 {
                    let (new_child, r) =
                        remove_node_at_depth(internal.left.take(), stem, sub_index, depth + 1);
                    internal.left = new_child;
                    removed = r;
                } else {
                    let (new_child, r) =
                        remove_node_at_depth(internal.right.take(), stem, sub_index, depth + 1);
                    internal.right = new_child;
                    removed = r;
                }

                // Collapse InternalNode if it now has only one child.
                // Also invalidate the cached hash — a descendant was mutated.
                internal.cached_hash = None;
                let updated = match (&internal.left, &internal.right) {
                    (None, None) => None,
                    (Some(_), None) => internal.left,
                    (None, Some(_)) => internal.right,
                    _ => Some(Box::new(Node::Internal(internal))),
                };
                (updated, removed)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(stem_byte: u8, sub: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = stem_byte;
        k[31] = sub;
        k
    }

    fn val(b: u8) -> [u8; 32] {
        [b; 32]
    }

    #[test]
    fn insert_and_get_single() {
        let mut trie = BinaryTrie::new();
        let k = key(0xAA, 0);
        trie.insert(k, val(1)).unwrap();
        assert_eq!(trie.get(k), Some(val(1)));
    }

    #[test]
    fn get_missing_returns_none() {
        let trie = BinaryTrie::new();
        assert_eq!(trie.get(key(0x01, 0)), None);
    }

    #[test]
    fn insert_same_stem_different_sub_index() {
        let mut trie = BinaryTrie::new();
        trie.insert(key(0x10, 0), val(10)).unwrap();
        trie.insert(key(0x10, 1), val(20)).unwrap();
        assert_eq!(trie.get(key(0x10, 0)), Some(val(10)));
        assert_eq!(trie.get(key(0x10, 1)), Some(val(20)));
    }

    #[test]
    fn insert_overwrites_existing_value() {
        let mut trie = BinaryTrie::new();
        trie.insert(key(0xBB, 5), val(1)).unwrap();
        trie.insert(key(0xBB, 5), val(2)).unwrap();
        assert_eq!(trie.get(key(0xBB, 5)), Some(val(2)));
    }

    #[test]
    fn insert_different_stems_causes_split() {
        let mut trie = BinaryTrie::new();
        // 0x00... and 0x80... differ at bit 0
        let k1 = [0u8; 32];
        let mut k2 = [0u8; 32];
        k2[0] = 0x80; // MSB set → bit 0 = 1
        trie.insert(k1, val(1)).unwrap();
        trie.insert(k2, val(2)).unwrap();
        assert_eq!(trie.get(k1), Some(val(1)));
        assert_eq!(trie.get(k2), Some(val(2)));
    }

    #[test]
    fn remove_existing_value() {
        let mut trie = BinaryTrie::new();
        let k = key(0xCC, 3);
        trie.insert(k, val(99)).unwrap();
        let removed = trie.remove(k);
        assert_eq!(removed, Some(val(99)));
        assert_eq!(trie.get(k), None);
    }

    #[test]
    fn remove_absent_returns_none() {
        let mut trie = BinaryTrie::new();
        assert_eq!(trie.remove(key(0x01, 0)), None);
    }

    #[test]
    fn remove_collapses_single_child_internal_node() {
        let mut trie = BinaryTrie::new();
        let k1 = [0u8; 32];
        let mut k2 = [0u8; 32];
        k2[0] = 0x80;
        trie.insert(k1, val(1)).unwrap();
        trie.insert(k2, val(2)).unwrap();
        // Remove one; the InternalNode at depth 0 should collapse.
        trie.remove(k2);
        assert_eq!(trie.get(k1), Some(val(1)));
        assert_eq!(trie.get(k2), None);
        // After collapse the root should be a StemNode directly.
        assert!(matches!(trie.root.as_deref(), Some(Node::Stem(_))));
    }

    #[test]
    fn remove_all_empties_trie() {
        let mut trie = BinaryTrie::new();
        let k = key(0xDD, 7);
        trie.insert(k, val(5)).unwrap();
        trie.remove(k);
        assert!(trie.root.is_none());
    }

    #[test]
    fn split_key_correctness() {
        let mut raw = [0u8; 32];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = i as u8;
        }
        let (stem, sub) = split_key(&raw);
        assert_eq!(&stem[..], &raw[..31]);
        assert_eq!(sub, 31);
    }

    #[test]
    fn insert_many_distinct_stems() {
        let mut trie = BinaryTrie::new();
        for i in 0u8..=255 {
            let mut k = [0u8; 32];
            k[0] = i;
            trie.insert(k, val(i)).unwrap();
        }
        for i in 0u8..=255 {
            let mut k = [0u8; 32];
            k[0] = i;
            assert_eq!(trie.get(k), Some(val(i)), "missing key {i}");
        }
    }

    #[test]
    fn get_wrong_stem_returns_none() {
        let mut trie = BinaryTrie::new();
        trie.insert(key(0xAA, 0), val(1)).unwrap();
        // Query a different stem that is not in the trie.
        assert_eq!(trie.get(key(0xBB, 0)), None);
    }

    #[test]
    fn remove_deep_collapse() {
        // 0x00... and 0x01... share 7 prefix bits (both start 0000000...),
        // diverging at bit 7. Inserting both creates 7 nested InternalNodes.
        // 0x80... diverges at bit 0.
        // After removing 0x01..., the 7-level chain should collapse back to
        // a single InternalNode with 0x00... left and 0x80... right.
        let mut trie = BinaryTrie::new();
        let k_00 = key(0x00, 0);
        let k_01 = key(0x01, 0);
        let k_80 = key(0x80, 0);

        trie.insert(k_00, val(1)).unwrap();
        trie.insert(k_01, val(2)).unwrap();
        trie.insert(k_80, val(3)).unwrap();

        trie.remove(k_01);

        // 0x00 and 0x80 should still be present.
        assert_eq!(trie.get(k_00), Some(val(1)));
        assert_eq!(trie.get(k_80), Some(val(3)));
        assert_eq!(trie.get(k_01), None);

        // Root hash should match a fresh trie with only 0x00 and 0x80.
        let mut fresh = BinaryTrie::new();
        fresh.insert(k_00, val(1)).unwrap();
        fresh.insert(k_80, val(3)).unwrap();

        assert_eq!(
            crate::merkle::merkelize(trie.root.as_deref_mut()),
            crate::merkle::merkelize(fresh.root.as_deref_mut())
        );
    }

    #[test]
    fn remove_preserves_sibling_sub_index() {
        // Insert two values on the same stem at different sub-indices.
        // Remove one; the other should remain and the StemNode should stay.
        let mut trie = BinaryTrie::new();
        let k0 = key(0xAA, 0);
        let k1 = key(0xAA, 1);
        trie.insert(k0, val(10)).unwrap();
        trie.insert(k1, val(20)).unwrap();

        let removed = trie.remove(k0);
        assert_eq!(removed, Some(val(10)));
        assert_eq!(trie.get(k0), None);
        assert_eq!(trie.get(k1), Some(val(20)));
        assert!(matches!(trie.root.as_deref(), Some(Node::Stem(_))));
    }
}
