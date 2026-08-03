//! Incremental insertion-based binary radix trie, backed by a
//! [`BinaryTrieDB`].
//!
//! The trie retains its node structure across insertions, splitting
//! nodes on descent, and hashes that structure on [`BinaryTrie::root`].
//! Canonical-form invariant: a branch's prefix is exactly the bits its
//! two subtrees share beyond the parent split, so the structure — and
//! therefore the root — depends only on the key/value set, never on
//! insertion order.
//!
//! Subtrees need not be in memory. A [`NodeRef`] is either a loaded
//! node or the hash of one that lives in the database under its bit
//! path; reads and writes resolve stored references as they descend,
//! and nothing else is touched. Because a stored reference already
//! *is* its subtree's hash, [`BinaryTrie::root`] never reads anything:
//! an unmodified opened trie reports its root without a single
//! database access.
//!
//! A loaded node carries two pieces of state about itself, correlated
//! when it changes but divergent afterwards. Its *cached hash* is
//! filled the first time anyone needs it, so [`BinaryTrie::root`]
//! hashes each node at most once. Its *dirty* flag says the database's
//! copy is stale, so [`BinaryTrie::commit`] writes only what changed.
//! They cannot be one field: `root()` fills every cached hash, which
//! would erase the record of what still needs writing.
//!
//! Deliberate simplifications, deferred to later tasks of the
//! persistent-state plan: no deletion. Recursion depth is bounded by
//! the key length in bits, so max-length keys could theoretically
//! overflow the stack; embedding keys are at most 66 bytes, and going
//! iterative is the fallback if that ever matters.

use std::sync::OnceLock;

use ethereum_types::H256;

use crate::error::BinaryTrieError;

use super::MAX_KEY_LENGTH;
use super::bits::bytes_to_bits;
use super::db::{BinaryTrieDB, InMemoryBinaryTrieDB};
use super::node::{
    EMPTY_TRIE_ROOT, StoredNode, blake3_hash, branch_hash, decode, encode_branch, encode_leaf,
    leaf_hash,
};
use super::path::BitPath;

enum Node {
    Leaf {
        key: Vec<u8>,
        value: [u8; 32],
    },
    Branch {
        /// Bits (0/1 per element) shared by every key below, relative
        /// to the parent's split point.
        prefix: Vec<u8>,
        left: NodeRef,
        right: NodeRef,
    },
}

/// A child: either the node itself, or the hash of a subtree the
/// database holds, to be loaded if and when the descent reaches it.
enum NodeRef {
    Loaded {
        node: Box<Node>,
        /// This node's hash, empty until something asks for it.
        hash: OnceLock<H256>,
        /// The database's copy of this node is stale, or absent.
        dirty: bool,
    },
    Stored(H256),
}

impl NodeRef {
    /// A node that was just built or just changed shape: nothing knows
    /// its hash, and the database does not have it.
    fn loaded(node: Node) -> Self {
        Self::Loaded {
            node: Box::new(node),
            hash: OnceLock::new(),
            dirty: true,
        }
    }

    /// A node read back from the database under `hash`: its hash is
    /// exactly the reference that pointed at it, and the copy on disk
    /// is the one in hand.
    fn resolved(node: Node, hash: H256) -> Self {
        Self::Loaded {
            node: Box::new(node),
            hash: OnceLock::from(hash),
            dirty: false,
        }
    }

    /// Record that this subtree changed: the cached hash is no longer
    /// this node's hash, and the database holds an older version.
    fn invalidate(&mut self) {
        if let Self::Loaded { hash, dirty, .. } = self {
            hash.take();
            *dirty = true;
        }
    }
}

/// Compressed binary radix trie over prefix-free byte keys and
/// 32-byte values, committing to its contents with a BLAKE3 root.
pub struct BinaryTrie {
    db: Box<dyn BinaryTrieDB>,
    root: Option<NodeRef>,
}

impl BinaryTrie {
    /// An empty trie writing to `db`.
    pub fn new(db: Box<dyn BinaryTrieDB>) -> Self {
        Self { db, root: None }
    }

    /// The trie `db` holds under `root`, with nothing loaded yet.
    ///
    /// Nodes are read lazily, so a wrong or absent root only surfaces
    /// when a traversal reaches a node that is not there.
    pub fn open(db: Box<dyn BinaryTrieDB>, root: H256) -> Self {
        Self {
            db,
            root: (root != EMPTY_TRIE_ROOT).then_some(NodeRef::Stored(root)),
        }
    }

    /// An empty trie over a fresh in-memory backend.
    pub fn new_temp() -> Self {
        Self::new(Box::new(InMemoryBinaryTrieDB::new_empty()))
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
    /// - [`BinaryTrieError::Backend`] or
    ///   [`BinaryTrieError::MalformedNode`] if a node on the path could
    ///   not be loaded.
    pub fn insert(&mut self, key: Vec<u8>, value: [u8; 32]) -> Result<(), BinaryTrieError> {
        if key.is_empty() {
            return Err(BinaryTrieError::EmptyKey);
        }
        if key.len() > MAX_KEY_LENGTH {
            return Err(BinaryTrieError::KeyTooLong);
        }
        let bits = bytes_to_bits(&key);
        let Self { db, root } = self;
        match root {
            None => {
                *root = Some(NodeRef::loaded(Node::Leaf { key, value }));
                Ok(())
            }
            Some(node_ref) => Self::insert_at(db.as_ref(), node_ref, &bits, 0, key, value),
        }
    }

    /// Load the node `node_ref` points at, caching it in place so the
    /// next traversal finds it loaded.
    ///
    /// `path` must be the bit path of `node_ref` itself: that is the
    /// key it was written under.
    ///
    /// The node arrives clean, with its hash already known: the
    /// reference being replaced *was* that hash, and the bytes it was
    /// decoded from are the ones on disk. So a read costs no hashing
    /// and leaves nothing for the next commit to write.
    fn resolve<'a>(
        db: &dyn BinaryTrieDB,
        node_ref: &'a mut NodeRef,
        path: &BitPath,
    ) -> Result<&'a mut Node, BinaryTrieError> {
        if let NodeRef::Stored(hash) = *node_ref {
            let encoded = db.get(path)?.ok_or_else(|| {
                BinaryTrieError::Backend(format!("no node stored at the path of {hash:#x}"))
            })?;
            let node = match decode(&encoded)? {
                StoredNode::Leaf { key, value } => Node::Leaf { key, value },
                StoredNode::Branch {
                    prefix,
                    left,
                    right,
                } => Node::Branch {
                    prefix,
                    left: NodeRef::Stored(left),
                    right: NodeRef::Stored(right),
                },
            };
            *node_ref = NodeRef::resolved(node, hash);
        }
        match node_ref {
            NodeRef::Loaded { node, .. } => Ok(node),
            NodeRef::Stored(_) => unreachable!("just replaced with a loaded node"),
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

    /// Insert into the subtree rooted at `node_ref`, whose path from
    /// the trie root has consumed the first `depth` bits of every key
    /// below it — and is therefore `bits[..depth]`, since the new key
    /// followed that path to get here.
    ///
    /// Every failure is detected before anything is mutated, so an
    /// error leaves the subtree — and therefore the trie — untouched.
    ///
    /// Success means something at or below this node changed, so this
    /// node's cached hash and stored copy are both out of date. The
    /// `&mut` descent visits exactly the nodes from the root down to
    /// the change, so invalidating one frame on the way out
    /// invalidates every ancestor of the change and nothing else.
    fn insert_at(
        db: &dyn BinaryTrieDB,
        node_ref: &mut NodeRef,
        bits: &[u8],
        depth: usize,
        key: Vec<u8>,
        value: [u8; 32],
    ) -> Result<(), BinaryTrieError> {
        let result = Self::insert_below(db, node_ref, bits, depth, key, value);
        if result.is_ok() {
            node_ref.invalidate();
        }
        result
    }

    /// The insertion itself; see [`BinaryTrie::insert_at`], which wraps
    /// it to invalidate the node on the way back out.
    fn insert_below(
        db: &dyn BinaryTrieDB,
        node_ref: &mut NodeRef,
        bits: &[u8],
        depth: usize,
        key: Vec<u8>,
        value: [u8; 32],
    ) -> Result<(), BinaryTrieError> {
        let node = Self::resolve(db, node_ref, &BitPath::from_bits(&bits[..depth]))?;
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
                    return Self::insert_at(db, child, bits, split + 1, key, value);
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
        // Both children are new to their positions: the leaf did not
        // exist, and the displaced subtree's own node now sits one
        // level deeper, so neither is where the database left it.
        let new_leaf = NodeRef::loaded(Node::Leaf { key, value });
        let displaced = NodeRef::loaded(displaced);
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
    ///
    /// Takes `&mut self` because a read loads the nodes on its path and
    /// keeps them: the next read down the same path costs nothing.
    ///
    /// # Errors
    ///
    /// [`BinaryTrieError::Backend`] or [`BinaryTrieError::MalformedNode`]
    /// if a node on the path could not be loaded.
    pub fn get(&mut self, key: &[u8]) -> Result<Option<[u8; 32]>, BinaryTrieError> {
        let bits = bytes_to_bits(key);
        let Self { db, root } = self;
        match root {
            None => Ok(None),
            Some(node_ref) => Self::get_at(db.as_ref(), node_ref, &bits, 0, key),
        }
    }

    fn get_at(
        db: &dyn BinaryTrieDB,
        node_ref: &mut NodeRef,
        bits: &[u8],
        depth: usize,
        key: &[u8],
    ) -> Result<Option<[u8; 32]>, BinaryTrieError> {
        match Self::resolve(db, node_ref, &BitPath::from_bits(&bits[..depth]))? {
            Node::Leaf {
                key: leaf_key,
                value,
            } => Ok((leaf_key.as_slice() == key).then_some(*value)),
            Node::Branch {
                prefix,
                left,
                right,
            } => {
                let split = depth + prefix.len();
                if split >= bits.len() || bits[depth..split] != prefix[..] {
                    return Ok(None);
                }
                let child = if bits[split] == 0 { left } else { right };
                Self::get_at(db, child, bits, split + 1, key)
            }
        }
    }

    /// Root hash: [`EMPTY_TRIE_ROOT`] for the empty trie, otherwise
    /// the recursive tagged BLAKE3 commitment of the node structure.
    ///
    /// Reads nothing: a stored reference is already the hash of what it
    /// points at, so only loaded nodes are hashed — and each of those
    /// at most once, since the answer is cached in the node. Insertion
    /// clears the cache on every node it descends through, so a cached
    /// hash is only ever returned for a subtree that has not changed
    /// since it was computed.
    pub fn root(&self) -> H256 {
        match &self.root {
            None => EMPTY_TRIE_ROOT,
            Some(node_ref) => Self::merkleize(node_ref),
        }
    }

    fn merkleize(node_ref: &NodeRef) -> H256 {
        match node_ref {
            NodeRef::Stored(hash) => *hash,
            NodeRef::Loaded { node, hash, .. } => *hash.get_or_init(|| Self::hash_node(node)),
        }
    }

    /// Hash of a loaded node from its children — the computation the
    /// cache memoizes, kept out of [`BinaryTrie::merkleize`] so that
    /// filling one node's cache never re-enters that same cache.
    fn hash_node(node: &Node) -> H256 {
        match node {
            Node::Leaf { key, value } => leaf_hash(key, value),
            Node::Branch {
                prefix,
                left,
                right,
            } => branch_hash(prefix, Self::merkleize(left), Self::merkleize(right)),
        }
    }

    /// Write the changed nodes to the database under their bit paths
    /// and return the root hash.
    ///
    /// Only the changed ones: a node whose stored copy is still current
    /// is skipped, along with everything below it. That is safe because
    /// insertion dirties every node from the root down to what it
    /// changed, so a clean node cannot have a dirty descendant — the
    /// invariant the skip rests on.
    ///
    /// Stored references are skipped too — their subtrees are already
    /// on disk, and a split never moves them, since the bits an
    /// ancestor stops consuming are exactly the bits its new child
    /// starts consuming.
    ///
    /// Committing an unchanged trie writes nothing at all, not even an
    /// empty batch.
    ///
    /// # Errors
    ///
    /// [`BinaryTrieError::Backend`] if the write fails. The dirty flags
    /// survive a failed write, so the nodes are offered again next
    /// time.
    pub fn commit(&mut self) -> Result<H256, BinaryTrieError> {
        let mut entries = Vec::new();
        let root = match &self.root {
            None => EMPTY_TRIE_ROOT,
            Some(node_ref) => Self::collect(node_ref, BitPath::new(), &mut entries),
        };
        if entries.is_empty() {
            return Ok(root);
        }
        self.db.put_batch(entries)?;
        if let Some(node_ref) = &mut self.root {
            Self::mark_clean(node_ref);
        }
        Ok(root)
    }

    /// Hash of the subtree at `path`, pushing every dirty node in it
    /// onto `entries` as (path, encoded bytes).
    fn collect(node_ref: &NodeRef, path: BitPath, entries: &mut Vec<(BitPath, Vec<u8>)>) -> H256 {
        let (node, hash) = match node_ref {
            NodeRef::Stored(hash) => return *hash,
            NodeRef::Loaded { node, hash, dirty } => {
                if !*dirty {
                    // The database already has this node, and — by the
                    // invariant [`BinaryTrie::commit`] documents —
                    // everything below it as well.
                    return *hash.get_or_init(|| Self::hash_node(node));
                }
                (node, hash)
            }
        };
        // A node's stored bytes are its hashing preimage, so the
        // encoding written and the hash returned cannot disagree.
        let encoded = match node.as_ref() {
            Node::Leaf { key, value } => encode_leaf(key, value),
            Node::Branch {
                prefix,
                left,
                right,
            } => {
                let left_hash = Self::collect(left, path.child(prefix, 0), entries);
                let right_hash = Self::collect(right, path.child(prefix, 1), entries);
                encode_branch(prefix, left_hash, right_hash)
            }
        };
        let computed = blake3_hash(&encoded);
        entries.push((path, encoded));
        // A dirty node may still hold a cached hash — `root()` fills
        // caches without cleaning anything — and it agrees with what
        // was just computed, since both describe the current subtree.
        *hash.get_or_init(|| computed)
    }

    /// Mark the subtree at `node_ref` as matching the database, having
    /// just written it. Stops at clean nodes: by the invariant
    /// [`BinaryTrie::commit`] relies on, there is nothing dirty below
    /// one.
    fn mark_clean(node_ref: &mut NodeRef) {
        let NodeRef::Loaded { node, dirty, .. } = node_ref else {
            return;
        };
        if !*dirty {
            return;
        }
        *dirty = false;
        if let Node::Branch { left, right, .. } = node.as_mut() {
            Self::mark_clean(left);
            Self::mark_clean(right);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::BinaryTrieError;
    use crate::trie::db::{BinaryTrieDB, InMemoryBinaryTrieDB, NodeMap};
    use crate::trie::node::EMPTY_TRIE_ROOT;
    use crate::trie::path::BitPath;
    use hex_literal::hex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Nodes read and nodes written, counted per node rather than per
    /// call: what matters is how much of the tree was touched, not how
    /// many batches it arrived in.
    #[derive(Clone, Default)]
    struct Counts {
        reads: Arc<AtomicUsize>,
        writes: Arc<AtomicUsize>,
    }

    impl Counts {
        fn reads(&self) -> usize {
            self.reads.load(Ordering::Relaxed)
        }

        fn writes(&self) -> usize {
            self.writes.load(Ordering::Relaxed)
        }
    }

    /// A backend that counts reads and writes, so a test can assert
    /// what a traversal actually loaded or stored rather than what it
    /// could have.
    struct CountingDB {
        inner: InMemoryBinaryTrieDB,
        counts: Counts,
    }

    impl CountingDB {
        fn over(map: NodeMap) -> (Self, Counts) {
            let counts = Counts::default();
            (
                Self {
                    inner: InMemoryBinaryTrieDB::new(map),
                    counts: counts.clone(),
                },
                counts,
            )
        }
    }

    impl BinaryTrieDB for CountingDB {
        fn get(&self, path: &BitPath) -> Result<Option<Vec<u8>>, BinaryTrieError> {
            self.counts.reads.fetch_add(1, Ordering::Relaxed);
            self.inner.get(path)
        }

        fn put_batch(&self, entries: Vec<(BitPath, Vec<u8>)>) -> Result<(), BinaryTrieError> {
            self.counts
                .writes
                .fetch_add(entries.len(), Ordering::Relaxed);
            self.inner.put_batch(entries)
        }
    }

    /// Five keys in the 34- and 66-byte shapes the embedding produces,
    /// pairwise prefix-free.
    fn sample_entries() -> Vec<(Vec<u8>, [u8; 32])> {
        let mut long = vec![0xab; 66];
        long[0] = 0x40;
        [vec![0x00; 34], vec![0x01; 34], vec![0x80; 34], long]
            .into_iter()
            .chain(std::iter::once(vec![0xff; 34]))
            .enumerate()
            .map(|(i, key)| (key, [i as u8; 32]))
            .collect()
    }

    /// Sixteen keys differing only in the low nibble of their first
    /// byte: 31 nodes in all, at most 5 of them on any root-to-leaf
    /// path.
    fn wide_entries() -> Vec<(Vec<u8>, [u8; 32])> {
        (0u8..16)
            .map(|i| {
                let mut key = vec![0x00; 34];
                key[0] = i;
                (key, [i; 32])
            })
            .collect()
    }

    /// Fill a fresh in-memory backend, commit, and hand back the node
    /// map plus the committed root.
    fn commit_entries(entries: &[(Vec<u8>, [u8; 32])]) -> (NodeMap, H256) {
        let db = InMemoryBinaryTrieDB::new_empty();
        let map = db.inner();
        let mut trie = BinaryTrie::new(Box::new(db));
        for (key, value) in entries {
            trie.insert(key.clone(), *value).unwrap();
        }
        let root = trie.commit().unwrap();
        assert_eq!(root, trie.root(), "commit and root must agree");
        (map, root)
    }

    #[test]
    fn round_trips_through_the_database() {
        let entries = sample_entries();
        let (map, root) = commit_entries(&entries);

        // A fresh handle on the same nodes: nothing carries over from
        // the trie that wrote them.
        let mut reopened = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map)), root);
        assert_eq!(reopened.root(), root);
        for (key, value) in &entries {
            assert_eq!(reopened.get(key).unwrap(), Some(*value));
        }
        assert_eq!(reopened.get(&[0x7f; 34]).unwrap(), None);
        assert_eq!(reopened.root(), root, "reading must not change the root");
    }

    #[test]
    fn opened_trie_computes_its_root_without_reading() {
        let (map, root) = commit_entries(&sample_entries());
        let (db, counts) = CountingDB::over(map);

        let trie = BinaryTrie::open(Box::new(db), root);
        assert_eq!(trie.root(), root);
        assert_eq!(counts.reads(), 0, "a stored reference already is its hash");
    }

    #[test]
    fn loads_lazily_on_descent() {
        let entries = wide_entries();
        let (map, root) = commit_entries(&entries);
        let (db, counts) = CountingDB::over(map);

        let mut trie = BinaryTrie::open(Box::new(db), root);
        let (key, value) = &entries[9];
        assert_eq!(trie.get(key).unwrap(), Some(*value));

        let loaded = counts.reads();
        assert!(
            loaded <= 5,
            "descent should read the path (<= 5 nodes), read {loaded}"
        );
        assert!(
            loaded < 2 * entries.len() - 1,
            "descent read {loaded} of the tree's {} nodes",
            2 * entries.len() - 1
        );
    }

    #[test]
    fn insert_into_an_opened_trie() {
        let entries = sample_entries();
        let (map, root) = commit_entries(&entries);

        let mut trie = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map.clone())), root);
        trie.insert(vec![0x7f; 34], [0xee; 32]).unwrap();
        let new_root = trie.commit().unwrap();
        assert_ne!(new_root, root, "a new key must move the root");

        let mut reopened = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map)), new_root);
        for (key, value) in &entries {
            assert_eq!(reopened.get(key).unwrap(), Some(*value));
        }
        assert_eq!(reopened.get(&[0x7f; 34]).unwrap(), Some([0xee; 32]));
        assert_eq!(reopened.root(), new_root);
    }

    #[test]
    fn splitting_a_branch_keeps_its_stored_subtrees_addressable() {
        let entries = wide_entries();
        let (map, root) = commit_entries(&entries);

        let mut trie = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map.clone())), root);
        // Diverges on the very first bit, inside the root branch's
        // prefix, so the root splits while both its subtrees are still
        // nothing but hashes. Their absolute paths must come out
        // unchanged, or the nodes below become unreachable.
        let mut newcomer = vec![0x00; 34];
        newcomer[0] = 0x80;
        trie.insert(newcomer.clone(), [0xcd; 32]).unwrap();
        let new_root = trie.commit().unwrap();

        let mut reopened = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map)), new_root);
        for (key, value) in &entries {
            assert_eq!(
                reopened.get(key).unwrap(),
                Some(*value),
                "key {key:02x?} after the split"
            );
        }
        assert_eq!(reopened.get(&newcomer).unwrap(), Some([0xcd; 32]));

        // And the result is the canonical tree for that key set, not
        // merely a self-consistent one.
        let mut from_scratch = BinaryTrie::new_temp();
        for (key, value) in entries.iter().chain(&[(newcomer, [0xcd; 32])]) {
            from_scratch.insert(key.clone(), *value).unwrap();
        }
        assert_eq!(new_root, from_scratch.root());
    }

    /// Root of a from-scratch trie over `entries` — the canonical
    /// answer any incrementally-updated trie must agree with.
    fn root_from_scratch(entries: &[(Vec<u8>, [u8; 32])]) -> H256 {
        let mut trie = BinaryTrie::new_temp();
        for (key, value) in entries {
            trie.insert(key.clone(), *value).unwrap();
        }
        trie.root()
    }

    /// A key that shares all eight leading bits with `wide_entries`'
    /// fifth key and diverges only in the second byte, so inserting it
    /// splits the leaf at the bottom of a five-node path rather than
    /// anything near the root.
    fn deep_split_key() -> Vec<u8> {
        let mut key = vec![0x00; 34];
        key[0] = 0x05;
        key[1] = 0x01;
        key
    }

    /// Longest root-to-leaf path in the `wide_entries` tree: a
    /// four-branch spine (splitting bits 4, 5, 6 and 7) plus the leaf.
    const WIDE_PATH_NODES: usize = 5;

    #[test]
    fn reopened_trie_commits_nothing_when_unchanged() {
        let entries = wide_entries();
        let (map, root) = commit_entries(&entries);
        let (db, counts) = CountingDB::over(map);

        let mut trie = BinaryTrie::open(Box::new(db), root);
        for (key, value) in entries.iter().take(4) {
            assert_eq!(trie.get(key).unwrap(), Some(*value));
        }
        assert!(counts.reads() > 0, "the reads should have loaded nodes");

        assert_eq!(trie.commit().unwrap(), root);
        assert_eq!(
            counts.writes(),
            0,
            "reading loads nodes but changes none of them, so there is \
             nothing the database does not already hold"
        );
    }

    #[test]
    fn resolved_nodes_are_clean() {
        let entries = wide_entries();
        let (map, root) = commit_entries(&entries);
        let (db, counts) = CountingDB::over(map);

        let mut trie = BinaryTrie::open(Box::new(db), root);
        let (key, value) = &entries[9];
        assert_eq!(trie.get(key).unwrap(), Some(*value));

        assert_eq!(
            trie.commit().unwrap(),
            root,
            "reading must not move the root"
        );
        assert_eq!(
            counts.writes(),
            0,
            "a node resolved from the database matches the database"
        );
    }

    #[test]
    fn insert_writes_only_the_touched_path() {
        let entries = wide_entries();
        let (map, root) = commit_entries(&entries);
        let (db, counts) = CountingDB::over(map);

        let newcomer = deep_split_key();
        let mut trie = BinaryTrie::open(Box::new(db), root);
        trie.insert(newcomer.clone(), [0xcd; 32]).unwrap();
        let new_root = trie.commit().unwrap();

        let mut expected = entries;
        expected.push((newcomer, [0xcd; 32]));
        assert_eq!(
            new_root,
            root_from_scratch(&expected),
            "the incremental root must be the canonical one"
        );

        // The split replaces the leaf it lands on with a branch and two
        // leaves — one node more than the path held — and rewrites the
        // ancestors above it because their child hashes moved.
        let bound = WIDE_PATH_NODES + 2;
        let written = counts.writes();
        assert!(
            written <= bound,
            "an insert should write the root-to-leaf path ({WIDE_PATH_NODES}) plus \
             the split's new branch and second leaf (+2) = {bound} nodes, wrote {written}"
        );
        let node_count = 2 * expected.len() - 1;
        assert!(
            written < node_count,
            "wrote {written} of the tree's {node_count} nodes"
        );
    }

    #[test]
    fn root_is_stable_across_calls_and_matches_a_fresh_trie() {
        let entries = wide_entries();
        let mut trie = BinaryTrie::new_temp();
        for (key, value) in &entries {
            trie.insert(key.clone(), *value).unwrap();
        }

        let first = trie.root();
        assert_eq!(first, trie.root(), "root() must not depend on call count");
        assert_eq!(first, root_from_scratch(&entries));

        // And again after a further insert, so a cache filled by the
        // first call cannot survive into the second answer.
        trie.insert(deep_split_key(), [0xcd; 32]).unwrap();
        let second = trie.root();
        assert_eq!(second, trie.root());
        let mut expected = entries;
        expected.push((deep_split_key(), [0xcd; 32]));
        assert_eq!(second, root_from_scratch(&expected));
    }

    #[test]
    fn mutation_invalidates_ancestor_hashes() {
        let entries = wide_entries();
        let mut trie = BinaryTrie::new_temp();
        for (key, value) in &entries {
            trie.insert(key.clone(), *value).unwrap();
        }
        // Fills every cached hash in the tree, root included.
        let before = trie.root();

        // Splits a leaf four branches down: every node between it and
        // the root now commits to something that changed.
        let newcomer = deep_split_key();
        trie.insert(newcomer.clone(), [0xcd; 32]).unwrap();
        let after = trie.root();

        assert_ne!(after, before, "a new key must move the root");
        let mut expected = entries;
        expected.push((newcomer, [0xcd; 32]));
        assert_eq!(
            after,
            root_from_scratch(&expected),
            "a stale ancestor hash is a wrong root"
        );
    }

    #[test]
    fn decode_failure_surfaces() {
        let db = InMemoryBinaryTrieDB::new_empty();
        let map = db.inner();
        db.put_batch(vec![(BitPath::new(), vec![0x7f, 0x01, 0x02])])
            .unwrap();

        let mut trie = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map)), H256([0x11; 32]));
        assert!(matches!(
            trie.get(&[0xab; 34]),
            Err(BinaryTrieError::MalformedNode(_))
        ));
    }

    #[test]
    fn open_at_the_empty_root_is_an_empty_trie() {
        let mut trie =
            BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new_empty()), EMPTY_TRIE_ROOT);
        assert_eq!(trie.root(), EMPTY_TRIE_ROOT);
        assert_eq!(trie.get(&[0xab; 34]).unwrap(), None);
        assert_eq!(trie.commit().unwrap(), EMPTY_TRIE_ROOT);
    }

    #[test]
    fn empty_trie_root_is_sentinel() {
        assert_eq!(BinaryTrie::new_temp().root(), EMPTY_TRIE_ROOT);
    }

    #[test]
    fn single_leaf_matches_spec_vector() {
        let mut trie = BinaryTrie::new_temp();
        trie.insert(vec![0u8; 34], [0x01; 32]).unwrap();
        assert_eq!(
            trie.root().0,
            hex!("4b60a28dce9f3529d103a26e00fadb98514cbd16ce03b7df752426addef9bbc7")
        );
    }

    #[test]
    fn get_returns_inserted_value_and_none_for_absent() {
        let mut trie = BinaryTrie::new_temp();
        trie.insert(vec![0xab; 34], [7; 32]).unwrap();
        assert_eq!(trie.get(&[0xab; 34]).unwrap(), Some([7; 32]));
        assert_eq!(trie.get(&[0xac; 34]).unwrap(), None);
    }

    #[test]
    fn overwrite_replaces_value() {
        let mut trie = BinaryTrie::new_temp();
        trie.insert(vec![0x42; 34], [1; 32]).unwrap();
        trie.insert(vec![0x42; 34], [2; 32]).unwrap();
        assert_eq!(trie.get(&[0x42; 34]).unwrap(), Some([2; 32]));
    }

    #[test]
    fn rejects_empty_key_and_oversized_key() {
        let mut trie = BinaryTrie::new_temp();
        assert_eq!(trie.insert(vec![], [0; 32]), Err(BinaryTrieError::EmptyKey));
        assert_eq!(
            trie.insert(vec![0; 8193], [0; 32]),
            Err(BinaryTrieError::KeyTooLong)
        );
    }

    #[test]
    fn rejects_prefix_violations_both_directions() {
        let mut trie = BinaryTrie::new_temp();
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
        let mut trie = BinaryTrie::new_temp();
        trie.insert(vec![0xaa, 0xbb], [1; 32]).unwrap();
        let root_before = trie.root();
        let _ = trie.insert(vec![0xaa], [2; 32]);
        let _ = trie.insert(vec![0xaa, 0xbb, 0xcc], [2; 32]);
        assert_eq!(trie.root(), root_before);
        assert_eq!(trie.get(&[0xaa, 0xbb]).unwrap(), Some([1; 32]));
    }

    #[test]
    fn failed_insert_below_branch_leaves_trie_unchanged() {
        // Two keys sharing their first 9 bits force a root branch with
        // a long prefix, so both Branch-arm error sites are reachable.
        let mut trie = BinaryTrie::new_temp();
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
        assert_eq!(trie.get(&[0xaa, 0xbb]).unwrap(), Some([1; 32]));
        assert_eq!(trie.get(&[0xaa, 0xcc]).unwrap(), Some([2; 32]));
    }

    #[test]
    fn insertion_order_does_not_change_root() {
        let keys: [&[u8]; 3] = [&[0xf0, 0x00], &[0xf1, 0x00], &[0x0f, 0x00]];
        let mut forward = BinaryTrie::new_temp();
        let mut reverse = BinaryTrie::new_temp();
        for k in keys {
            forward.insert(k.to_vec(), [9; 32]).unwrap();
        }
        for k in keys.iter().rev() {
            reverse.insert(k.to_vec(), [9; 32]).unwrap();
        }
        assert_eq!(forward.root(), reverse.root());
    }
}
