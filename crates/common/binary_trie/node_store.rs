use std::num::NonZeroUsize;
use std::sync::Mutex;

use lru::LruCache;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::BinaryTrieError;
#[cfg(any(test, feature = "rocksdb"))]
use crate::node::{InternalNode, STEM_VALUES, StemNode};
use crate::node::{Node, NodeId};

/// Default maximum number of clean nodes kept in the LRU cache.
///
/// With sparse StemNode values, node sizes are: InternalNode ≈ 65 bytes,
/// StemNode ≈ 450 bytes (BTreeMap overhead + 1-5 values). A cap of 2M
/// gives roughly 2M * ~250 bytes avg ≈ ~500 MB.
const DEFAULT_CLEAN_CACHE_CAP: usize = 2_000_000;

// Meta keys for RocksDB storage (stored alongside nodes in BINARY_TRIE_NODES CF).
// The 0xFF prefix ensures they don't collide with u64 node IDs (8 bytes, no prefix).
#[cfg(feature = "rocksdb")]
const META_ROOT: &[u8] = &[0xFF, b'R'];
#[cfg(feature = "rocksdb")]
const META_NEXT_ID: &[u8] = &[0xFF, b'N'];

/// Returns an 8-byte key for a node: raw `id` as little-endian u64.
#[cfg(any(test, feature = "rocksdb"))]
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
///   - `cached_hash` and `cached_subtree` are not serialized (they are recomputed on demand).
#[cfg(any(test, feature = "rocksdb"))]
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
            // Build presence bitmap: 256 bits = 32 bytes.
            let mut bitmap = [0u8; 32];
            for &idx in stem.values.keys() {
                bitmap[idx as usize / 8] |= 1 << (idx as usize % 8);
            }

            let mut buf = Vec::with_capacity(1 + 31 + 32 + stem.values.len() * 32);
            buf.push(0x02);
            buf.extend_from_slice(&stem.stem);
            buf.extend_from_slice(&bitmap);
            // BTreeMap iterates in key order, matching bitmap bit order.
            for v in stem.values.values() {
                buf.extend_from_slice(v);
            }
            buf
        }
    }
}

/// Deserialize a node from bytes.
#[cfg(any(test, feature = "rocksdb"))]
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
                cached_subtree: None,
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

/// In-memory (and optionally RocksDB-backed) store for binary trie nodes.
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
/// - `get` returns a shared reference; loads from RocksDB on a cache miss.
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
    #[cfg(feature = "rocksdb")]
    db: Option<std::sync::Arc<rocksdb::DBWithThreadMode<rocksdb::MultiThreaded>>>,
    /// Name of the column family used for nodes and metadata.
    #[cfg(feature = "rocksdb")]
    pub nodes_cf: &'static str,
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
            #[cfg(feature = "rocksdb")]
            db: None,
            #[cfg(feature = "rocksdb")]
            nodes_cf: "",
        }
    }

    /// Open a persistent NodeStore backed by a shared RocksDB instance.
    ///
    /// Reads `next_id` from the `META_NEXT_ID` key in `nodes_cf`. If absent, starts at 1.
    #[cfg(feature = "rocksdb")]
    pub fn open(
        db: std::sync::Arc<rocksdb::DBWithThreadMode<rocksdb::MultiThreaded>>,
        nodes_cf: &'static str,
    ) -> Result<Self, BinaryTrieError> {
        let next_id = {
            let cf = db
                .cf_handle(nodes_cf)
                .ok_or_else(|| BinaryTrieError::StoreError(format!("CF '{nodes_cf}' not found")))?;
            match db
                .get_cf(&cf, META_NEXT_ID)
                .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?
            {
                Some(bytes) if bytes.len() >= 8 => {
                    u64::from_le_bytes(bytes[..8].try_into().unwrap())
                }
                _ => 1,
            }
            // cf is dropped here, releasing the borrow on db
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
            db: Some(db),
            nodes_cf,
        })
    }

    /// Load the persisted root NodeId from the database, if any.
    #[cfg(feature = "rocksdb")]
    pub fn load_root(&self) -> Option<NodeId> {
        let db = self.db.as_ref()?;
        let cf = db.cf_handle(self.nodes_cf)?;
        let bytes = db.get_cf(&cf, META_ROOT).ok()??;
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
    /// Lookup order: dirty_nodes → warm_nodes → clean_cache → RocksDB.
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
        #[cfg(feature = "rocksdb")]
        {
            let node = self.load_from_db(id)?;
            let cloned = node.clone();
            self.clean_cache.lock().unwrap().put(id, node);
            Ok(cloned)
        }
        #[cfg(not(feature = "rocksdb"))]
        Err(BinaryTrieError::NodeNotFound(id))
    }

    /// Get a shared reference to a node by ID, populating the cache on miss.
    ///
    /// Used by mutation paths (insert, remove, merkelize) where callers need
    /// a reference rather than an owned clone. Takes `&mut self` to allow
    /// cache insertion without the overhead of a Mutex lock.
    ///
    /// Lookup order: dirty_nodes → warm_nodes → clean_cache → RocksDB.
    pub fn get_mut(&mut self, id: NodeId) -> Result<&Node, BinaryTrieError> {
        if self.dirty_nodes.contains_key(&id) {
            return Ok(self.dirty_nodes.get(&id).unwrap());
        }
        if self.warm_nodes.contains_key(&id) {
            return Ok(self.warm_nodes.get(&id).unwrap());
        }
        // Check clean cache; on miss, load from DB.
        if !self.clean_cache.get_mut().unwrap().contains(&id) {
            #[cfg(feature = "rocksdb")]
            {
                let node = self.load_from_db(id)?;
                self.clean_cache.get_mut().unwrap().put(id, node);
            }
            #[cfg(not(feature = "rocksdb"))]
            {
                return Err(BinaryTrieError::NodeNotFound(id));
            }
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
        #[cfg(feature = "rocksdb")]
        {
            self.load_from_db(id)
        }
        #[cfg(not(feature = "rocksdb"))]
        {
            Err(BinaryTrieError::NodeNotFound(id))
        }
    }

    /// Put a node back (or update an existing one). Marks the node dirty.
    pub fn put(&mut self, id: NodeId, node: Node) {
        self.warm_nodes.remove(&id);
        self.clean_cache.get_mut().unwrap().pop(&id);
        self.dirty_nodes.insert(id, node);
        self.dirty_ids.insert(id);
        self.freed.remove(&id);
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

    /// Write dirty and freed nodes plus metadata into a caller-supplied
    /// `WriteBatch`. Used by `BinaryTrieState::flush` to build a single atomic
    /// batch that also contains storage_keys entries.
    ///
    /// All writes go to `nodes_cf` via CF-aware `put_cf`/`delete_cf`.
    /// After writing, performs generational rotation (dirty → warm → LRU).
    #[cfg(feature = "rocksdb")]
    pub fn flush_to_batch(
        &mut self,
        batch: &mut rocksdb::WriteBatch,
        nodes_cf: &impl rocksdb::AsColumnFamilyRef,
        root: Option<NodeId>,
    ) {
        // Write all dirty nodes.
        for (id, node) in &self.dirty_nodes {
            let key = node_key(*id);
            let bytes = serialize_node(node);
            batch.put_cf(nodes_cf, key, bytes);
        }

        // Delete all freed nodes.
        for &id in &self.freed {
            batch.delete_cf(nodes_cf, node_key(id));
        }

        // Write root metadata.
        let root_bytes = root.unwrap_or(0).to_le_bytes();
        batch.put_cf(nodes_cf, META_ROOT, root_bytes);

        // Write next_id metadata.
        batch.put_cf(nodes_cf, META_NEXT_ID, self.next_id.to_le_bytes());

        self.rotate_generations();
        self.freed.clear();
    }

    /// Strip subtree caches from all dirty nodes to reduce memory.
    ///
    /// Called after `state_root()` — the subtree caches are only needed during
    /// merkelization and will be rebuilt on the next call if needed. This
    /// reduces dirty StemNodes from ~25KB to ~8.5KB each.
    pub fn strip_dirty_subtrees(&mut self) {
        for node in self.dirty_nodes.values_mut() {
            if let Node::Stem(stem) = node {
                stem.cached_subtree = None;
            }
        }
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

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    #[cfg(feature = "rocksdb")]
    fn load_from_db(&self, id: NodeId) -> Result<Node, BinaryTrieError> {
        let db = self.db.as_ref().ok_or(BinaryTrieError::NodeNotFound(id))?;
        let cf = db.cf_handle(self.nodes_cf).ok_or_else(|| {
            BinaryTrieError::StoreError(format!("CF '{}' not found", self.nodes_cf))
        })?;
        let key = node_key(id);
        match db
            .get_cf(&cf, &key)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?
        {
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
        // Node is gone from cache; get() should fail (no DB).
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

    #[test]
    fn put_clean_keeps_dirty_node_in_dirty_map() {
        let mut store = NodeStore::new_memory();
        let id = store.create(Node::Internal(InternalNode::new(None, None)));

        // Simulate take/put_clean cycle (as merkelization does).
        let node = store.take(id).unwrap();
        store.put_clean(id, node);

        // Node should still be in dirty_nodes (not demoted to clean_cache).
        assert!(store.dirty_ids.contains(&id));
        assert!(store.dirty_nodes.contains_key(&id));
        assert!(!store.clean_cache.lock().unwrap().contains(&id));
    }

    #[test]
    fn rotate_generations_moves_dirty_to_warm() {
        let mut store = NodeStore::new_memory();
        let id = store.create(Node::Internal(InternalNode::new(None, None)));
        assert!(store.dirty_nodes.contains_key(&id));

        store.rotate_generations();

        assert!(!store.dirty_nodes.contains_key(&id));
        assert!(store.warm_nodes.contains_key(&id));
        assert!(!store.dirty_ids.contains(&id));

        // Second rotation moves warm to clean cache.
        store.rotate_generations();
        assert!(!store.warm_nodes.contains_key(&id));
        assert!(store.clean_cache.lock().unwrap().contains(&id));
    }

    #[test]
    fn free_schedules_deletion() {
        let mut store = NodeStore::new_memory();
        let id = store.create(Node::Internal(InternalNode::new(None, None)));
        store.free(id);

        assert!(store.freed.contains(&id));
        assert!(!store.dirty_nodes.contains_key(&id));
        assert!(!store.warm_nodes.contains_key(&id));
        assert!(!store.clean_cache.lock().unwrap().contains(&id));
        assert!(matches!(
            store.get(id),
            Err(BinaryTrieError::NodeNotFound(_))
        ));
    }

    #[test]
    fn get_missing_returns_not_found() {
        let mut store = NodeStore::new_memory();
        assert!(matches!(
            store.get(42),
            Err(BinaryTrieError::NodeNotFound(42))
        ));
    }

    // --- serialize / deserialize roundtrip ---

    #[test]
    fn roundtrip_internal_both_none() {
        let node = Node::Internal(InternalNode::new(None, None));
        let bytes = serialize_node(&node);
        assert_eq!(bytes.len(), 17);
        assert_eq!(bytes[0], 0x01);

        let restored = deserialize_node(&bytes).unwrap();
        if let Node::Internal(internal) = restored {
            assert_eq!(internal.left, None);
            assert_eq!(internal.right, None);
            assert_eq!(internal.cached_hash, None);
        } else {
            panic!("expected Internal");
        }
    }

    #[test]
    fn roundtrip_internal_with_children() {
        let node = Node::Internal(InternalNode::new(Some(3), Some(7)));
        let bytes = serialize_node(&node);
        let restored = deserialize_node(&bytes).unwrap();
        if let Node::Internal(internal) = restored {
            assert_eq!(internal.left, Some(3));
            assert_eq!(internal.right, Some(7));
        } else {
            panic!("expected Internal");
        }
    }

    #[test]
    fn roundtrip_stem_empty() {
        let stem_node = make_stem(0x42);
        let node = Node::Stem(stem_node);
        let bytes = serialize_node(&node);
        // tag(1) + stem(31) + bitmap(32) + 0 values = 64 bytes
        assert_eq!(bytes.len(), 64);
        assert_eq!(bytes[0], 0x02);

        let restored = deserialize_node(&bytes).unwrap();
        if let Node::Stem(sn) = restored {
            assert_eq!(sn.stem[0], 0x42);
            assert!(sn.values.is_empty());
            assert!(sn.cached_hash.is_none());
            assert!(sn.cached_subtree.is_none());
        } else {
            panic!("expected Stem");
        }
    }

    #[test]
    fn roundtrip_stem_partial_values() {
        let mut stem_node = make_stem(0x01);
        stem_node.set_value(0, [0xAAu8; 32]);
        stem_node.set_value(127, [0xBBu8; 32]);
        stem_node.set_value(255, [0xCCu8; 32]);

        let node = Node::Stem(stem_node);
        let bytes = serialize_node(&node);
        // tag(1) + stem(31) + bitmap(32) + 3 values * 32 = 160 bytes
        assert_eq!(bytes.len(), 160);

        let restored = deserialize_node(&bytes).unwrap();
        if let Node::Stem(sn) = restored {
            assert_eq!(sn.get_value(0), Some([0xAAu8; 32]));
            assert_eq!(sn.get_value(127), Some([0xBBu8; 32]));
            assert_eq!(sn.get_value(255), Some([0xCCu8; 32]));
            // Other slots should be None.
            assert_eq!(sn.get_value(1), None);
            assert_eq!(sn.get_value(128), None);
        } else {
            panic!("expected Stem");
        }
    }

    #[test]
    fn roundtrip_stem_all_values() {
        let mut stem_node = make_stem(0xFF);
        for i in 0u8..=255 {
            stem_node.set_value(i, [i; 32]);
        }
        let node = Node::Stem(stem_node);
        let bytes = serialize_node(&node);
        // tag(1) + stem(31) + bitmap(32) + 256 values * 32 = 8256 bytes
        assert_eq!(bytes.len(), 8256);

        let restored = deserialize_node(&bytes).unwrap();
        if let Node::Stem(sn) = restored {
            for i in 0u8..=255 {
                assert_eq!(sn.get_value(i), Some([i; 32]), "mismatch at index {i}");
            }
        } else {
            panic!("expected Stem");
        }
    }

    #[test]
    fn deserialize_empty_bytes_returns_error() {
        assert!(matches!(
            deserialize_node(&[]),
            Err(BinaryTrieError::DeserializationError(_))
        ));
    }

    #[test]
    fn deserialize_unknown_tag_returns_error() {
        assert!(matches!(
            deserialize_node(&[0xAA]),
            Err(BinaryTrieError::DeserializationError(_))
        ));
    }

    #[test]
    fn deserialize_internal_too_short_returns_error() {
        // Only 10 bytes — needs 17.
        let bytes = [0x01u8; 10];
        assert!(matches!(
            deserialize_node(&bytes),
            Err(BinaryTrieError::DeserializationError(_))
        ));
    }

    #[test]
    fn deserialize_stem_too_short_returns_error() {
        // Only 30 bytes — needs at least 64.
        let bytes = [0x02u8; 30];
        assert!(matches!(
            deserialize_node(&bytes),
            Err(BinaryTrieError::DeserializationError(_))
        ));
    }

    #[test]
    fn node_key_encoding() {
        // Node keys are now raw u64 LE (8 bytes, no prefix).
        let key = node_key(1);
        assert_eq!(key.len(), 8);
        assert_eq!(&key[..], &1u64.to_le_bytes());

        let key_max = node_key(u64::MAX);
        assert_eq!(&key_max[..], &u64::MAX.to_le_bytes());
    }

    // --- memory-only NodeStore integrated operations ---

    #[test]
    fn store_tree_structure() {
        let mut store = NodeStore::new_memory();

        // Create two stem nodes.
        let mut s1 = make_stem(0x00);
        s1.set_value(0, [1u8; 32]);
        let mut s2 = make_stem(0xFF);
        s2.set_value(0, [2u8; 32]);

        let s1_id = store.create(Node::Stem(s1));
        let s2_id = store.create(Node::Stem(s2));

        // Create an internal node pointing to them.
        let root_id = store.create(Node::Internal(InternalNode::new(Some(s1_id), Some(s2_id))));

        // Verify the structure.
        let root = store.get(root_id).unwrap();
        if let Node::Internal(internal) = root {
            assert_eq!(internal.left, Some(s1_id));
            assert_eq!(internal.right, Some(s2_id));
        } else {
            panic!("expected Internal at root");
        }

        let left = store.get(s1_id).unwrap();
        assert!(matches!(left, Node::Stem(_)));

        let right = store.get(s2_id).unwrap();
        assert!(matches!(right, Node::Stem(_)));
    }

    // --- Interior mutability / &self read tests ---

    #[test]
    fn get_by_value_matches_get_mut_by_ref() {
        let mut store = NodeStore::new_memory();
        let mut stem = make_stem(0xAB);
        stem.set_value(0, [1u8; 32]);
        stem.set_value(42, [2u8; 32]);
        let id = store.create(Node::Stem(stem));

        // get(&self) returns by clone
        let by_value = store.get(id).unwrap();
        // get_mut(&mut self) returns by reference
        let by_ref = store.get_mut(id).unwrap();

        // Both should have the same data.
        if let (Node::Stem(v), Node::Stem(r)) = (&by_value, by_ref) {
            assert_eq!(v.stem, r.stem);
            assert_eq!(v.get_value(0), r.get_value(0));
            assert_eq!(v.get_value(42), r.get_value(42));
            assert_eq!(v.get_value(1), r.get_value(1)); // None
        } else {
            panic!("expected Stem nodes");
        }
    }

    #[test]
    fn concurrent_reads_via_shared_ref() {
        use std::sync::Arc;

        let mut store = NodeStore::new_memory();
        let mut stem = make_stem(0x01);
        stem.set_value(0, [42u8; 32]);
        let id = store.create(Node::Stem(stem));

        // Rotate to move node into warm (simulates post-flush state).
        store.rotate_generations();

        let store = Arc::new(store);

        // Spawn two threads that read concurrently via &self.
        let s1 = Arc::clone(&store);
        let s2 = Arc::clone(&store);

        let t1 = std::thread::spawn(move || {
            for _ in 0..1000 {
                let node = s1.get(id).unwrap();
                if let Node::Stem(s) = node {
                    assert_eq!(s.get_value(0), Some([42u8; 32]));
                }
            }
        });

        let t2 = std::thread::spawn(move || {
            for _ in 0..1000 {
                let node = s2.get(id).unwrap();
                if let Node::Stem(s) = node {
                    assert_eq!(s.get_value(0), Some([42u8; 32]));
                }
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
    }
}
