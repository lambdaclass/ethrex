mod branch;
mod extension;
mod leaf;

use std::sync::Arc;
#[cfg(not(all(feature = "eip-8025", target_arch = "riscv64")))]
pub use std::sync::OnceLock;

/// `OnceLock` replacement for zkVM guest gated on `eip-8025` feature
///
/// `std::sync::OnceLock` atomics are pure overhead in zkVM guest.
/// This struct copies the methods from `once_cell::unsync::OnceCell` and uses unsafe
/// to get around the Sync requirement.
///
/// This code is only sound because the guest is guaranteed to be single-threaded.
#[cfg(all(feature = "eip-8025", target_arch = "riscv64"))]
pub struct OnceLock<T>(core::cell::UnsafeCell<Option<T>>);

#[cfg(all(feature = "eip-8025", target_arch = "riscv64"))]
unsafe impl<T: Sync> Sync for OnceLock<T> {}

#[cfg(all(feature = "eip-8025", target_arch = "riscv64"))]
impl<T> OnceLock<T> {
    #[inline]
    fn new() -> Self {
        Self(core::cell::UnsafeCell::new(None))
    }

    #[inline]
    fn get(&self) -> Option<&T> {
        unsafe { &*self.0.get() }.as_ref()
    }

    #[inline]
    fn get_or_init(&self, f: impl FnOnce() -> T) -> &T {
        match self.get_or_try_init(|| Ok::<T, core::convert::Infallible>(f())) {
            Ok(val) => val,
            Err(e) => match e {},
        }
    }

    #[inline]
    fn get_or_try_init<E>(&self, f: impl FnOnce() -> Result<T, E>) -> Result<&T, E> {
        if let Some(val) = self.get() {
            return Ok(val);
        }
        self.try_init(f)
    }

    #[inline]
    fn set(&self, value: T) -> Result<(), T> {
        match self.try_insert(value) {
            Ok(_) => Ok(()),
            Err((_, value)) => Err(value),
        }
    }

    #[inline]
    fn try_insert(&self, value: T) -> Result<&T, (&T, T)> {
        if let Some(old) = self.get() {
            return Err((old, value));
        }
        let slot = unsafe { &mut *self.0.get() };
        Ok(slot.insert(value))
    }

    #[inline]
    fn try_init<E>(&self, f: impl FnOnce() -> Result<T, E>) -> Result<&T, E> {
        let val = f()?;
        let slot = unsafe { &mut *self.0.get() };
        debug_assert!(slot.is_none());
        Ok(slot.insert(val))
    }

    #[inline]
    fn take(&mut self) -> Option<T> {
        self.0.get_mut().take()
    }
}

#[cfg(all(feature = "eip-8025", target_arch = "riscv64"))]
impl<T: PartialEq> PartialEq for OnceLock<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

#[cfg(all(feature = "eip-8025", target_arch = "riscv64"))]
impl<T> Default for OnceLock<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(feature = "eip-8025", target_arch = "riscv64"))]
impl<T: Eq> Eq for OnceLock<T> {}

#[cfg(all(feature = "eip-8025", target_arch = "riscv64"))]
impl<T: Clone> Clone for OnceLock<T> {
    #[inline]
    fn clone(&self) -> OnceLock<T> {
        match self.get() {
            Some(value) => OnceLock::from(value.clone()),
            None => OnceLock::new(),
        }
    }
}

#[cfg(all(feature = "eip-8025", target_arch = "riscv64"))]
impl<T: std::fmt::Debug> std::fmt::Debug for OnceLock<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_tuple("OnceLock");
        match self.get() {
            Some(v) => d.field(v),
            None => d.field(&format_args!("<uninit>")),
        };
        d.finish()
    }
}

#[cfg(all(feature = "eip-8025", target_arch = "riscv64"))]
impl<T> From<T> for OnceLock<T> {
    #[inline]
    fn from(value: T) -> Self {
        OnceLock {
            0: core::cell::UnsafeCell::new(Some(value)),
        }
    }
}

pub use branch::BranchNode;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
pub use extension::ExtensionNode;
pub use leaf::LeafNode;
use rkyv::{
    de::Pooling,
    rancor::Source,
    ser::{Allocator, Sharing, Writer},
    validation::{ArchiveContext, SharedContext},
    with::Skip,
};

use ethrex_crypto::{Crypto, NativeCrypto};

use crate::{NodeRLP, TrieDB, error::TrieError, nibbles::Nibbles};

use super::{ValueRLP, node_hash::NodeHash};

/// A reference to a node.
///
/// Explicit rkyv bounds are needed because this is a recursive type, whose
/// bounds can't be automatically resolved.
#[derive(
    Clone,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Serialize,
    rkyv::Deserialize,
    rkyv::Archive,
)]
#[rkyv(serialize_bounds(__S: Writer + Allocator + Sharing, __S::Error: Source))]
#[rkyv(deserialize_bounds(__D: Pooling, __D::Error: Source))]
#[rkyv(bytecheck(bounds(__C: ArchiveContext + SharedContext)))]
pub enum NodeRef {
    /// The node is embedded within the reference.
    Node(
        #[rkyv(omit_bounds)] Arc<Node>,
        #[rkyv(with = Skip)]
        #[serde(skip)]
        OnceLock<NodeHash>,
    ),
    /// The node is in the database, referenced by its hash.
    Hash(NodeHash),
}

impl NodeRef {
    /// Gets a shared reference to the inner node.
    /// Requires that the trie is in a consistent state, ie that all leaves being pointed are in the database.
    /// Outside of snapsync this should always be the case.
    pub fn get_node(&self, db: &dyn TrieDB, path: Nibbles) -> Result<Option<Arc<Node>>, TrieError> {
        match self {
            NodeRef::Node(node, _) => Ok(Some(node.clone())),
            NodeRef::Hash(hash @ NodeHash::Inline(_)) => {
                Ok(Some(Arc::new(Node::decode(hash.as_ref())?)))
            }
            NodeRef::Hash(_) => db
                .get(path)?
                .filter(|rlp| !rlp.is_empty())
                .map(|rlp| Ok(Arc::new(Node::decode(&rlp)?)))
                .transpose(),
        }
    }

    /// Gets a shared reference to the inner node, checking its hash.
    /// Returns `Ok(None)` if the hash is invalid.
    ///
    /// Uses `NativeCrypto` directly because this function is only reachable from
    /// native storage/sync paths (`get_root_node`, `get_proof`, `validate`,
    /// `verify_range`, trie iterator) — never from the guest program path, which
    /// traverses via `Node::get()`.
    pub fn get_node_checked(
        &self,
        db: &dyn TrieDB,
        path: Nibbles,
    ) -> Result<Option<Arc<Node>>, TrieError> {
        match self {
            NodeRef::Node(node, _) => Ok(Some(node.clone())),
            NodeRef::Hash(hash @ NodeHash::Inline(_)) => {
                Ok(Some(Arc::new(Node::decode(hash.as_ref())?)))
            }
            NodeRef::Hash(hash @ NodeHash::Hashed(_)) => {
                db.get(path)?
                    .filter(|rlp| !rlp.is_empty())
                    .and_then(|rlp| match Node::decode(&rlp) {
                        Ok(node) => (node.compute_hash(&NativeCrypto) == *hash)
                            .then_some(Ok(Arc::new(node))),
                        Err(err) => Some(Err(TrieError::RLPDecode(err))),
                    })
                    .transpose()
            }
        }
    }

    /// Gets a mutable shared reference to the inner node.
    ///
    /// # Caution
    ///
    /// 1. If more than one strong reference exists to this node, it will be cloned (see `Arc::make_mut`).
    /// 2. Mutating the inner node without updating parents can lead to trie inconsistencies.
    pub(crate) fn get_node_mut(
        &mut self,
        db: &dyn TrieDB,
        path: Nibbles,
    ) -> Result<Option<&mut Node>, TrieError> {
        match self {
            NodeRef::Node(node, _) => Ok(Some(Arc::make_mut(node))),
            NodeRef::Hash(hash @ NodeHash::Inline(_)) => {
                let node = Node::decode(hash.as_ref())?;
                *self = NodeRef::Node(Arc::new(node), OnceLock::from(*hash));
                self.get_node_mut(db, path)
            }
            NodeRef::Hash(hash @ NodeHash::Hashed(_)) => {
                let Some(node) = db
                    .get(path.clone())?
                    .filter(|rlp| !rlp.is_empty())
                    .map(|rlp| Node::decode(&rlp).map_err(TrieError::RLPDecode))
                    .transpose()?
                else {
                    return Ok(None);
                };
                *self = NodeRef::Node(Arc::new(node), OnceLock::from(*hash));
                self.get_node_mut(db, path)
            }
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            NodeRef::Node(_, _) => true,
            NodeRef::Hash(hash) => hash.is_valid(),
        }
    }

    /// Hash every dirty node of the subtrie rooted here and collect their
    /// `(path, encoding)` entries (plus leaf flat key/values) into `acc`.
    ///
    /// Dirty nodes are hashed in level order, deepest first, batching each depth
    /// through [`Crypto::keccak256_batch`] so the 4-way SIMD lanes fill across
    /// the whole level (see [`batch_commit_subtrie`]). To keep that batching
    /// cache-resident regardless of trie size, the work is bounded by
    /// [`commit_dirty`]: subtrees larger than a cache-sized cap are split by
    /// recursing into children first, so a huge trie is committed as many small
    /// cache-resident batched subtries rather than one level-order sweep over
    /// the whole heap. A node whose hash is already memoized is clean and is
    /// skipped — the memoized `OnceLock` is the dirtiness marker.
    pub fn commit(
        &self,
        path: Nibbles,
        acc: &mut Vec<(Nibbles, Vec<u8>)>,
        crypto: &dyn Crypto,
    ) -> NodeHash {
        match self {
            NodeRef::Node(_, hash) => {
                if let Some(hash) = hash.get() {
                    return *hash;
                }
                commit_dirty(self, path, acc, crypto, BATCH_SUBTREE_CAP);
                *hash.get().expect("commit_dirty hashes the root node")
            }
            NodeRef::Hash(hash) => *hash,
        }
    }

    pub fn compute_hash(&self, crypto: &dyn Crypto) -> NodeHash {
        *self.compute_hash_ref(crypto)
    }

    pub fn compute_hash_ref(&self, crypto: &dyn Crypto) -> &NodeHash {
        match self {
            NodeRef::Node(node, hash) => hash.get_or_init(|| node.compute_hash(crypto)),
            NodeRef::Hash(hash) => hash,
        }
    }

    pub fn compute_hash_no_alloc(&self, buf: &mut Vec<u8>, crypto: &dyn Crypto) -> &NodeHash {
        match self {
            NodeRef::Node(node, hash) => {
                hash.get_or_init(|| node.compute_hash_no_alloc(buf, crypto))
            }
            NodeRef::Hash(hash) => hash,
        }
    }

    pub fn memoize_hashes(&self, buf: &mut Vec<u8>, crypto: &dyn Crypto) {
        if let NodeRef::Node(node, hash) = &self
            && hash.get().is_none()
        {
            node.memoize_hashes(buf, crypto);
            let _ = hash.set(node.compute_hash_no_alloc(buf, crypto));
        }
    }

    /// Resets the memoized hash of this Node
    ///
    /// This is used when mutating a node in place, in which case the memoized hash
    /// is not valid anymore.
    pub fn clear_hash(&mut self) {
        if let NodeRef::Node(_, hash) = self {
            hash.take();
        }
    }
}

impl Default for NodeRef {
    fn default() -> Self {
        Self::Hash(NodeHash::default())
    }
}

impl From<Node> for NodeRef {
    fn from(value: Node) -> Self {
        Self::Node(Arc::new(value), OnceLock::new())
    }
}

impl From<NodeHash> for NodeRef {
    fn from(value: NodeHash) -> Self {
        Self::Hash(value)
    }
}

impl From<Arc<Node>> for NodeRef {
    fn from(value: Arc<Node>) -> Self {
        Self::Node(value, OnceLock::new())
    }
}

impl PartialEq for NodeRef {
    fn eq(&self, other: &Self) -> bool {
        let mut buf = Vec::new();
        self.compute_hash_no_alloc(&mut buf, &NativeCrypto)
            == other.compute_hash_no_alloc(&mut buf, &NativeCrypto)
    }
}

pub enum ValueOrHash {
    Value(ValueRLP),
    Hash(NodeHash),
}

impl From<ValueRLP> for ValueOrHash {
    fn from(value: ValueRLP) -> Self {
        Self::Value(value)
    }
}

impl From<NodeHash> for ValueOrHash {
    fn from(value: NodeHash) -> Self {
        Self::Hash(value)
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Deserialize,
    rkyv::Serialize,
    rkyv::Archive,
)]
/// A Node in an Ethereum Compatible Patricia Merkle Trie
pub enum Node {
    Branch(Box<BranchNode>),
    Extension(ExtensionNode),
    Leaf(LeafNode),
}

impl Default for Node {
    fn default() -> Self {
        // empty leaf node as a placeholder
        Self::Leaf(LeafNode {
            partial: Nibbles::from_bytes(&[]),
            value: Vec::new(),
        })
    }
}

impl From<Box<BranchNode>> for Node {
    fn from(val: Box<BranchNode>) -> Self {
        Node::Branch(val)
    }
}

impl From<BranchNode> for Node {
    fn from(val: BranchNode) -> Self {
        Node::Branch(Box::new(val))
    }
}

impl From<ExtensionNode> for Node {
    fn from(val: ExtensionNode) -> Self {
        Node::Extension(val)
    }
}

impl From<LeafNode> for Node {
    fn from(val: LeafNode) -> Self {
        Node::Leaf(val)
    }
}

impl Node {
    /// Retrieves a value from the subtrie originating from this node given its path
    pub fn get(&self, db: &dyn TrieDB, path: Nibbles) -> Result<Option<ValueRLP>, TrieError> {
        match self {
            Node::Branch(n) => n.get(db, path),
            Node::Extension(n) => n.get(db, path),
            Node::Leaf(n) => n.get(path),
        }
    }

    /// Inserts a value into the subtrie originating from this node.
    pub fn insert(
        &mut self,
        db: &dyn TrieDB,
        path: Nibbles,
        value: impl Into<ValueOrHash>,
    ) -> Result<(), TrieError> {
        let new_node = match self {
            Node::Branch(n) => {
                n.insert(db, path, value.into())?;
                Ok(None)
            }
            Node::Extension(n) => n.insert(db, path, value.into()),
            Node::Leaf(n) => n.insert(path, value.into()),
        };
        if let Some(new_node) = new_node? {
            *self = new_node;
        }
        Ok(())
    }

    /// Removes a value from the subtrie originating from this node given its path
    /// Returns a bool indicating if the new subtrie is empty, and the removed value if it existed in the subtrie
    pub fn remove(
        &mut self,
        db: &dyn TrieDB,
        path: Nibbles,
    ) -> Result<(bool, Option<ValueRLP>), TrieError> {
        let (new_root, value) = match self {
            Node::Branch(n) => n.remove(db, path),
            Node::Extension(n) => n.remove(db, path),
            Node::Leaf(n) => n.remove(path),
        }?;

        let is_trie_empty = new_root.is_none();
        if let Some(NodeRemoveResult::New(new_root)) = new_root {
            *self = new_root;
        }
        Ok((is_trie_empty, value))
    }

    /// Traverses own subtrie until reaching the node containing `path`
    /// Appends all encoded nodes traversed to `node_path` (including self)
    /// Only nodes with encoded len over or equal to 32 bytes are included
    pub fn get_path(
        &self,
        db: &dyn TrieDB,
        path: Nibbles,
        node_path: &mut Vec<Vec<u8>>,
    ) -> Result<(), TrieError> {
        match self {
            Node::Branch(n) => n.get_path(db, path, node_path),
            Node::Extension(n) => n.get_path(db, path, node_path),
            Node::Leaf(n) => n.get_path(node_path),
        }
    }

    /// Computes the node's hash
    pub fn compute_hash(&self, crypto: &dyn Crypto) -> NodeHash {
        let mut buf = Vec::new();
        self.memoize_hashes(&mut buf, crypto);
        match self {
            Node::Branch(n) => n.compute_hash_no_alloc(&mut buf, crypto),
            Node::Extension(n) => n.compute_hash_no_alloc(&mut buf, crypto),
            Node::Leaf(n) => n.compute_hash_no_alloc(&mut buf, crypto),
        }
    }

    /// Computes the node's hash
    pub fn compute_hash_no_alloc(&self, buf: &mut Vec<u8>, crypto: &dyn Crypto) -> NodeHash {
        self.memoize_hashes(buf, crypto);
        match self {
            Node::Branch(n) => n.compute_hash_no_alloc(buf, crypto),
            Node::Extension(n) => n.compute_hash_no_alloc(buf, crypto),
            Node::Leaf(n) => n.compute_hash_no_alloc(buf, crypto),
        }
    }

    /// Recursively memoizes the hashes of all nodes of the subtrie that has
    /// `self` as root (post-order traversal)
    /// Recursively memoizes the hashes of all descendant nodes of `self`.
    ///
    /// Nodes are hashed in level order, deepest first: every node at a given
    /// depth is mutually independent (a node's hash depends only on its
    /// children, which are strictly deeper), so a whole level is hashed in one
    /// batched keccak call. This fills the 4-way SIMD lanes far better than
    /// hashing a single branch's ≤16 children, and reuses `buf` as a single
    /// encoding arena so there is no per-node allocation.
    pub fn memoize_hashes(&self, buf: &mut Vec<u8>, crypto: &dyn Crypto) {
        batch_memoize_subtrie(self, buf, crypto);
    }

    /// Recursively encodes all embedded nodes of the subtrie that has
    /// `self` as root.
    ///
    /// This won't encode nodes which are not embedded in `self`.
    pub fn encode_subtrie(&self, encoded: &mut Vec<NodeRLP>) -> Result<(), TrieError> {
        match self {
            Node::Branch(node) => {
                for choice in &node.choices {
                    if let NodeRef::Node(choice, _) = choice {
                        choice.encode_subtrie(encoded)?;
                    }
                }
            }
            Node::Extension(node) => {
                if let NodeRef::Node(child, _) = &node.child {
                    child.encode_subtrie(encoded)?;
                }
            }
            Node::Leaf(_) => {}
        };

        encoded.push(self.encode_to_vec());
        Ok(())
    }
}

/// Group every unhashed descendant `NodeRef::Node` of `node` by depth
/// (`node`'s direct children are depth 0). Subtrees whose root already carries a
/// memoized hash are skipped entirely.
fn collect_unhashed_by_depth<'a>(
    node: &'a Node,
    depth: usize,
    by_depth: &mut Vec<Vec<&'a NodeRef>>,
) {
    let children: &[NodeRef] = match node {
        Node::Branch(n) => &n.choices,
        Node::Extension(n) => std::slice::from_ref(&n.child),
        Node::Leaf(_) => return,
    };
    if by_depth.len() <= depth {
        by_depth.resize_with(depth + 1, Vec::new);
    }
    for child in children {
        if let NodeRef::Node(child_node, hash) = child
            && hash.get().is_none()
        {
            by_depth[depth].push(child);
            collect_unhashed_by_depth(child_node, depth + 1, by_depth);
        }
    }
}

/// Memoize the hashes of all descendants of `root` in level order, deepest
/// first, batching each level through [`Crypto::keccak256_batch`].
///
/// Correctness relies on the depth ordering: when a level is encoded, every
/// child hash it embeds was memoized by an earlier (deeper) level, so
/// `BranchNode`/`ExtensionNode` encoding reads cached hashes and never triggers
/// a fallback keccak. Encodings under 32 bytes inline exactly as
/// [`NodeHash::from_encoded`] would.
fn batch_memoize_subtrie(root: &Node, buf: &mut Vec<u8>, crypto: &dyn Crypto) {
    let mut by_depth: Vec<Vec<&NodeRef>> = Vec::new();
    collect_unhashed_by_depth(root, 0, &mut by_depth);

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for level in by_depth.iter().rev() {
        buf.clear();
        ranges.clear();
        for &node_ref in level {
            if let NodeRef::Node(node, _) = node_ref {
                let start = buf.len();
                node.encode(buf);
                ranges.push((start, buf.len() - start));
            }
        }

        // Hash the 32+ byte encodings as one batch; shorter ones inline.
        let batch_inputs: Vec<&[u8]> = ranges
            .iter()
            .filter(|&&(_, len)| len >= 32)
            .map(|&(start, len)| &buf[start..start + len])
            .collect();
        let mut hashed = crypto.keccak256_batch(&batch_inputs).into_iter();

        for (&node_ref, &(start, len)) in level.iter().zip(ranges.iter()) {
            if let NodeRef::Node(_, hash) = node_ref {
                let node_hash = if len >= 32 {
                    NodeHash::from_slice(&hashed.next().expect("one hash per 32+ byte encoding"))
                } else {
                    NodeHash::from_slice(&buf[start..start + len])
                };
                let _ = hash.set(node_hash);
            }
        }
    }
}

/// Group every dirty (unhashed) node of the subtrie rooted at `node_ref` by
/// depth (the root is depth 0), recording each node's trie path. Subtrees whose
/// root already carries a memoized hash are clean and skipped; child paths are
/// only built for nodes actually descended into.
fn collect_dirty_by_depth<'a>(
    node_ref: &'a NodeRef,
    path: Nibbles,
    depth: usize,
    by_depth: &mut Vec<Vec<(Nibbles, &'a NodeRef)>>,
) {
    let NodeRef::Node(node, hash) = node_ref else {
        return;
    };
    if hash.get().is_some() {
        return;
    }
    if by_depth.len() <= depth {
        by_depth.resize_with(depth + 1, Vec::new);
    }
    match node.as_ref() {
        Node::Branch(branch) => {
            for (choice, child) in branch.choices.iter().enumerate() {
                if matches!(child, NodeRef::Node(_, h) if h.get().is_none()) {
                    collect_dirty_by_depth(
                        child,
                        path.append_new(choice as u8),
                        depth + 1,
                        by_depth,
                    );
                }
            }
        }
        Node::Extension(ext) => {
            if matches!(&ext.child, NodeRef::Node(_, h) if h.get().is_none()) {
                collect_dirty_by_depth(&ext.child, path.concat(&ext.prefix), depth + 1, by_depth);
            }
        }
        Node::Leaf(_) => {}
    }
    by_depth[depth].push((path, node_ref));
}

/// Dirty subtrees with at most this many nodes are committed as one batched
/// level-order sweep ([`batch_commit_subtrie`]); larger ones are split. The cap
/// keeps each sweep's working set (the `by_depth` collection, the encoding
/// arena, the batch buffers) cache-resident, so a large trie is committed as
/// many small batched subtries instead of one heap-wide sweep whose cache
/// misses would erode the SIMD win. Sized (empirically) above typical per-block
/// / per-shard commits so those are batched whole and unaffected, while very
/// large tries (e.g. a monolithic state-trie rebuild) are split to bound the
/// working set.
const BATCH_SUBTREE_CAP: usize = 16384;

/// Commit the dirty subtrie rooted at `node_ref`, keeping each batched sweep
/// cache-resident: if the subtree is small enough, batch it wholesale;
/// otherwise recurse into its dirty children first (each a smaller subtree) and
/// then hash `node_ref` itself as a single node.
fn commit_dirty(
    node_ref: &NodeRef,
    path: Nibbles,
    acc: &mut Vec<(Nibbles, Vec<u8>)>,
    crypto: &dyn Crypto,
    cap: usize,
) {
    let NodeRef::Node(node, hash) = node_ref else {
        return;
    };
    if hash.get().is_some() {
        return;
    }
    if dirty_subtree_within(node_ref, cap) {
        batch_commit_subtrie(node_ref, path, acc, crypto);
        return;
    }

    // Subtree too large to batch cache-resident: split it. A leaf is a size-1
    // subtree so it is always batched above; only branches/extensions reach here.
    match node.as_ref() {
        Node::Branch(branch) => {
            for (choice, child) in branch.choices.iter().enumerate() {
                if matches!(child, NodeRef::Node(_, h) if h.get().is_none()) {
                    commit_dirty(child, path.append_new(choice as u8), acc, crypto, cap);
                }
            }
        }
        Node::Extension(ext) => {
            if matches!(&ext.child, NodeRef::Node(_, h) if h.get().is_none()) {
                commit_dirty(&ext.child, path.concat(&ext.prefix), acc, crypto, cap);
            }
        }
        Node::Leaf(_) => {}
    }

    // Children are now hashed; hash and emit this node (single, scalar).
    let mut buf = Vec::new();
    node.encode(&mut buf);
    let node_hash = NodeHash::from_encoded(&buf, crypto);
    let _ = hash.set(node_hash);
    if let Node::Leaf(leaf) = node.as_ref() {
        acc.push((path.concat(&leaf.partial), leaf.value.clone()));
    }
    acc.push((path, buf));
}

/// True if the dirty subtree rooted at `node_ref` has at most `cap` nodes.
/// Walks at most `cap + 1` nodes (stops as soon as the budget is exceeded), so
/// it is cheap even for very large subtrees.
fn dirty_subtree_within(node_ref: &NodeRef, cap: usize) -> bool {
    fn walk(node_ref: &NodeRef, budget: &mut usize) -> bool {
        let NodeRef::Node(node, hash) = node_ref else {
            return true;
        };
        if hash.get().is_some() {
            return true;
        }
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        match node.as_ref() {
            Node::Branch(branch) => {
                for child in branch.choices.iter() {
                    if !walk(child, budget) {
                        return false;
                    }
                }
            }
            Node::Extension(ext) => {
                if !walk(&ext.child, budget) {
                    return false;
                }
            }
            Node::Leaf(_) => {}
        }
        true
    }
    let mut budget = cap;
    walk(node_ref, &mut budget)
}

/// Commit the subtrie rooted at `root_ref`: hash every dirty node in level
/// order, deepest first, batching each level through [`Crypto::keccak256_batch`],
/// and push each node's `(path, encoding)` (and, for leaves, the flat
/// key/value) into `acc`.
///
/// Deepest-first ordering guarantees a node's children are memoized before it
/// is encoded, so `BranchNode`/`ExtensionNode` encoding reads cached child
/// hashes and never triggers a fallback keccak. Node encodings live in a single
/// reused `arena` (offset/len ranges), and each `acc` entry is an exact-size
/// copy — no per-node growth reallocation.
fn batch_commit_subtrie(
    root_ref: &NodeRef,
    root_path: Nibbles,
    acc: &mut Vec<(Nibbles, Vec<u8>)>,
    crypto: &dyn Crypto,
) {
    let mut by_depth: Vec<Vec<(Nibbles, &NodeRef)>> = Vec::new();
    collect_dirty_by_depth(root_ref, root_path, 0, &mut by_depth);

    // Process each level in bounded windows so the transient arena / batch
    // buffers stay cache-resident: a full leaf level of a large trie would
    // otherwise make multi-MB temporaries and thrash cache, erasing the win.
    const WINDOW: usize = 512;
    let mut arena: Vec<u8> = Vec::new();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    // Consume `by_depth` so each node's collected path is *moved* into `acc`
    // rather than cloned — one `Nibbles` allocation per node, matching the
    // scalar commit (the extra clone was the main source of run-to-run jitter).
    for mut level in by_depth.into_iter().rev() {
        for window in level.chunks_mut(WINDOW) {
            arena.clear();
            ranges.clear();
            for (_, node_ref) in window.iter() {
                if let NodeRef::Node(node, _) = node_ref {
                    let start = arena.len();
                    node.encode(&mut arena);
                    ranges.push((start, arena.len() - start));
                }
            }

            let batch_inputs: Vec<&[u8]> = ranges
                .iter()
                .filter(|&&(_, len)| len >= 32)
                .map(|&(start, len)| &arena[start..start + len])
                .collect();
            let mut hashed = crypto.keccak256_batch(&batch_inputs).into_iter();

            for ((node_path, node_ref), &(start, len)) in window.iter_mut().zip(ranges.iter()) {
                let NodeRef::Node(node, hash) = *node_ref else {
                    continue;
                };
                let encoding = &arena[start..start + len];
                let node_hash = if len >= 32 {
                    NodeHash::from_slice(&hashed.next().expect("one hash per 32+ byte encoding"))
                } else {
                    NodeHash::from_slice(encoding)
                };
                let _ = hash.set(node_hash);
                if let Node::Leaf(leaf) = node.as_ref() {
                    acc.push((node_path.concat(&leaf.partial), leaf.value.clone()));
                }
                acc.push((std::mem::take(node_path), encoding.to_vec()));
            }
        }
    }
}

/// Used as return type for `Node` remove operations that may resolve into either:
/// - a mutation of the `Node`
/// - a new `Node`
pub enum NodeRemoveResult {
    Mutated,
    New(Node),
}
