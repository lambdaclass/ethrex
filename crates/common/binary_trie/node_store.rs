use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::BinaryTrieError;
#[cfg(any(test, feature = "rocksdb"))]
use crate::node::{InternalNode, STEM_VALUES, StemNode};
use crate::node::{Node, NodeId};

// Key prefixes for RocksDB storage
#[cfg(any(test, feature = "rocksdb"))]
const NODE_PREFIX: u8 = 0x01;
#[cfg(feature = "rocksdb")]
const META_PREFIX: u8 = 0xFF;
#[cfg(feature = "rocksdb")]
const META_ROOT: &[u8] = &[META_PREFIX, b'R'];
#[cfg(feature = "rocksdb")]
const META_NEXT_ID: &[u8] = &[META_PREFIX, b'N'];

/// Returns a 9-byte key for a node: `NODE_PREFIX || id as little-endian u64`.
#[cfg(any(test, feature = "rocksdb"))]
fn node_key(id: NodeId) -> [u8; 9] {
    let mut key = [0u8; 9];
    key[0] = NODE_PREFIX;
    key[1..].copy_from_slice(&id.to_le_bytes());
    key
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
            for (i, val) in stem.values.iter().enumerate() {
                if val.is_some() {
                    bitmap[i / 8] |= 1 << (i % 8);
                }
            }

            // Count present values to pre-allocate.
            let present_count = stem.values.iter().filter(|v| v.is_some()).count();
            let mut buf = Vec::with_capacity(1 + 31 + 32 + present_count * 32);
            buf.push(0x02);
            buf.extend_from_slice(&stem.stem);
            buf.extend_from_slice(&bitmap);
            for val in stem.values.iter() {
                if let Some(v) = val {
                    buf.extend_from_slice(v);
                }
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

            // Reconstruct the values array from the bitmap and packed data.
            let mut values = Box::new([None::<[u8; 32]>; STEM_VALUES]);
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
                    values[i] = Some(v);
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
    cache: FxHashMap<NodeId, Node>,
    dirty: FxHashSet<NodeId>,
    freed: FxHashSet<NodeId>,
    next_id: NodeId,
    #[cfg(feature = "rocksdb")]
    db: Option<std::sync::Arc<rocksdb::DB>>,
}

impl NodeStore {
    /// Create a pure in-memory NodeStore (no persistence).
    pub fn new_memory() -> Self {
        Self {
            cache: FxHashMap::default(),
            dirty: FxHashSet::default(),
            freed: FxHashSet::default(),
            next_id: 1,
            #[cfg(feature = "rocksdb")]
            db: None,
        }
    }

    /// Open a persistent NodeStore backed by RocksDB.
    ///
    /// Reads `next_id` from the `META_NEXT_ID` key. If absent, starts at 1.
    #[cfg(feature = "rocksdb")]
    pub fn open(db: std::sync::Arc<rocksdb::DB>) -> Result<Self, BinaryTrieError> {
        let next_id = match db
            .get(META_NEXT_ID)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?
        {
            Some(bytes) if bytes.len() >= 8 => u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            _ => 1,
        };

        Ok(Self {
            cache: FxHashMap::default(),
            dirty: FxHashSet::default(),
            freed: FxHashSet::default(),
            next_id,
            db: Some(db),
        })
    }

    /// Load the persisted root NodeId from the database, if any.
    #[cfg(feature = "rocksdb")]
    pub fn load_root(&self) -> Option<NodeId> {
        let db = self.db.as_ref()?;
        let bytes = db.get(META_ROOT).ok()??;
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

    /// Create a new node, assign it an ID, insert into cache, and mark dirty.
    pub fn create(&mut self, node: Node) -> NodeId {
        let id = self.alloc_id();
        self.cache.insert(id, node);
        self.dirty.insert(id);
        id
    }

    /// Get a shared reference to a node by ID.
    ///
    /// On a cache miss, attempts to load from RocksDB (if a database is
    /// configured). Returns `NodeNotFound` if the node is absent from both.
    pub fn get(&mut self, id: NodeId) -> Result<&Node, BinaryTrieError> {
        // Load from DB if not in cache.
        if !self.cache.contains_key(&id) {
            #[cfg(feature = "rocksdb")]
            {
                let node = self.load_from_db(id)?;
                self.cache.insert(id, node);
            }
            #[cfg(not(feature = "rocksdb"))]
            {
                return Err(BinaryTrieError::NodeNotFound(id));
            }
        }
        Ok(self.cache.get(&id).unwrap())
    }

    /// Remove a node from the cache and return it.
    ///
    /// The caller must return the node via `put` or `free` on all code paths.
    /// On a cache miss, attempts to load from RocksDB first.
    pub fn take(&mut self, id: NodeId) -> Result<Node, BinaryTrieError> {
        if let Some(node) = self.cache.remove(&id) {
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
        self.cache.insert(id, node);
        self.dirty.insert(id);
        // If it was in the freed set (shouldn't happen in normal use), un-free it.
        self.freed.remove(&id);
    }

    /// Put a node back without marking it dirty.
    ///
    /// Use this when only the in-memory cache needs to be updated — for example,
    /// after computing and storing a `cached_hash` on a node that was otherwise
    /// not mutated.
    pub fn put_clean(&mut self, id: NodeId, node: Node) {
        self.cache.insert(id, node);
    }

    /// Schedule a node for deletion on the next `flush`.
    pub fn free(&mut self, id: NodeId) {
        self.cache.remove(&id);
        self.dirty.remove(&id);
        self.freed.insert(id);
    }

    /// Flush all dirty and freed nodes to RocksDB, writing the root and
    /// next_id metadata atomically via a `WriteBatch`.
    ///
    /// After flushing, the dirty and freed sets are cleared.
    #[cfg(feature = "rocksdb")]
    pub fn flush(&mut self, root: Option<NodeId>) -> Result<(), BinaryTrieError> {
        let db = match self.db.as_ref() {
            Some(db) => db.clone(),
            None => return Ok(()), // In-memory only — nothing to flush.
        };

        let mut batch = rocksdb::WriteBatch::default();
        self.write_to_batch(&mut batch, root);

        db.write(batch)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;

        self.dirty.clear();
        self.freed.clear();
        Ok(())
    }

    /// Write dirty and freed nodes plus metadata into a caller-supplied
    /// `WriteBatch`. Used by `BinaryTrieState::flush` to build a single atomic
    /// batch that also contains code_store and storage_keys entries.
    ///
    /// Clears the dirty and freed sets after writing.
    #[cfg(feature = "rocksdb")]
    pub fn flush_to_batch(&mut self, batch: &mut rocksdb::WriteBatch, root: Option<NodeId>) {
        self.write_to_batch(batch, root);
        self.dirty.clear();
        self.freed.clear();
    }

    /// Internal helper: writes dirty nodes, freed deletions, and metadata into `batch`.
    /// Does NOT clear dirty/freed — callers do that after the batch is committed.
    #[cfg(feature = "rocksdb")]
    fn write_to_batch(&self, batch: &mut rocksdb::WriteBatch, root: Option<NodeId>) {
        // Write all dirty nodes.
        for &id in &self.dirty {
            if let Some(node) = self.cache.get(&id) {
                let key = node_key(id);
                let bytes = serialize_node(node);
                batch.put(key, bytes);
            } else {
                debug_assert!(
                    false,
                    "dirty node {id} not in cache at flush time — likely take/put imbalance"
                );
            }
        }

        // Delete all freed nodes.
        for &id in &self.freed {
            batch.delete(node_key(id));
        }

        // Write root metadata.
        let root_bytes = root.unwrap_or(0).to_le_bytes();
        batch.put(META_ROOT, root_bytes);

        // Write next_id metadata.
        batch.put(META_NEXT_ID, self.next_id.to_le_bytes());
    }

    /// Return the next ID that will be allocated (for testing/debugging).
    pub fn next_id(&self) -> NodeId {
        self.next_id
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    #[cfg(feature = "rocksdb")]
    fn load_from_db(&self, id: NodeId) -> Result<Node, BinaryTrieError> {
        let db = self.db.as_ref().ok_or(BinaryTrieError::NodeNotFound(id))?;
        let key = node_key(id);
        match db
            .get(&key)
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
        // Drain dirty set.
        store.dirty.clear();

        store.put_clean(id, Node::Internal(InternalNode::new(Some(2), Some(3))));
        assert!(!store.dirty.contains(&id));
        assert!(store.cache.contains_key(&id));
    }

    #[test]
    fn free_schedules_deletion() {
        let mut store = NodeStore::new_memory();
        let id = store.create(Node::Internal(InternalNode::new(None, None)));
        store.free(id);

        assert!(store.freed.contains(&id));
        assert!(!store.cache.contains_key(&id));
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
            assert!(sn.values.iter().all(|v| v.is_none()));
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
    fn node_key_has_correct_prefix_and_encoding() {
        let key = node_key(1);
        assert_eq!(key[0], NODE_PREFIX);
        assert_eq!(&key[1..], &1u64.to_le_bytes());

        let key_max = node_key(u64::MAX);
        assert_eq!(key_max[0], NODE_PREFIX);
        assert_eq!(&key_max[1..], &u64::MAX.to_le_bytes());
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
}
