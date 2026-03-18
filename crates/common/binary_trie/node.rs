/// Maximum traversal depth: 31 bytes * 8 bits = 248 levels of InternalNodes.
pub const MAX_DEPTH: usize = 248;

/// Number of leaf values per StemNode (one per possible sub-index byte value).
pub const STEM_VALUES: usize = 256;

/// A node in the binary trie.
pub enum Node {
    Internal(InternalNode),
    Stem(StemNode),
}

/// Number of entries in the flat 511-node binary Merkle tree over 256 leaves.
/// Layout: index 0 = subtree root, indices 255..510 = leaf hashes (256 leaves).
pub const SUBTREE_SIZE: usize = 511;

/// An internal branching node with a left (bit=0) and right (bit=1) child.
pub struct InternalNode {
    pub left: Option<Box<Node>>,
    pub right: Option<Box<Node>>,
    /// Cached hash: `merkle_hash_64(left_hash || right_hash)`. Set to `None`
    /// whenever a descendant is mutated; filled lazily by `merkle::hash_node`.
    pub cached_hash: Option<[u8; 32]>,
}

impl InternalNode {
    pub fn new(left: Option<Box<Node>>, right: Option<Box<Node>>) -> Self {
        Self {
            left,
            right,
            cached_hash: None,
        }
    }
}

/// A stem node holding up to 256 32-byte values, keyed by the sub-index byte.
///
/// The values array is heap-allocated because 256 * 33 bytes would be too large
/// to place on the stack (especially inside recursive tree operations).
pub struct StemNode {
    pub stem: [u8; 31],
    /// Each slot holds a 32-byte value or None (empty).
    pub values: Box<[Option<[u8; 32]>; STEM_VALUES]>,
    /// Cached intermediate hashes for the 256-value subtree (511 entries).
    ///
    /// Flat binary tree layout:
    ///   - Index 0            = subtree root
    ///   - Indices 1, 2       = children of root
    ///   - ...
    ///   - Indices 255..=510  = leaf hashes (hash of each value, or ZERO_HASH)
    ///
    /// `None` means the cache has never been built; it is populated on the first
    /// `merkelize` call and updated incrementally on each `set_value`/`remove_value`.
    pub cached_subtree: Option<Box<[[u8; 32]; SUBTREE_SIZE]>>,
    /// Cached overall StemNode hash: `merkle_hash_64(stem || 0x00 || subtree_root)`.
    /// Cleared whenever a value is set or removed.
    pub cached_hash: Option<[u8; 32]>,
}

impl StemNode {
    /// Create a new empty StemNode with the given 31-byte stem.
    pub fn new(stem: [u8; 31]) -> Self {
        // Box::new([None; 256]) would require Copy on the Option type; we build manually.
        let values = Box::new([None::<[u8; 32]>; STEM_VALUES]);
        Self {
            stem,
            values,
            cached_subtree: None,
            cached_hash: None,
        }
    }

    /// Retrieve the value at the given sub-index (0–255).
    pub fn get_value(&self, sub_index: u8) -> Option<[u8; 32]> {
        self.values[sub_index as usize]
    }

    /// Set the value at the given sub-index, updating the subtree cache incrementally.
    pub fn set_value(&mut self, sub_index: u8, value: [u8; 32]) {
        self.values[sub_index as usize] = Some(value);
        self.update_subtree_cache(sub_index);
        self.cached_hash = None;
    }

    /// Remove the value at the given sub-index, returning the previous value if any.
    pub fn remove_value(&mut self, sub_index: u8) -> Option<[u8; 32]> {
        let old = self.values[sub_index as usize].take();
        if old.is_some() {
            self.update_subtree_cache(sub_index);
            self.cached_hash = None;
        }
        old
    }

    /// Returns true if all 256 value slots are empty.
    pub fn is_empty(&self) -> bool {
        self.values.iter().all(|v| v.is_none())
    }

    /// Update the subtree cache for a single changed leaf at `sub_index`.
    ///
    /// If the cache has never been built (`None`), this is a no-op — the full
    /// cache will be constructed lazily by `merkle::compute_subtree_root` on the
    /// next merkelization call.
    fn update_subtree_cache(&mut self, sub_index: u8) {
        use crate::hash::blake3_hash;
        use crate::merkle::ZERO_HASH;
        use crate::merkle::merkle_hash_64_pub;

        let cache = match self.cached_subtree.as_mut() {
            Some(c) => c,
            None => return, // Not yet built; full build will happen at merkelize time.
        };

        // Leaf index in the flat tree: leaves occupy indices 255..=510.
        let leaf_flat = 255 + sub_index as usize;

        // Recompute the leaf hash.
        cache[leaf_flat] = self.values[sub_index as usize]
            .map(|v| blake3_hash(&v))
            .unwrap_or(ZERO_HASH);

        // Walk up from the leaf to the root, recomputing each parent.
        let mut idx = leaf_flat;
        while idx > 0 {
            let parent = (idx - 1) / 2;
            let left_child = 2 * parent + 1;
            let right_child = 2 * parent + 2;
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&cache[left_child]);
            buf[32..].copy_from_slice(&cache[right_child]);
            cache[parent] = merkle_hash_64_pub(&buf);
            idx = parent;
        }
    }
}

/// Extract bit `depth` from a 31-byte stem using MSB-first ordering.
///
/// Bit 0 is the most-significant bit of byte 0.
/// Bit 7 is the least-significant bit of byte 0.
/// Bit 8 is the most-significant bit of byte 1.
/// ...
/// Bit 247 is the least-significant bit of byte 30.
///
/// Returns 0 or 1.
pub fn stem_bit(stem: &[u8; 31], depth: usize) -> u8 {
    debug_assert!(depth < MAX_DEPTH, "depth {depth} exceeds MAX_DEPTH");
    let byte_index = depth / 8;
    let bit_index = 7 - (depth % 8); // MSB first: bit 0 of depth maps to bit 7 within the byte
    (stem[byte_index] >> bit_index) & 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_bit_msb_first() {
        // stem[0] = 0b10000000 → bit 0 = 1, bit 1 = 0, ..., bit 7 = 0
        let mut stem = [0u8; 31];
        stem[0] = 0b1000_0000;
        assert_eq!(stem_bit(&stem, 0), 1);
        assert_eq!(stem_bit(&stem, 1), 0);
        assert_eq!(stem_bit(&stem, 7), 0);
    }

    #[test]
    fn stem_bit_lsb_of_byte0() {
        // stem[0] = 0b00000001 → bit 7 = 1
        let mut stem = [0u8; 31];
        stem[0] = 0b0000_0001;
        assert_eq!(stem_bit(&stem, 0), 0);
        assert_eq!(stem_bit(&stem, 7), 1);
    }

    #[test]
    fn stem_bit_second_byte() {
        // stem[1] = 0b10000000 → bit 8 = 1, bit 9 = 0
        let mut stem = [0u8; 31];
        stem[1] = 0b1000_0000;
        assert_eq!(stem_bit(&stem, 7), 0);
        assert_eq!(stem_bit(&stem, 8), 1);
        assert_eq!(stem_bit(&stem, 9), 0);
    }

    #[test]
    fn stem_bit_all_ones() {
        let stem = [0xFFu8; 31];
        for depth in 0..MAX_DEPTH {
            assert_eq!(stem_bit(&stem, depth), 1, "depth {depth} should be 1");
        }
    }

    #[test]
    fn stem_bit_all_zeros() {
        let stem = [0x00u8; 31];
        for depth in 0..MAX_DEPTH {
            assert_eq!(stem_bit(&stem, depth), 0, "depth {depth} should be 0");
        }
    }

    #[test]
    fn stem_bit_last_byte() {
        // stem[30] = 0b00000001 → bit 247 = 1
        let mut stem = [0u8; 31];
        stem[30] = 0b0000_0001;
        assert_eq!(stem_bit(&stem, 247), 1);
        assert_eq!(stem_bit(&stem, 246), 0);
    }

    #[test]
    fn stem_node_new_is_empty() {
        let stem = [0u8; 31];
        let node = StemNode::new(stem);
        assert!(node.is_empty());
    }

    #[test]
    fn stem_node_set_get() {
        let stem = [0u8; 31];
        let mut node = StemNode::new(stem);
        let value = [1u8; 32];
        node.set_value(42, value);
        assert_eq!(node.get_value(42), Some(value));
        assert_eq!(node.get_value(0), None);
        assert!(!node.is_empty());
    }

    #[test]
    fn stem_node_remove_value() {
        let stem = [0u8; 31];
        let mut node = StemNode::new(stem);
        let value = [7u8; 32];
        node.set_value(0, value);
        assert!(!node.is_empty());
        let removed = node.remove_value(0);
        assert_eq!(removed, Some(value));
        assert!(node.is_empty());
    }

    #[test]
    fn stem_node_remove_absent_returns_none() {
        let stem = [0u8; 31];
        let mut node = StemNode::new(stem);
        assert_eq!(node.remove_value(5), None);
    }

    #[test]
    fn stem_node_all_sub_indices() {
        let stem = [0xABu8; 31];
        let mut node = StemNode::new(stem);
        for i in 0u8..=255 {
            node.set_value(i, [i; 32]);
        }
        for i in 0u8..=255 {
            assert_eq!(node.get_value(i), Some([i; 32]));
        }
        assert!(!node.is_empty());
    }
}
