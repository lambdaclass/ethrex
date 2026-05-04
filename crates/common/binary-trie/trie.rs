use crate::error::BinaryTrieError;
use crate::node::{InternalNode, MAX_DEPTH, Node, NodeId, StemNode, stem_bit};
use crate::node_store::NodeStore;
use crate::proof::{BinaryTrieProof, get_proof_impl};

/// The binary trie. Holds a `NodeStore` for node allocation and a root `NodeId`.
pub struct BinaryTrie {
    pub store: NodeStore,
    pub root: Option<NodeId>,
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
        Self {
            store: NodeStore::new_memory(),
            root: None,
        }
    }

    /// Insert a key-value pair into the trie.
    pub fn insert(&mut self, key: [u8; 32], value: [u8; 32]) -> Result<(), BinaryTrieError> {
        let (stem, sub_index) = split_key(&key);
        self.root = Some(insert_node(
            &mut self.store,
            self.root.take(),
            stem,
            sub_index,
            value,
            0,
        )?);
        Ok(())
    }

    /// Insert multiple values that share the same 31-byte stem in a single trie walk.
    ///
    /// This avoids redundant path traversals when inserting several sub-indices
    /// for the same stem (e.g. basic_data + code_hash for one account).
    pub fn insert_multi(
        &mut self,
        stem: [u8; 31],
        values: &[(u8, [u8; 32])],
    ) -> Result<(), BinaryTrieError> {
        if values.is_empty() {
            return Ok(());
        }
        self.root = Some(insert_node_multi(
            &mut self.store,
            self.root.take(),
            stem,
            values,
            0,
        )?);
        Ok(())
    }

    /// Look up the value for a key, returning None if absent.
    ///
    /// Uses `get_with_promotion` internally: cold cache misses are promoted into
    /// warm_nodes so subsequent reads in the same block avoid cloning.
    pub fn get(&mut self, key: [u8; 32]) -> Option<[u8; 32]> {
        let (stem, sub_index) = split_key(&key);
        get_node(&mut self.store, self.root, &stem, sub_index)
    }

    /// Look up the value for a key using only shared references (no promotion).
    ///
    /// Falls back to the cloning path on cache misses. Used by read paths that
    /// hold only a `&self` reference (e.g. `BinaryTrieWrapper` behind an RwLock).
    pub fn get_shared(&self, key: [u8; 32]) -> Option<[u8; 32]> {
        let (stem, sub_index) = split_key(&key);
        get_node_shared(&self.store, self.root, &stem, sub_index)
    }

    /// Remove the value for a key, returning the previous value if it existed.
    pub fn remove(&mut self, key: [u8; 32]) -> Result<Option<[u8; 32]>, BinaryTrieError> {
        let (stem, sub_index) = split_key(&key);
        let (new_root, removed) = remove_node(&mut self.store, self.root.take(), &stem, sub_index)?;
        self.root = new_root;
        Ok(removed)
    }

    /// Generate a Merkle proof for `key`.
    ///
    /// `state_root()` must be called before this method so that all node hashes
    /// are cached. Returns an error if any sibling node has not been merkelized.
    pub fn get_proof(&self, key: [u8; 32]) -> Result<BinaryTrieProof, BinaryTrieError> {
        get_proof_impl(self, key)
    }

    /// Collect all dirty/freed trie nodes and metadata as `WriteOp`s.
    ///
    /// Used by `BinaryTrieState::prepare_flush` to combine trie and storage_keys
    /// into a single atomic write batch.
    pub fn collect_flush_ops(&mut self) -> Vec<crate::db::WriteOp> {
        self.store.collect_flush_ops(self.root)
    }
}

impl Default for BinaryTrie {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Iterative insert
// ---------------------------------------------------------------------------

/// Walk down the trie to the insertion point, collecting the path of internal
/// nodes. Then handle the leaf (create/update/split), and walk back up doing
/// in-place mutations on the already-dirty ancestors.
///
/// Compared to the recursive version this avoids:
/// - Recursive stack frames (depth can be 20-30+)
/// - take+put round-trip on each ancestor during the return trip
/// - Ancestor updates entirely when inserting into an existing same-stem node
fn insert_node(
    store: &mut NodeStore,
    root_id: Option<NodeId>,
    stem: [u8; 31],
    sub_index: u8,
    value: [u8; 32],
    _depth: usize,
) -> Result<NodeId, BinaryTrieError> {
    // Phase 1: Walk down, collecting (internal_node_id, bit_direction).
    // Internal nodes are taken out and immediately put back (promoting them to
    // dirty). We only read their child pointer - modification happens in phase 3.
    let mut path: Vec<(NodeId, u8)> = Vec::new();
    let mut current = root_id;
    let mut depth = 0usize;

    let (leaf_id, child_changed) = loop {
        match current {
            None => {
                // Empty slot: create a new StemNode.
                let mut stem_node = StemNode::new(stem);
                stem_node.set_value(sub_index, value);
                break (store.create(Node::Stem(stem_node)), true);
            }
            Some(id) => {
                // Fast path: peek at internal nodes without take/put round-trip.
                let bit = stem_bit(&stem, depth);
                if let Some(child) = store.peek_internal_child_and_ensure_dirty(id, bit)? {
                    if depth >= MAX_DEPTH {
                        return Err(BinaryTrieError::MaxDepthExceeded);
                    }
                    path.push((id, bit));
                    current = child;
                    depth += 1;
                    continue;
                }
                // Stem node: need take/put for mutation.
                let existing = store.take(id)?;
                match existing {
                    Node::Stem(mut stem_node) => {
                        if stem_node.stem == stem {
                            // Same stem: update value, ID unchanged.
                            stem_node.set_value(sub_index, value);
                            store.put(id, Node::Stem(stem_node));
                            break (id, false);
                        } else {
                            if depth >= MAX_DEPTH {
                                store.put(id, Node::Stem(stem_node));
                                return Err(BinaryTrieError::MaxDepthExceeded);
                            }
                            store.free(id);
                            let new_id =
                                split_stems(store, stem_node, stem, sub_index, value, depth)?;
                            break (new_id, true);
                        }
                    }
                    Node::Internal(_) => {
                        unreachable!(
                            "peek_internal_child_and_ensure_dirty returned None for Internal node"
                        );
                    }
                }
            }
        }
    };

    // Phase 3: Walk back up, updating ancestors in-place.
    if child_changed {
        // The leaf has a new NodeId - update child pointers up the path.
        let mut child_id = leaf_id;
        for &(ancestor_id, bit) in path.iter().rev() {
            store.update_dirty_child(ancestor_id, bit, child_id);
            child_id = ancestor_id;
        }
    } else {
        // Same-stem update: child pointers are correct, just invalidate hashes.
        // Once we hit an ancestor whose hash is already None, all above are too.
        for &(ancestor_id, _) in path.iter().rev() {
            if !store.invalidate_dirty_hash(ancestor_id) {
                break;
            }
        }
    }

    Ok(if path.is_empty() { leaf_id } else { path[0].0 })
}

/// Like `insert_node` but sets multiple (sub_index, value) pairs on the same stem
/// in a single trie walk. Avoids redundant path traversals for same-stem inserts.
fn insert_node_multi(
    store: &mut NodeStore,
    root_id: Option<NodeId>,
    stem: [u8; 31],
    values: &[(u8, [u8; 32])],
    _depth: usize,
) -> Result<NodeId, BinaryTrieError> {
    debug_assert!(!values.is_empty());

    // Phase 1: Walk down, collecting (internal_node_id, bit_direction).
    let mut path: Vec<(NodeId, u8)> = Vec::new();
    let mut current = root_id;
    let mut depth = 0usize;

    let (leaf_id, child_changed) = loop {
        match current {
            None => {
                // Empty slot: create a new StemNode with all values.
                let mut stem_node = StemNode::new(stem);
                for &(sub_index, value) in values {
                    stem_node.set_value(sub_index, value);
                }
                break (store.create(Node::Stem(stem_node)), true);
            }
            Some(id) => {
                // Fast path: peek at internal nodes without take/put round-trip.
                let bit = stem_bit(&stem, depth);
                if let Some(child) = store.peek_internal_child_and_ensure_dirty(id, bit)? {
                    if depth >= MAX_DEPTH {
                        return Err(BinaryTrieError::MaxDepthExceeded);
                    }
                    path.push((id, bit));
                    current = child;
                    depth += 1;
                    continue;
                }
                // Stem node: need take/put for mutation.
                let existing = store.take(id)?;
                match existing {
                    Node::Stem(mut stem_node) => {
                        if stem_node.stem == stem {
                            // Same stem: set all values, ID unchanged.
                            for &(sub_index, value) in values {
                                stem_node.set_value(sub_index, value);
                            }
                            store.put(id, Node::Stem(stem_node));
                            break (id, false);
                        } else {
                            if depth >= MAX_DEPTH {
                                store.put(id, Node::Stem(stem_node));
                                return Err(BinaryTrieError::MaxDepthExceeded);
                            }
                            store.free(id);
                            let new_id = split_stems_multi(store, stem_node, stem, values, depth)?;
                            break (new_id, true);
                        }
                    }
                    Node::Internal(_) => {
                        unreachable!(
                            "peek_internal_child_and_ensure_dirty returned None for Internal node"
                        );
                    }
                }
            }
        }
    };

    // Phase 3: Walk back up, updating ancestors in-place.
    if child_changed {
        let mut child_id = leaf_id;
        for &(ancestor_id, bit) in path.iter().rev() {
            store.update_dirty_child(ancestor_id, bit, child_id);
            child_id = ancestor_id;
        }
    } else {
        // Same-stem update: just invalidate hashes up the path.
        for &(ancestor_id, _) in path.iter().rev() {
            if !store.invalidate_dirty_hash(ancestor_id) {
                break;
            }
        }
    }

    Ok(if path.is_empty() { leaf_id } else { path[0].0 })
}

/// Create a chain of InternalNodes until the two stems diverge, then place each
/// StemNode in the appropriate child slot.
fn split_stems(
    store: &mut NodeStore,
    existing: StemNode,
    new_stem: [u8; 31],
    new_sub_index: u8,
    new_value: [u8; 32],
    depth: usize,
) -> Result<NodeId, BinaryTrieError> {
    if depth >= MAX_DEPTH {
        return Err(BinaryTrieError::MaxDepthExceeded);
    }

    let existing_bit = stem_bit(&existing.stem, depth);
    let new_bit = stem_bit(&new_stem, depth);

    if existing_bit != new_bit {
        // The stems diverge here — place each in the appropriate child.
        let mut new_stem_node = StemNode::new(new_stem);
        new_stem_node.set_value(new_sub_index, new_value);

        let existing_id = store.create(Node::Stem(existing));
        let new_id = store.create(Node::Stem(new_stem_node));

        let (left, right) = if existing_bit == 0 {
            (Some(existing_id), Some(new_id))
        } else {
            (Some(new_id), Some(existing_id))
        };

        Ok(store.create(Node::Internal(InternalNode::new(left, right))))
    } else {
        // Bits agree at this depth — recurse one level deeper.
        let inner = split_stems(
            store,
            existing,
            new_stem,
            new_sub_index,
            new_value,
            depth + 1,
        )?;
        let (left, right) = if new_bit == 0 {
            (Some(inner), None)
        } else {
            (None, Some(inner))
        };
        Ok(store.create(Node::Internal(InternalNode::new(left, right))))
    }
}

/// Like `split_stems` but the new stem gets multiple values set.
fn split_stems_multi(
    store: &mut NodeStore,
    existing: StemNode,
    new_stem: [u8; 31],
    values: &[(u8, [u8; 32])],
    depth: usize,
) -> Result<NodeId, BinaryTrieError> {
    if depth >= MAX_DEPTH {
        return Err(BinaryTrieError::MaxDepthExceeded);
    }

    let existing_bit = stem_bit(&existing.stem, depth);
    let new_bit = stem_bit(&new_stem, depth);

    if existing_bit != new_bit {
        let mut new_stem_node = StemNode::new(new_stem);
        for &(sub_index, value) in values {
            new_stem_node.set_value(sub_index, value);
        }

        let existing_id = store.create(Node::Stem(existing));
        let new_id = store.create(Node::Stem(new_stem_node));

        let (left, right) = if existing_bit == 0 {
            (Some(existing_id), Some(new_id))
        } else {
            (Some(new_id), Some(existing_id))
        };

        Ok(store.create(Node::Internal(InternalNode::new(left, right))))
    } else {
        let inner = split_stems_multi(store, existing, new_stem, values, depth + 1)?;
        let (left, right) = if new_bit == 0 {
            (Some(inner), None)
        } else {
            (None, Some(inner))
        };
        Ok(store.create(Node::Internal(InternalNode::new(left, right))))
    }
}

// ---------------------------------------------------------------------------
// Recursive get
// ---------------------------------------------------------------------------

fn get_node(
    store: &mut NodeStore,
    node_id: Option<NodeId>,
    stem: &[u8; 31],
    sub_index: u8,
) -> Option<[u8; 32]> {
    get_node_at_depth(store, node_id, stem, sub_index, 0)
}

fn get_node_at_depth(
    store: &mut NodeStore,
    node_id: Option<NodeId>,
    stem: &[u8; 31],
    sub_index: u8,
    depth: usize,
) -> Option<[u8; 32]> {
    let id = node_id?;
    let node = store.get_with_promotion(id).ok()?;
    match node {
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
            let child = if bit == 0 {
                internal.left
            } else {
                internal.right
            };
            get_node_at_depth(store, child, stem, sub_index, depth + 1)
        }
    }
}

// Shared (cloning) read path — used where only `&NodeStore` is available.

fn get_node_shared(
    store: &NodeStore,
    node_id: Option<NodeId>,
    stem: &[u8; 31],
    sub_index: u8,
) -> Option<[u8; 32]> {
    get_node_shared_at_depth(store, node_id, stem, sub_index, 0)
}

fn get_node_shared_at_depth(
    store: &NodeStore,
    node_id: Option<NodeId>,
    stem: &[u8; 31],
    sub_index: u8,
    depth: usize,
) -> Option<[u8; 32]> {
    let id = node_id?;
    let node = store.get(id).ok()?;
    match node {
        Node::Stem(ref stem_node) => {
            if &stem_node.stem == stem {
                stem_node.get_value(sub_index)
            } else {
                None
            }
        }
        Node::Internal(ref internal) => {
            if depth >= MAX_DEPTH {
                return None;
            }
            let bit = stem_bit(stem, depth);
            let child = if bit == 0 {
                internal.left
            } else {
                internal.right
            };
            get_node_shared_at_depth(store, child, stem, sub_index, depth + 1)
        }
    }
}

// ---------------------------------------------------------------------------
// Recursive remove
// ---------------------------------------------------------------------------

/// Returns the updated node ID (None if it should be collapsed) and the removed value.
fn remove_node(
    store: &mut NodeStore,
    node_id: Option<NodeId>,
    stem: &[u8; 31],
    sub_index: u8,
) -> Result<(Option<NodeId>, Option<[u8; 32]>), BinaryTrieError> {
    remove_node_at_depth(store, node_id, stem, sub_index, 0)
}

fn remove_node_at_depth(
    store: &mut NodeStore,
    node_id: Option<NodeId>,
    stem: &[u8; 31],
    sub_index: u8,
    depth: usize,
) -> Result<(Option<NodeId>, Option<[u8; 32]>), BinaryTrieError> {
    let id = match node_id {
        None => return Ok((None, None)),
        Some(id) => id,
    };

    let node = store.take(id)?;

    match node {
        Node::Stem(mut stem_node) => {
            if &stem_node.stem != stem {
                store.put(id, Node::Stem(stem_node));
                return Ok((Some(id), None));
            }
            let removed = stem_node.remove_value(sub_index);
            if stem_node.is_empty() {
                // StemNode is now empty — free it.
                store.free(id);
                Ok((None, removed))
            } else {
                store.put(id, Node::Stem(stem_node));
                Ok((Some(id), removed))
            }
        }

        Node::Internal(mut internal) => {
            if depth >= MAX_DEPTH {
                store.put(id, Node::Internal(internal));
                return Ok((Some(id), None));
            }
            let bit = stem_bit(stem, depth);
            let result = if bit == 0 {
                remove_node_at_depth(store, internal.left, stem, sub_index, depth + 1)
            } else {
                remove_node_at_depth(store, internal.right, stem, sub_index, depth + 1)
            };
            let (new_child, removed) = match result {
                Ok(r) => r,
                Err(e) => {
                    // Put node back before propagating error.
                    store.put(id, Node::Internal(internal));
                    return Err(e);
                }
            };
            if bit == 0 {
                internal.left = new_child;
            } else {
                internal.right = new_child;
            }

            // Collapse: if one child is now None and the survivor is a StemNode,
            // promote it. We must NOT promote a surviving InternalNode because
            // get_node_at_depth uses `depth` to select the traversal bit —
            // promoting an InternalNode to a shallower depth would use the wrong
            // bit, losing data on the other branch.
            internal.cached_hash = None;

            // Check what the surviving child looks like (if any).
            let updated = match (internal.left, internal.right) {
                (None, None) => {
                    store.free(id);
                    None
                }
                (Some(child_id), None) => {
                    let is_stem = store
                        .get(child_id)
                        .map(|n| matches!(n, Node::Stem(_)))
                        .unwrap_or(false);
                    if is_stem {
                        store.free(id);
                        Some(child_id)
                    } else {
                        internal.left = Some(child_id);
                        store.put(id, Node::Internal(internal));
                        Some(id)
                    }
                }
                (None, Some(child_id)) => {
                    let is_stem = store
                        .get(child_id)
                        .map(|n| matches!(n, Node::Stem(_)))
                        .unwrap_or(false);
                    if is_stem {
                        store.free(id);
                        Some(child_id)
                    } else {
                        internal.right = Some(child_id);
                        store.put(id, Node::Internal(internal));
                        Some(id)
                    }
                }
                (Some(l), Some(r)) => {
                    internal.left = Some(l);
                    internal.right = Some(r);
                    store.put(id, Node::Internal(internal));
                    Some(id)
                }
            };
            Ok((updated, removed))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Node;

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
        let mut trie = BinaryTrie::new();
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
        let removed = trie.remove(k).unwrap();
        assert_eq!(removed, Some(val(99)));
        assert_eq!(trie.get(k), None);
    }

    #[test]
    fn remove_absent_returns_none() {
        let mut trie = BinaryTrie::new();
        assert_eq!(trie.remove(key(0x01, 0)).unwrap(), None);
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
        trie.remove(k2).unwrap();
        assert_eq!(trie.get(k1), Some(val(1)));
        assert_eq!(trie.get(k2), None);
        // After collapse the root should be a StemNode directly.
        let root_id = trie.root.unwrap();
        assert!(matches!(trie.store.get(root_id).unwrap(), Node::Stem(_)));
    }

    #[test]
    fn remove_all_empties_trie() {
        let mut trie = BinaryTrie::new();
        let k = key(0xDD, 7);
        trie.insert(k, val(5)).unwrap();
        trie.remove(k).unwrap();
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

        trie.remove(k_01).unwrap();

        // 0x00 and 0x80 should still be present.
        assert_eq!(trie.get(k_00), Some(val(1)));
        assert_eq!(trie.get(k_80), Some(val(3)));
        assert_eq!(trie.get(k_01), None);

        // Root hash should match a fresh trie with only 0x00 and 0x80.
        let mut fresh = BinaryTrie::new();
        fresh.insert(k_00, val(1)).unwrap();
        fresh.insert(k_80, val(3)).unwrap();

        assert_eq!(
            crate::merkle::merkelize(&mut trie),
            crate::merkle::merkelize(&mut fresh)
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

        let removed = trie.remove(k0).unwrap();
        assert_eq!(removed, Some(val(10)));
        assert_eq!(trie.get(k0), None);
        assert_eq!(trie.get(k1), Some(val(20)));
        let root_id = trie.root.unwrap();
        assert!(matches!(trie.store.get(root_id).unwrap(), Node::Stem(_)));
    }

    #[test]
    fn insert_multi_basic() {
        let mut trie = BinaryTrie::new();
        let stem = [0xAB; 31];
        let v0 = [1u8; 32];
        let v1 = [2u8; 32];
        trie.insert_multi(stem, &[(0, v0), (1, v1)]).unwrap();

        let mut key0 = [0u8; 32];
        key0[..31].copy_from_slice(&stem);
        key0[31] = 0;
        let mut key1 = [0u8; 32];
        key1[..31].copy_from_slice(&stem);
        key1[31] = 1;

        assert_eq!(trie.get(key0), Some(v0));
        assert_eq!(trie.get(key1), Some(v1));
    }

    #[test]
    fn insert_multi_into_existing_stem() {
        let mut trie = BinaryTrie::new();
        let stem = [0xCD; 31];
        let v0 = [10u8; 32];
        let v1 = [20u8; 32];
        let v2 = [30u8; 32];

        // Insert one value via regular insert.
        let mut key0 = [0u8; 32];
        key0[..31].copy_from_slice(&stem);
        key0[31] = 0;
        trie.insert(key0, v0).unwrap();

        // Insert two more via insert_multi.
        trie.insert_multi(stem, &[(1, v1), (2, v2)]).unwrap();

        let mut key1 = [0u8; 32];
        key1[..31].copy_from_slice(&stem);
        key1[31] = 1;
        let mut key2 = [0u8; 32];
        key2[..31].copy_from_slice(&stem);
        key2[31] = 2;

        assert_eq!(trie.get(key0), Some(v0));
        assert_eq!(trie.get(key1), Some(v1));
        assert_eq!(trie.get(key2), Some(v2));
    }

    #[test]
    fn insert_multi_causes_split() {
        let mut trie = BinaryTrie::new();
        let stem_a = [0x00; 31]; // bit 0 = 0
        let stem_b = [0x80; 31]; // bit 0 = 1 (different at depth 0)

        let va = [1u8; 32];
        trie.insert_multi(stem_a, &[(0, va)]).unwrap();

        let vb0 = [2u8; 32];
        let vb1 = [3u8; 32];
        trie.insert_multi(stem_b, &[(0, vb0), (1, vb1)]).unwrap();

        let mut key_a0 = [0u8; 32];
        key_a0[..31].copy_from_slice(&stem_a);
        key_a0[31] = 0;
        let mut key_b0 = [0u8; 32];
        key_b0[..31].copy_from_slice(&stem_b);
        key_b0[31] = 0;
        let mut key_b1 = [0u8; 32];
        key_b1[..31].copy_from_slice(&stem_b);
        key_b1[31] = 1;

        assert_eq!(trie.get(key_a0), Some(va));
        assert_eq!(trie.get(key_b0), Some(vb0));
        assert_eq!(trie.get(key_b1), Some(vb1));
    }

    #[test]
    fn insert_multi_single_value() {
        let mut trie_multi = BinaryTrie::new();
        let mut trie_single = BinaryTrie::new();
        let stem = [0x42; 31];
        let v = [7u8; 32];

        let mut key = [0u8; 32];
        key[..31].copy_from_slice(&stem);
        key[31] = 5;

        trie_single.insert(key, v).unwrap();
        trie_multi.insert_multi(stem, &[(5, v)]).unwrap();

        assert_eq!(trie_multi.get(key), trie_single.get(key));
    }

    #[test]
    fn insert_multi_overwrites() {
        let mut trie = BinaryTrie::new();
        let stem = [0x33; 31];
        let old_v = [1u8; 32];
        let new_v = [9u8; 32];

        let mut key = [0u8; 32];
        key[..31].copy_from_slice(&stem);
        key[31] = 0;

        trie.insert(key, old_v).unwrap();
        trie.insert_multi(stem, &[(0, new_v)]).unwrap();

        assert_eq!(trie.get(key), Some(new_v));
    }

    #[test]
    fn insert_multi_empty_is_noop() {
        let mut trie = BinaryTrie::new();
        let stem = [0x11; 31];
        trie.insert_multi(stem, &[]).unwrap();
        assert!(trie.root.is_none());
    }

    #[test]
    fn remove_preserves_siblings_in_deep_tree() {
        // Reproduce the block-7230 bug: StemNode at depth 26 loses sibling
        // values when one sub-index is removed.
        let mut trie = BinaryTrie::new();

        // Target stem: arbitrary 31 bytes
        let target_stem = [
            120u8, 51, 78, 133, 189, 220, 159, 26, 100, 76, 202, 249, 180, 89, 193, 93, 1, 43, 203,
            121, 29, 193, 209, 111, 220, 186, 157, 182, 152, 205, 187,
        ];

        // Create a "neighbor" stem that shares the first 26 bits with target
        // but diverges at bit 26, forcing the target StemNode to depth 26.
        // Bit 26 is in byte 3 (26/8=3), bit position 26%8=2 (counting from MSB).
        let mut neighbor_stem = target_stem;
        // Flip bit 26: byte 3, bit 2 from MSB
        neighbor_stem[3] ^= 0x20; // flip bit at position 2 in byte 3

        // Insert values on the target stem at sub-indices 49, 50, 52
        let mut k49 = [0u8; 32];
        k49[..31].copy_from_slice(&target_stem);
        k49[31] = 49;
        let mut k50 = [0u8; 32];
        k50[..31].copy_from_slice(&target_stem);
        k50[31] = 50;
        let mut k52 = [0u8; 32];
        k52[..31].copy_from_slice(&target_stem);
        k52[31] = 52;

        trie.insert(k52, val(52)).unwrap();
        trie.insert(k50, val(50)).unwrap();
        trie.insert(k49, val(49)).unwrap();

        // Insert neighbor to force depth
        let mut kn = [0u8; 32];
        kn[..31].copy_from_slice(&neighbor_stem);
        kn[31] = 0;
        trie.insert(kn, val(99)).unwrap();

        // Verify all values present
        assert_eq!(trie.get(k49), Some(val(49)));
        assert_eq!(trie.get(k50), Some(val(50)));
        assert_eq!(trie.get(k52), Some(val(52)));
        assert_eq!(trie.get(kn), Some(val(99)));

        // Overwrite k49 then remove it (matching the production pattern)
        trie.insert(k49, val(1)).unwrap();
        assert_eq!(trie.get(k49), Some(val(1)));

        // Remove k49
        let removed = trie.remove(k49).unwrap();
        assert_eq!(removed, Some(val(1)));

        // THE BUG: siblings on the same stem must survive
        assert_eq!(trie.get(k49), None, "k49 should be removed");
        assert_eq!(trie.get(k50), Some(val(50)), "k50 should survive");
        assert_eq!(trie.get(k52), Some(val(52)), "k52 should survive");
        assert_eq!(trie.get(kn), Some(val(99)), "neighbor should survive");
    }
}
