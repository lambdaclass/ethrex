use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::db::{TrieBackend, WriteOp};
use crate::error::BinaryTrieError;
use crate::node::{InternalNode, Node, NodeId, STEM_VALUES, StemNode};

/// Default maximum number of clean nodes kept in the LRU cache.
///
/// Clean LRU cache capacity. InternalNode ~100 bytes, StemNode ~200 bytes
/// with sparse values. Combined with dirty_nodes and warm_nodes (unbounded
/// HashMaps), total memory is hard to predict. 100K entries keeps memory
/// bounded while still caching the hot working set. Nodes evicted from the
/// LRU are loaded from the backend on demand.
const DEFAULT_CLEAN_CACHE_CAP: usize = 100_000;

// Meta keys for storage (stored alongside nodes in BINARY_TRIE_NODES table).
// The 0xFF prefix ensures they don't collide with u64 node IDs (8 bytes, no prefix).
const META_ROOT: &[u8] = &[0xFF, b'R'];
const META_NEXT_ID: &[u8] = &[0xFF, b'N'];

/// Returns an 8-byte key for a node: raw `id` as little-endian u64.
fn node_key(id: NodeId) -> [u8; 8] {
    id.to_le_bytes()
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

/// Serialize a node to bytes.
///
/// InternalNode (tag 0x01): `[0x01, left_id: u64 LE, right_id: u64 LE]` = 17 bytes
///   - `None` child is encoded as 0 (NodeId 0 is never allocated).
///
/// StemNode (tag 0x02): `[0x02, stem: 31 bytes, presence_bitmap: 32 bytes, values...]`
///   - `presence_bitmap`: bit i (0-indexed from the LSB of byte 0) is set if `values[i].is_some()`.
///   - Only present values are serialized, in order of index.
///   - `cached_hash` is not serialized (recomputed on demand).
fn serialize_node(node: &Node) -> Vec<u8> {
    match node {
        Node::Internal(internal) => {
            let mut buf = Vec::with_capacity(17);
            buf.push(0x01);
            buf.extend_from_slice(&internal.left.unwrap_or(0).to_le_bytes());
            buf.extend_from_slice(&internal.right.unwrap_or(0).to_le_bytes());
            buf
        }
        Node::Stem(stem) => {
            // Single pass: build bitmap and collect values simultaneously.
            // BTreeMap iterates in ascending key order, which matches the bitmap's
            // bit order (bit i set iff values[i] is present).
            let mut bitmap = [0u8; 32];
            let mut values_buf: Vec<u8> = Vec::with_capacity(stem.values.len() * 32);
            for (&idx, v) in stem.values.iter() {
                bitmap[idx as usize / 8] |= 1 << (idx as usize % 8);
                values_buf.extend_from_slice(v);
            }

            let mut buf = Vec::with_capacity(1 + 31 + 32 + values_buf.len());
            buf.push(0x02);
            buf.extend_from_slice(&stem.stem);
            buf.extend_from_slice(&bitmap);
            buf.extend_from_slice(&values_buf);
            buf
        }
    }
}

/// Deserialize a node from bytes.
fn deserialize_node(bytes: &[u8]) -> Result<Node, BinaryTrieError> {
    if bytes.is_empty() {
        return Err(BinaryTrieError::DeserializationError(
            "empty bytes".to_string(),
        ));
    }

    match bytes[0] {
        0x01 => {
            // InternalNode: tag(1) + left(8) + right(8) = 17 bytes
            if bytes.len() < 17 {
                return Err(BinaryTrieError::DeserializationError(format!(
                    "InternalNode too short: {} bytes",
                    bytes.len()
                )));
            }
            let left_id = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
            let right_id = u64::from_le_bytes(bytes[9..17].try_into().unwrap());
            Ok(Node::Internal(InternalNode {
                left: if left_id == 0 { None } else { Some(left_id) },
                right: if right_id == 0 { None } else { Some(right_id) },
                cached_hash: None,
            }))
        }
        0x02 => {
            // StemNode: tag(1) + stem(31) + bitmap(32) + values(N*32)
            // Minimum: 1 + 31 + 32 = 64 bytes (no values present)
            if bytes.len() < 64 {
                return Err(BinaryTrieError::DeserializationError(format!(
                    "StemNode too short: {} bytes",
                    bytes.len()
                )));
            }
            let stem: [u8; 31] = bytes[1..32].try_into().unwrap();
            let bitmap: [u8; 32] = bytes[32..64].try_into().unwrap();

            // Reconstruct the values map from the bitmap and packed data.
            let mut values = std::collections::BTreeMap::new();
            let mut offset = 64usize;
            for i in 0..STEM_VALUES {
                let byte = bitmap[i / 8];
                let bit = (byte >> (i % 8)) & 1;
                if bit == 1 {
                    if offset + 32 > bytes.len() {
                        return Err(BinaryTrieError::DeserializationError(format!(
                            "StemNode data truncated at value index {i}"
                        )));
                    }
                    let mut v = [0u8; 32];
                    v.copy_from_slice(&bytes[offset..offset + 32]);
                    values.insert(i as u8, v);
                    offset += 32;
                }
            }

            Ok(Node::Stem(StemNode {
                stem,
                values,
                cached_hash: None,
            }))
        }
        tag => Err(BinaryTrieError::DeserializationError(format!(
            "unknown node tag: 0x{tag:02X}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// NodeStore
// ---------------------------------------------------------------------------

/// In-memory (and optionally backend-persisted) store for binary trie nodes.
///
/// Nodes are identified by a stable `NodeId` (a monotonically increasing u64
/// starting at 1). The value 0 is reserved as the "None" sentinel used in the
/// serialized InternalNode format.
///
/// ## Write path
/// - `create` allocates a new ID, inserts into cache, marks dirty.
/// - `put` updates an existing node in cache, marks dirty.
/// - `put_clean` updates in cache without marking dirty (used for cached_hash updates).
/// - `free` schedules a node for deletion on the next `flush`.
///
/// ## Read path
/// - `get` returns a shared reference; loads from the backend on a cache miss.
/// - `take` removes the node from the cache so the caller can modify it before
///   calling `put` or `free`.
pub struct NodeStore {
    /// Dirty (modified) nodes — guaranteed to stay in memory until flush.
    dirty_nodes: FxHashMap<NodeId, Node>,
    /// Tracks which node IDs are dirty. Survives `take`/`put_clean` cycles
    /// (e.g. during merkelization) so that `put_clean` routes the node back
    /// to `dirty_nodes` instead of the clean LRU cache.
    dirty_ids: FxHashSet<NodeId>,
    /// Warm (recently flushed) nodes — read-only, from the previous checkpoint
    /// interval. Avoids the post-checkpoint cold-start by keeping the hot
    /// working set in memory without LRU churn.
    warm_nodes: FxHashMap<NodeId, Node>,
    /// Clean (read-only) nodes — LRU-evicted when the cap is reached.
    /// Wrapped in a Mutex so that cache population on read can occur with `&self`.
    clean_cache: Mutex<LruCache<NodeId, Node>>,
    /// IDs of nodes scheduled for deletion on the next flush.
    freed: FxHashSet<NodeId>,
    next_id: NodeId,
    /// Persistence backend (None for pure in-memory operation).
    backend: Option<Arc<dyn TrieBackend>>,
    /// Name of the table used for nodes and metadata.
    pub nodes_table: &'static str,
}

impl NodeStore {
    /// Create a pure in-memory NodeStore (no persistence).
    pub fn new_memory() -> Self {
        Self {
            dirty_nodes: FxHashMap::default(),
            dirty_ids: FxHashSet::default(),
            warm_nodes: FxHashMap::default(),
            clean_cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(DEFAULT_CLEAN_CACHE_CAP).unwrap(),
            )),
            freed: FxHashSet::default(),
            next_id: 1,
            backend: None,
            nodes_table: "",
        }
    }

    /// Open a persistent NodeStore backed by a `TrieBackend`.
    ///
    /// Reads `next_id` from the `META_NEXT_ID` key in `nodes_table`. If absent, starts at 1.
    pub fn open(
        backend: Arc<dyn TrieBackend>,
        nodes_table: &'static str,
    ) -> Result<Self, BinaryTrieError> {
        let next_id = match backend.get(nodes_table, META_NEXT_ID)? {
            Some(bytes) if bytes.len() >= 8 => u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            _ => 1,
        };

        Ok(Self {
            dirty_nodes: FxHashMap::default(),
            dirty_ids: FxHashSet::default(),
            warm_nodes: FxHashMap::default(),
            clean_cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(DEFAULT_CLEAN_CACHE_CAP).unwrap(),
            )),
            freed: FxHashSet::default(),
            next_id,
            backend: Some(backend),
            nodes_table,
        })
    }

    /// Load the persisted root NodeId from the backend, if any.
    pub fn load_root(&self) -> Option<NodeId> {
        let backend = self.backend.as_ref()?;
        let bytes = backend.get(self.nodes_table, META_ROOT).ok()??;
        if bytes.len() < 8 {
            return None;
        }
        let id = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        if id == 0 { None } else { Some(id) }
    }

    /// Allocate the next node ID.
    fn alloc_id(&mut self) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Create a new node, assign it an ID, insert into dirty map.
    pub fn create(&mut self, node: Node) -> NodeId {
        let id = self.alloc_id();
        self.dirty_nodes.insert(id, node);
        self.dirty_ids.insert(id);
        id
    }

    /// Get a node by ID, returning it by value (cloned).
    ///
    /// Lookup order: dirty_nodes → warm_nodes → clean_cache → backend.
    ///
    /// Takes `&self` so read paths (trie traversal) do not require `&mut`.
    /// Cache population on a miss uses an internal `Mutex`.
    pub fn get(&self, id: NodeId) -> Result<Node, BinaryTrieError> {
        if let Some(node) = self.dirty_nodes.get(&id) {
            return Ok(node.clone());
        }
        if let Some(node) = self.warm_nodes.get(&id) {
            return Ok(node.clone());
        }
        // Check clean cache (brief lock); populate on miss.
        {
            let mut cache = self.clean_cache.lock().unwrap();
            if let Some(node) = cache.get(&id) {
                return Ok(node.clone());
            }
        }
        {
            let node = self.load_from_db(id)?;
            let cloned = node.clone();
            self.clean_cache.lock().unwrap().put(id, node);
            Ok(cloned)
        }
    }

    /// Get a shared reference to a node without cloning.
    ///
    /// Checks dirty_nodes and warm_nodes (both plain HashMaps) first. On a miss,
    /// loads from the LRU/backend into warm_nodes and returns a reference to it.
    /// This avoids cloning on every step of a trie walk.
    ///
    /// Lookup order: dirty_nodes → warm_nodes → clean_cache → backend.
    pub fn get_with_promotion(&mut self, id: NodeId) -> Result<&Node, BinaryTrieError> {
        if self.dirty_nodes.contains_key(&id) {
            return Ok(self.dirty_nodes.get(&id).unwrap());
        }
        if self.warm_nodes.contains_key(&id) {
            return Ok(self.warm_nodes.get(&id).unwrap());
        }
        // Cache miss: load from LRU or backend, promote into warm_nodes so
        // subsequent reads in the same traversal are reference-cheap without
        // marking the node dirty.
        let node = {
            let cache = self.clean_cache.get_mut().unwrap();
            if let Some(n) = cache.pop(&id) {
                n
            } else {
                self.load_from_db(id)?
            }
        };
        self.warm_nodes.insert(id, node);
        Ok(self.warm_nodes.get(&id).unwrap())
    }

    /// Get a shared reference to a node by ID, populating the cache on miss.
    ///
    /// Used by mutation paths (insert, remove, merkelize) where callers need
    /// a reference rather than an owned clone. Takes `&mut self` to allow
    /// cache insertion without the overhead of a Mutex lock.
    ///
    /// Lookup order: dirty_nodes → warm_nodes → clean_cache → backend.
    pub fn get_mut(&mut self, id: NodeId) -> Result<&Node, BinaryTrieError> {
        if let Some(node) = self.dirty_nodes.get(&id) {
            return Ok(node);
        }
        if let Some(node) = self.warm_nodes.get(&id) {
            return Ok(node);
        }
        // Check clean cache; on miss, load from backend.
        if !self.clean_cache.get_mut().unwrap().contains(&id) {
            let node = self.load_from_db(id)?;
            self.clean_cache.get_mut().unwrap().put(id, node);
        }
        Ok(self.clean_cache.get_mut().unwrap().get(&id).unwrap())
    }

    /// Remove a node from the store and return it.
    ///
    /// The caller must return the node via `put` or `free` on all code paths.
    pub fn take(&mut self, id: NodeId) -> Result<Node, BinaryTrieError> {
        if let Some(node) = self.dirty_nodes.remove(&id) {
            return Ok(node);
        }
        if let Some(node) = self.warm_nodes.remove(&id) {
            return Ok(node);
        }
        if let Some(node) = self.clean_cache.get_mut().unwrap().pop(&id) {
            return Ok(node);
        }
        self.load_from_db(id)
    }

    /// Put a node back (or update an existing one). Marks the node dirty.
    pub fn put(&mut self, id: NodeId, node: Node) {
        // If already dirty, it cannot be in warm or clean tiers — skip the
        // hash lookups for those maps (common case during mutation).
        if !self.dirty_ids.contains(&id) {
            self.warm_nodes.remove(&id);
            self.clean_cache.get_mut().unwrap().pop(&id);
            self.freed.remove(&id);
        }
        self.dirty_nodes.insert(id, node);
        self.dirty_ids.insert(id);
    }

    /// Put a node back without marking it dirty.
    ///
    /// Uses `dirty_ids` (not `dirty_nodes`) to check dirtiness, because the
    /// node may have been temporarily removed by `take` during merkelization.
    pub fn put_clean(&mut self, id: NodeId, node: Node) {
        if self.dirty_ids.contains(&id) {
            self.dirty_nodes.insert(id, node);
        } else {
            self.clean_cache.get_mut().unwrap().put(id, node);
        }
    }

    /// Schedule a node for deletion on the next `flush`.
    pub fn free(&mut self, id: NodeId) {
        self.dirty_nodes.remove(&id);
        self.dirty_ids.remove(&id);
        self.warm_nodes.remove(&id);
        self.clean_cache.get_mut().unwrap().pop(&id);
        self.freed.insert(id);
    }

    /// Collect all dirty and freed nodes plus metadata as `WriteOp`s.
    ///
    /// Used by `BinaryTrieState::prepare_flush` to build a single atomic batch
    /// that also contains storage_keys entries.
    ///
    /// After collecting, performs generational rotation (dirty → warm → LRU).
    pub fn collect_flush_ops(&mut self, root: Option<NodeId>) -> Vec<WriteOp> {
        let mut ops = Vec::with_capacity(self.dirty_nodes.len() + self.freed.len() + 2);

        // Write all dirty nodes.
        for (id, node) in &self.dirty_nodes {
            ops.push(WriteOp::Put {
                table: self.nodes_table,
                key: Box::from(node_key(*id)),
                value: serialize_node(node),
            });
        }

        // Delete all freed nodes.
        for &id in &self.freed {
            ops.push(WriteOp::Delete {
                table: self.nodes_table,
                key: Box::from(node_key(id)),
            });
        }

        // Write root metadata.
        ops.push(WriteOp::Put {
            table: self.nodes_table,
            key: Box::from(META_ROOT),
            value: root.unwrap_or(0).to_le_bytes().to_vec(),
        });

        // Write next_id metadata.
        ops.push(WriteOp::Put {
            table: self.nodes_table,
            key: Box::from(META_NEXT_ID),
            value: self.next_id.to_le_bytes().to_vec(),
        });

        self.rotate_generations();
        self.freed.clear();

        ops
    }

    /// Generational rotation: old warm → LRU, dirty → warm.
    ///
    /// This avoids the post-checkpoint cold-start. The just-flushed dirty
    /// nodes become the warm pool (fast HashMap lookups, no LRU churn).
    /// The previous warm pool (now 2 intervals old) is demoted to the LRU.
    fn rotate_generations(&mut self) {
        let cache = self.clean_cache.get_mut().unwrap();
        // Demote old warm nodes into the LRU (strip caches to save memory).
        for (id, mut node) in self.warm_nodes.drain() {
            node.strip_caches();
            cache.put(id, node);
        }
        // Move dirty nodes into warm (strip subtrees, keep as hot read-only).
        self.warm_nodes = std::mem::take(&mut self.dirty_nodes);
        for node in self.warm_nodes.values_mut() {
            node.strip_caches();
        }
        self.dirty_ids.clear();
    }

    /// Return the next ID that will be allocated (for testing/debugging).
    pub fn next_id(&self) -> NodeId {
        self.next_id
    }

    /// Return the number of clean nodes in the LRU cache.
    pub fn clean_cache_len(&self) -> usize {
        self.clean_cache.lock().unwrap().len()
    }

    /// Move the top `levels` of the trie from warm_nodes into the LRU cache
    /// so they survive `clear_warm_nodes`. These nodes are touched by every
    /// insert and re-reading them from RocksDB after each flush is wasteful.
    pub fn pin_top_levels(&mut self, root: Option<NodeId>, levels: usize) {
        let Some(root_id) = root else { return };
        let cache = self.clean_cache.get_mut().unwrap();
        let mut queue = vec![root_id];
        for _ in 0..levels {
            let mut next_level = Vec::new();
            for id in queue {
                if let Some(node) = self.warm_nodes.remove(&id) {
                    if let Node::Internal(ref internal) = node {
                        if let Some(left) = internal.left {
                            next_level.push(left);
                        }
                        if let Some(right) = internal.right {
                            next_level.push(right);
                        }
                    }
                    cache.put(id, node);
                }
            }
            queue = next_level;
        }
    }

    /// Drop all warm nodes, freeing memory. Use during bulk imports where
    /// re-reads are rare and bounded memory matters more than cache hits.
    pub fn clear_warm_nodes(&mut self) {
        self.warm_nodes.clear();
        self.warm_nodes.shrink_to_fit();
    }

    /// Return the number of warm (recently flushed) nodes.
    pub fn warm_len(&self) -> usize {
        self.warm_nodes.len()
    }

    /// Return the number of dirty nodes pending flush.
    pub fn dirty_len(&self) -> usize {
        self.dirty_nodes.len()
    }

    /// Return the number of freed nodes pending flush.
    pub fn freed_len(&self) -> usize {
        self.freed.len()
    }

    /// Read an internal node's child pointer and ensure the node is dirty.
    ///
    /// For already-dirty nodes this is a single HashMap lookup (no remove+reinsert).
    /// For non-dirty nodes it loads and promotes to dirty.
    ///
    /// Returns:
    /// - `Ok(Some(child_id))` if the node is Internal (child in direction `bit`)
    /// - `Ok(None)` if the node is a Stem (caller should use `take` instead)
    pub fn peek_internal_child_and_ensure_dirty(
        &mut self,
        id: NodeId,
        bit: u8,
    ) -> Result<Option<Option<NodeId>>, BinaryTrieError> {
        // Fast path: already dirty, just read the child pointer.
        if let Some(Node::Internal(internal)) = self.dirty_nodes.get(&id) {
            let child = if bit == 0 {
                internal.left
            } else {
                internal.right
            };
            return Ok(Some(child));
        }
        if self.dirty_ids.contains(&id) {
            // Dirty but it's a Stem node.
            return Ok(None);
        }

        // Not dirty: load from warm/clean/backend and promote.
        let node = if let Some(node) = self.warm_nodes.remove(&id) {
            node
        } else if let Some(node) = self.clean_cache.get_mut().unwrap().pop(&id) {
            node
        } else {
            self.load_from_db(id)?
        };

        let result = if let Node::Internal(ref internal) = node {
            let child = if bit == 0 {
                internal.left
            } else {
                internal.right
            };
            Some(child)
        } else {
            None
        };

        self.dirty_nodes.insert(id, node);
        self.dirty_ids.insert(id);
        Ok(result)
    }

    /// Update a dirty internal node's child pointer and clear its cached hash
    /// in-place, without removing and reinserting into the HashMap.
    ///
    /// Returns `true` if the node was found in dirty_nodes and updated.
    pub fn update_dirty_child(&mut self, id: NodeId, bit: u8, child_id: NodeId) -> bool {
        if let Some(Node::Internal(internal)) = self.dirty_nodes.get_mut(&id) {
            if bit == 0 {
                internal.left = Some(child_id);
            } else {
                internal.right = Some(child_id);
            }
            internal.cached_hash = None;
            true
        } else {
            false
        }
    }

    /// Clear the cached hash of a dirty internal node in-place.
    ///
    /// Returns `true` if the hash was present and cleared, `false` if already
    /// `None` or the node was not found in dirty_nodes.
    pub fn invalidate_dirty_hash(&mut self, id: NodeId) -> bool {
        if let Some(Node::Internal(internal)) = self.dirty_nodes.get_mut(&id) {
            if internal.cached_hash.is_some() {
                internal.cached_hash = None;
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn load_from_db(&self, id: NodeId) -> Result<Node, BinaryTrieError> {
        let backend = self
            .backend
            .as_ref()
            .ok_or(BinaryTrieError::NodeNotFound(id))?;
        let key = node_key(id);
        match backend.get(self.nodes_table, &key)? {
            Some(bytes) => deserialize_node(&bytes),
            None => Err(BinaryTrieError::NodeNotFound(id)),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{InternalNode, StemNode};

    fn make_stem(byte: u8) -> StemNode {
        let mut stem = [0u8; 31];
        stem[0] = byte;
        StemNode::new(stem)
    }

    // --- create / get / take / put / free cycle ---

    #[test]
    fn create_and_get_internal() {
        let mut store = NodeStore::new_memory();
        let id = store.create(Node::Internal(InternalNode::new(None, None)));
        assert_eq!(id, 1);

        let node = store.get(id).unwrap();
        assert!(matches!(node, Node::Internal(_)));
    }

    #[test]
    fn create_and_get_stem() {
        let mut store = NodeStore::new_memory();
        let stem_node = make_stem(0xAB);
        let id = store.create(Node::Stem(stem_node));
        assert_eq!(id, 1);

        let node = store.get(id).unwrap();
        assert!(matches!(node, Node::Stem(_)));
    }

    #[test]
    fn ids_are_monotonically_increasing() {
        let mut store = NodeStore::new_memory();
        let id1 = store.create(Node::Internal(InternalNode::new(None, None)));
        let id2 = store.create(Node::Internal(InternalNode::new(None, None)));
        let id3 = store.create(Node::Internal(InternalNode::new(None, None)));
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
        assert_eq!(store.next_id(), 4);
    }

    #[test]
    fn take_removes_from_cache() {
        let mut store = NodeStore::new_memory();
        let id = store.create(Node::Internal(InternalNode::new(None, None)));
        let _node = store.take(id).unwrap();
        // Node is gone from cache; get() should fail (no backend).
        assert!(matches!(
            store.get(id),
            Err(BinaryTrieError::NodeNotFound(_))
        ));
    }

    #[test]
    fn put_after_take() {
        let mut store = NodeStore::new_memory();
        let id = store.create(Node::Internal(InternalNode::new(None, None)));
        let mut node = store.take(id).unwrap();

        // Mutate the node.
        if let Node::Internal(ref mut internal) = node {
            internal.cached_hash = Some([0xAB; 32]);
        }
        store.put(id, node);

        let node_back = store.get(id).unwrap();
        if let Node::Internal(internal) = node_back {
            assert_eq!(internal.cached_hash, Some([0xAB; 32]));
        } else {
            panic!("expected Internal");
        }
    }

    #[test]
    fn put_clean_does_not_mark_dirty() {
        let mut store = NodeStore::new_memory();
        let id = store.create(Node::Internal(InternalNode::new(None, None)));
        // Rotate generations (simulates a flush).
        store.rotate_generations();

        store.put_clean(id, Node::Internal(InternalNode::new(Some(2), Some(3))));
        assert!(!store.dirty_ids.contains(&id));
        assert!(!store.dirty_nodes.contains_key(&id));
        assert!(store.clean_cache.lock().unwrap().contains(&id));
    }
}
