use std::collections::BTreeMap;
use std::sync::Arc;

use ethrex_binary_trie::db::{TrieBackend, WriteOp};
use ethrex_binary_trie::error::BinaryTrieError;
use ethrex_binary_trie::hash::blake3_hash;
use ethrex_binary_trie::merkle::{ZERO_HASH, merkle_hash_64_pub};
use ethrex_binary_trie::node::{
    InternalNode, MAX_DEPTH, Node, NodeId, STEM_VALUES, SUBTREE_SIZE, StemNode, stem_bit,
};
use ethrex_binary_trie::{META_NEXT_ID, META_ROOT, node_key, serialize_node};

/// Entry on the open right spine during bulk trie construction.
struct SpineEntry {
    /// Depth of this internal node (0 = root).
    depth: usize,
    /// Allocated NodeId for this internal node.
    node_id: NodeId,
    /// Left child (NodeId, hash) — set when the left subtree is finalized.
    left: Option<(NodeId, [u8; 32])>,
    /// Right child (NodeId, hash) — filled during collapse.
    right: Option<(NodeId, [u8; 32])>,
}

/// Builds a binary trie from sorted (tree_key, value) entries in a single pass.
///
/// Instead of inserting entries one by one (which requires random trie traversal),
/// this builder exploits the sorted order to construct the trie left-to-right.
/// Only the "right spine" (path from root to the rightmost open node) is kept
/// in memory. Completed subtrees are serialized and flushed immediately.
pub struct BulkTrieBuilder {
    /// Open right spine: InternalNodes from root (index 0) to current position.
    spine: Vec<SpineEntry>,
    /// Next NodeId to allocate (monotonically increasing from 1).
    next_id: NodeId,
    /// Buffered write operations, flushed when reaching threshold.
    write_buf: Vec<WriteOp>,
    /// Flush write_buf when it reaches this many operations.
    flush_threshold: usize,
    /// Backend for writing nodes.
    backend: Arc<dyn TrieBackend>,
    /// Table name for nodes.
    nodes_table: &'static str,
    /// Reusable buffer for StemNode subtree hash computation (511 entries).
    subtree_buf: Box<[[u8; 32]; SUBTREE_SIZE]>,
    /// The most recent leaf (StemNode) not yet attached to an InternalNode,
    /// together with its logical depth. StemNodes are at depth MAX_DEPTH (= 248)
    /// meaning they live below all InternalNode levels.
    pending_leaf: Option<(NodeId, [u8; 32], usize)>,
    /// Previous stem, for computing divergence depth and wrapper directions.
    prev_stem: Option<[u8; 31]>,
    /// Total nodes written (for progress tracking).
    pub nodes_written: u64,
}

impl BulkTrieBuilder {
    pub fn new(backend: Arc<dyn TrieBackend>, nodes_table: &'static str) -> Self {
        Self {
            spine: Vec::new(),
            next_id: 1,
            write_buf: Vec::new(),
            flush_threshold: 500_000,
            backend,
            nodes_table,
            subtree_buf: Box::new([[0u8; 32]; SUBTREE_SIZE]),
            pending_leaf: None,
            prev_stem: None,
            nodes_written: 0,
        }
    }

    fn alloc_id(&mut self) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn buffer_node(&mut self, id: NodeId, node: &Node) -> Result<(), BinaryTrieError> {
        self.write_buf.push(WriteOp::Put {
            table: self.nodes_table,
            key: Box::from(node_key(id)),
            value: serialize_node(node),
        });
        self.nodes_written += 1;
        if self.write_buf.len() >= self.flush_threshold {
            self.flush_buffer()?;
        }
        Ok(())
    }

    fn flush_buffer(&mut self) -> Result<(), BinaryTrieError> {
        if self.write_buf.is_empty() {
            return Ok(());
        }
        let ops = std::mem::take(&mut self.write_buf);
        self.backend.write_batch(ops)
    }

    /// Find the first bit position where two stems differ (MSB-first ordering).
    /// Returns MAX_DEPTH (248) if stems are identical (should not happen in practice).
    fn divergence_depth(a: &[u8; 31], b: &[u8; 31]) -> usize {
        for byte_idx in 0..31 {
            let xor = a[byte_idx] ^ b[byte_idx];
            if xor != 0 {
                // leading_zeros gives the count of zero bits before the first set bit (MSB-first).
                // Since stem_bit uses MSB-first ordering, this maps directly to the depth offset.
                return byte_idx * 8 + xor.leading_zeros() as usize;
            }
        }
        MAX_DEPTH
    }

    /// Compute the hash of a StemNode given its stem and values.
    fn compute_stem_hash(
        subtree_buf: &mut [[u8; 32]; SUBTREE_SIZE],
        stem: &[u8; 31],
        values: &BTreeMap<u8, [u8; 32]>,
    ) -> [u8; 32] {
        // Fill leaves with ZERO_HASH.
        for i in 0..STEM_VALUES {
            subtree_buf[255 + i] = ZERO_HASH;
        }
        // Overwrite present values with their blake3 hashes.
        for (&idx, val) in values {
            subtree_buf[255 + idx as usize] = blake3_hash(val);
        }
        // Reduce bottom-up across the complete binary tree over 256 leaves.
        for parent in (0..255usize).rev() {
            let left_child = 2 * parent + 1;
            let right_child = 2 * parent + 2;
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&subtree_buf[left_child]);
            buf[32..].copy_from_slice(&subtree_buf[right_child]);
            subtree_buf[parent] = merkle_hash_64_pub(&buf);
        }
        let subtree_root = subtree_buf[0];

        // hash(stem || 0x00 || subtree_root)
        let mut buf = [0u8; 64];
        buf[..31].copy_from_slice(stem);
        buf[31] = 0x00;
        buf[32..].copy_from_slice(&subtree_root);
        merkle_hash_64_pub(&buf)
    }

    /// Serialize an InternalNode from a spine entry and compute its hash.
    fn finalize_internal(entry: &SpineEntry) -> ([u8; 32], Node) {
        let left_hash = entry.left.map(|(_, h)| h).unwrap_or(ZERO_HASH);
        let right_hash = entry.right.map(|(_, h)| h).unwrap_or(ZERO_HASH);
        let left_id = entry.left.map(|(id, _)| id);
        let right_id = entry.right.map(|(id, _)| id);

        let node = Node::Internal(InternalNode {
            left: left_id,
            right: right_id,
            cached_hash: None,
        });

        let mut hash_buf = [0u8; 64];
        hash_buf[..32].copy_from_slice(&left_hash);
        hash_buf[32..].copy_from_slice(&right_hash);
        let hash = merkle_hash_64_pub(&hash_buf);

        (hash, node)
    }

    /// Wrap `child` in a chain of single-child InternalNodes from depth `from_depth - 1`
    /// down to depth `to_depth`, following the path bits of `stem`.
    ///
    /// This mirrors the wrapping that `BinaryTrie::split_stems` creates when two stems
    /// share a common prefix: each shared-prefix level becomes a one-child InternalNode.
    ///
    /// If `child` is a bare StemNode (signalled by `from_depth == MAX_DEPTH`), no
    /// wrapping is needed and `child` is returned as-is (StemNodes are direct children
    /// of InternalNodes in the trie, with no extra wrapper above them).
    ///
    /// If `from_depth <= to_depth`, also a no-op (already at target depth).
    fn create_wrappers(
        &mut self,
        child: (NodeId, [u8; 32]),
        from_depth: usize,
        to_depth: usize,
        stem: &[u8; 31],
    ) -> Result<(NodeId, [u8; 32]), BinaryTrieError> {
        // StemNodes are immediate children of InternalNodes — no wrapper nodes above them.
        if from_depth == MAX_DEPTH {
            return Ok(child);
        }
        // No wrappers needed if already at or above target.
        if from_depth <= to_depth {
            return Ok(child);
        }
        // Build wrappers from depth from_depth-1 down to to_depth (bottom-up).
        let mut current = child;
        for d in (to_depth..from_depth).rev() {
            let bit = stem_bit(stem, d);
            let id = self.alloc_id();
            let (left_id, right_id, left_hash, right_hash) = if bit == 0 {
                (Some(current.0), None, current.1, ZERO_HASH)
            } else {
                (None, Some(current.0), ZERO_HASH, current.1)
            };
            let node = Node::Internal(InternalNode {
                left: left_id,
                right: right_id,
                cached_hash: None,
            });
            self.buffer_node(id, &node)?;
            let mut hash_buf = [0u8; 64];
            hash_buf[..32].copy_from_slice(&left_hash);
            hash_buf[32..].copy_from_slice(&right_hash);
            let hash = merkle_hash_64_pub(&hash_buf);
            current = (id, hash);
        }
        Ok(current)
    }

    /// Insert a stem with its values. Stems MUST be inserted in sorted order.
    pub fn insert_stem(
        &mut self,
        stem: [u8; 31],
        values: &BTreeMap<u8, [u8; 32]>,
    ) -> Result<(), BinaryTrieError> {
        // Create and serialize the StemNode.
        let stem_node = StemNode {
            stem,
            values: values.clone(),
            cached_hash: None,
        };
        let stem_hash = Self::compute_stem_hash(&mut self.subtree_buf, &stem, values);
        let stem_id = self.alloc_id();
        self.buffer_node(stem_id, &Node::Stem(stem_node))?;

        if self.prev_stem.is_none() {
            // First stem: record as pending leaf at "leaf depth" (below all InternalNodes).
            self.pending_leaf = Some((stem_id, stem_hash, MAX_DEPTH));
            self.prev_stem = Some(stem);
            return Ok(());
        }

        let prev = self.prev_stem.unwrap();
        let div = Self::divergence_depth(&prev, &stem);

        // Take the pending leaf (previous StemNode or last returned subtree root).
        let (mut child_id, mut child_hash, mut child_depth) = self.pending_leaf.take().unwrap();

        // Collapse spine entries at depth >= div, bottom-up.
        // Each popped entry receives the current child as its right subtree.
        while let Some(entry) = self.spine.last() {
            if entry.depth < div {
                break;
            }
            let mut entry = self.spine.pop().unwrap();

            // Wrap the child in single-child InternalNodes to bridge from child_depth
            // down to entry.depth + 1, if there's a gap (shared-prefix levels).
            let wrapped =
                self.create_wrappers((child_id, child_hash), child_depth, entry.depth + 1, &prev)?;
            entry.right = Some(wrapped);

            let (hash, node) = Self::finalize_internal(&entry);
            self.buffer_node(entry.node_id, &node)?;

            child_id = entry.node_id;
            child_hash = hash;
            child_depth = entry.depth;
        }

        // child is now the collapsed left subtree at the divergence depth `div`.
        // Wrap it to bridge from child_depth down to div + 1.
        let wrapped_left =
            self.create_wrappers((child_id, child_hash), child_depth, div + 1, &prev)?;

        // Create a new InternalNode at depth `div` with the wrapped left child.
        let div_node_id = self.alloc_id();
        self.spine.push(SpineEntry {
            depth: div,
            node_id: div_node_id,
            left: Some(wrapped_left),
            right: None,
        });

        // The new StemNode becomes the pending leaf.
        self.pending_leaf = Some((stem_id, stem_hash, MAX_DEPTH));
        self.prev_stem = Some(stem);
        Ok(())
    }

    /// Finalize the trie. Returns `(root_node_id, root_hash)`.
    ///
    /// Must be called after all stems have been inserted. Flushes all buffered
    /// write operations including root and next_id metadata.
    pub fn finish(&mut self) -> Result<(Option<NodeId>, [u8; 32]), BinaryTrieError> {
        if self.pending_leaf.is_none() {
            // Empty trie.
            self.flush_buffer()?;
            return Ok((None, ZERO_HASH));
        }

        let (mut child_id, mut child_hash, mut child_depth) = self.pending_leaf.take().unwrap();
        let prev_stem = self.prev_stem.unwrap_or([0u8; 31]);

        // Collapse the entire remaining spine (bottom-up).
        while let Some(mut entry) = self.spine.pop() {
            let wrapped = self.create_wrappers(
                (child_id, child_hash),
                child_depth,
                entry.depth + 1,
                &prev_stem,
            )?;
            entry.right = Some(wrapped);

            let (hash, node) = Self::finalize_internal(&entry);
            self.buffer_node(entry.node_id, &node)?;

            child_id = entry.node_id;
            child_hash = hash;
            child_depth = entry.depth;
        }

        // If the root is an InternalNode above depth 0, wrap it up to depth 0.
        // For a bare StemNode root (single stem, no InternalNodes), no wrapping.
        let (root_id, root_hash) =
            self.create_wrappers((child_id, child_hash), child_depth, 0, &prev_stem)?;

        // Write root and next_id metadata.
        self.write_buf.push(WriteOp::Put {
            table: self.nodes_table,
            key: Box::from(META_ROOT),
            value: root_id.to_le_bytes().to_vec(),
        });
        self.write_buf.push(WriteOp::Put {
            table: self.nodes_table,
            key: Box::from(META_NEXT_ID),
            value: self.next_id.to_le_bytes().to_vec(),
        });

        self.flush_buffer()?;
        Ok((Some(root_id), root_hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_binary_trie::merkle::merkelize;
    use ethrex_binary_trie::trie::{BinaryTrie, split_key};

    /// No-op backend that discards all writes; used for hash-only tests.
    struct NoOpBackend;

    impl TrieBackend for NoOpBackend {
        fn get(
            &self,
            _table: &'static str,
            _key: &[u8],
        ) -> Result<Option<Vec<u8>>, BinaryTrieError> {
            Ok(None)
        }
        fn write_batch(&self, _ops: Vec<WriteOp>) -> Result<(), BinaryTrieError> {
            Ok(())
        }
        fn full_iterator(
            &self,
            _table: &'static str,
        ) -> Result<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)>>, BinaryTrieError> {
            Ok(Box::new(std::iter::empty()))
        }
    }

    /// Insert entries into both a BulkTrieBuilder and a BinaryTrie, then verify
    /// they produce the same root hash.
    fn verify_bulk_matches_trie(entries: &[([u8; 32], [u8; 32])]) {
        // Sort entries by key.
        let mut sorted = entries.to_vec();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));

        // Reference: BinaryTrie.
        let mut trie = BinaryTrie::new();
        for (key, value) in &sorted {
            trie.insert(*key, *value).unwrap();
        }
        let expected_root = merkelize(&mut trie);

        // Build via BulkTrieBuilder.
        let backend = Arc::new(NoOpBackend);
        let mut builder = BulkTrieBuilder::new(backend, "test");

        let mut i = 0;
        while i < sorted.len() {
            let (stem, sub_index) = split_key(&sorted[i].0);
            let mut values: BTreeMap<u8, [u8; 32]> = BTreeMap::new();
            values.insert(sub_index, sorted[i].1);
            i += 1;
            // Collect additional entries sharing the same stem.
            while i < sorted.len() && sorted[i].0[..31] == stem[..] {
                let (_, si) = split_key(&sorted[i].0);
                values.insert(si, sorted[i].1);
                i += 1;
            }
            builder.insert_stem(stem, &values).unwrap();
        }
        let (_, actual_root) = builder.finish().unwrap();

        assert_eq!(
            expected_root, actual_root,
            "BulkTrieBuilder root hash does not match BinaryTrie root hash"
        );
    }

    #[test]
    fn single_stem() {
        let mut key = [0u8; 32];
        key[0] = 0xAB;
        verify_bulk_matches_trie(&[(key, [1u8; 32])]);
    }

    #[test]
    fn two_stems_diverge_at_bit_0() {
        let mut k1 = [0u8; 32];
        k1[0] = 0x00; // bit 0 = 0
        let mut k2 = [0u8; 32];
        k2[0] = 0x80; // bit 0 = 1
        verify_bulk_matches_trie(&[(k1, [1u8; 32]), (k2, [2u8; 32])]);
    }

    #[test]
    fn three_stems() {
        let mut k1 = [0u8; 32];
        k1[0] = 0x00;
        let mut k2 = [0u8; 32];
        k2[0] = 0x40;
        let mut k3 = [0u8; 32];
        k3[0] = 0x80;
        verify_bulk_matches_trie(&[(k1, [1u8; 32]), (k2, [2u8; 32]), (k3, [3u8; 32])]);
    }

    #[test]
    fn same_stem_multiple_values() {
        let stem = [0xABu8; 31];
        let mut k1 = [0u8; 32];
        k1[..31].copy_from_slice(&stem);
        k1[31] = 0;
        let mut k2 = [0u8; 32];
        k2[..31].copy_from_slice(&stem);
        k2[31] = 1;
        let mut k3 = [0u8; 32];
        k3[..31].copy_from_slice(&stem);
        k3[31] = 5;
        verify_bulk_matches_trie(&[(k1, [10u8; 32]), (k2, [20u8; 32]), (k3, [30u8; 32])]);
    }

    #[test]
    fn deep_shared_prefix() {
        // Two stems sharing 247 bits of prefix, differing only in the last bit.
        let mut stem1 = [0u8; 31];
        stem1[30] = 0x00;
        let mut stem2 = [0u8; 31];
        stem2[30] = 0x01; // last bit differs
        let mut k1 = [0u8; 32];
        k1[..31].copy_from_slice(&stem1);
        let mut k2 = [0u8; 32];
        k2[..31].copy_from_slice(&stem2);
        verify_bulk_matches_trie(&[(k1, [1u8; 32]), (k2, [2u8; 32])]);
    }

    #[test]
    fn random_entries() {
        // Simple deterministic PRNG (LCG).
        let mut rng_state: u64 = 12345;
        let mut next = || -> u8 {
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng_state >> 33) as u8
        };

        let mut entries = Vec::new();
        let mut seen_keys = std::collections::HashSet::new();
        for _ in 0..1000 {
            let mut key = [0u8; 32];
            for b in key.iter_mut() {
                *b = next();
            }
            if seen_keys.insert(key) {
                let mut value = [0u8; 32];
                for b in value.iter_mut() {
                    *b = next();
                }
                entries.push((key, value));
            }
        }
        verify_bulk_matches_trie(&entries);
    }
}
