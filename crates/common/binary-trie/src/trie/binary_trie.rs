//! Incrementally updated binary radix trie, backed by a
//! [`BinaryTrieDB`].
//!
//! The trie retains its node structure across updates, splitting nodes
//! on insertion and collapsing them on removal, and hashes that
//! structure on [`BinaryTrie::root`]. Canonical-form invariant: a
//! branch's prefix is exactly the bits its two subtrees share beyond
//! the parent split, so the structure — and therefore the root —
//! depends only on the key/value set, never on the order of the
//! insertions and removals that arrived at it. A split and a collapse
//! are inverses: the one moves bits from a prefix into a new branch
//! above, the other folds them back into the survivor below.
//!
//! A whole trie can also be built at once, by
//! [`BinaryTrie::from_sorted_leaves`], from leaves already in bit
//! order. That the structure is canonical is what makes this possible:
//! sorted input determines the tree, so a single bottom-up fold builds
//! every node in its final shape rather than splitting and re-splitting
//! its way there.
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
//! A removal also leaves paths behind: the leaf's own, and the
//! survivor's, once a collapse moves it up. [`BinaryTrie::commit`]
//! sends those to the database as empty-valued entries — tombstones
//! the backend deletes — in the same batch as the writes.
//!
//! Recursion depth is bounded by the key length in bits, so max-length
//! keys could theoretically overflow the stack; embedding keys are at
//! most 66 bytes, and going iterative is the fallback if that ever
//! matters.

use std::collections::{BTreeSet, HashSet};
use std::sync::OnceLock;

use ethereum_types::H256;

use crate::error::BinaryTrieError;

use super::MAX_KEY_LENGTH;
use super::bits::{bits_start_with, bytes_to_bits};
use super::db::{BinaryTrieDB, InMemoryBinaryTrieDB};
use super::node::{
    EMPTY_TRIE_ROOT, StoredNode, blake3_hash, branch_hash, decode, encode_branch, encode_leaf,
    leaf_hash,
};
use super::path::BitPath;
use super::prefix::KeyPrefix;

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

/// A leaf awaiting its place in a bulk build, with its key expanded to
/// bits once up front rather than at every level of the fold that
/// passes over it.
struct SortedLeaf {
    bits: Vec<u8>,
    key: Vec<u8>,
    value: [u8; 32],
}

/// What a removal did to the subtree it was applied to, reported back
/// up the descent.
enum Removal {
    /// The key is not in this subtree; nothing changed.
    Absent,
    /// The key was removed and this subtree still has a node at its
    /// path — either because the removal was deeper down, or because
    /// this node collapsed into its surviving child.
    Removed([u8; 32]),
    /// The key was removed and *this node* was it, so the reference to
    /// it must go. Only the parent can act on that: it alone knows the
    /// sibling that takes its place.
    Vanished([u8; 32]),
}

/// Where a node's subtree sits relative to a [`KeyPrefix`] the descent
/// is following.
///
/// Computed by [`BinaryTrie::locate`] and shared by the two prefix
/// operations, which differ only in what they do with the answer.
enum Span {
    /// No key below this node is under the prefix.
    Outside,
    /// *Every* key below this node is under the prefix, because the
    /// prefix ran out at or inside this node. A branch always has two
    /// children and a leaf always has a value, so the subtree is never
    /// empty — which is what makes an existence check stop here.
    Covered,
    /// The prefix reaches past this branch, so only part of its subtree
    /// can be under it: continue into the `bit` child, whose own path
    /// consumes bits up to and including `split`.
    Below { split: usize, bit: u8 },
}

/// What a prefix removal did to the subtree it was applied to, the
/// [`Removal`] of [`BinaryTrie::remove_prefix`]. It counts leaves rather
/// than carrying a value, there being no single value to return.
enum PrefixRemoval {
    /// Nothing under this subtree was under the prefix.
    Absent,
    /// Leaves went and this subtree still has a node at its path.
    Removed(usize),
    /// Leaves went and this subtree is *entirely* gone, so the reference
    /// to it must go. Only the parent can act on that.
    Vanished(usize),
}

/// Compressed binary radix trie over prefix-free byte keys and
/// 32-byte values, committing to its contents with a BLAKE3 root.
pub struct BinaryTrie {
    db: Box<dyn BinaryTrieDB>,
    root: Option<NodeRef>,
    /// Paths whose nodes left the tree since the last commit, to be
    /// deleted from the database by the next one.
    ///
    /// Ordered rather than hashed so a commit batch is deterministic;
    /// the set is at most two paths per removal, so the ordering costs
    /// nothing worth counting.
    pending_removal: BTreeSet<BitPath>,
}

impl BinaryTrie {
    /// An empty trie writing to `db`.
    pub fn new(db: Box<dyn BinaryTrieDB>) -> Self {
        Self {
            db,
            root: None,
            pending_removal: BTreeSet::new(),
        }
    }

    /// The trie `db` holds under `root`, with nothing loaded yet.
    ///
    /// Nodes are read lazily, so a wrong or absent root only surfaces
    /// when a traversal reaches a node that is not there.
    pub fn open(db: Box<dyn BinaryTrieDB>, root: H256) -> Self {
        Self {
            db,
            root: (root != EMPTY_TRIE_ROOT).then_some(NodeRef::Stored(root)),
            pending_removal: BTreeSet::new(),
        }
    }

    /// An empty trie over a fresh in-memory backend.
    pub fn new_temp() -> Self {
        Self::new(Box::new(InMemoryBinaryTrieDB::new_empty()))
    }

    /// Build a trie over `leaves`, which must already be in ascending
    /// bit order and hold each key once.
    ///
    /// One bottom-up pass instead of one descent per key. Repeated
    /// [`BinaryTrie::insert`] reaches the same tree, but walks from the
    /// root every time and may split a branch that a later insertion
    /// splits again; a sorted fold visits each node once and builds it
    /// in its final shape. Genesis needs this — the whole state arrives
    /// at once as an alloc — and so, later, does snapshot import.
    ///
    /// **Ordering.** Sort by bytes: for the prefix-free keys this trie
    /// accepts, plain lexicographic byte order *is* bit order. The two
    /// disagree only when one key's bits are a prefix of another's —
    /// byte order puts the shorter first, bit order has nothing to say
    /// — and that is exactly the case the trie rejects.
    ///
    /// **Why sorting is enough to determine the shape.** A run of
    /// leaves agreeing on their first `depth` bits is one subtree; it
    /// is a single [`Node::Leaf`] if it holds one leaf, and otherwise a
    /// [`Node::Branch`] whose prefix runs from `depth` to the first bit
    /// the run disagrees on, splitting there. Because the input is
    /// sorted, the two sides of that split are contiguous — a boundary
    /// index, not a filter — and the run's shared prefix is whatever
    /// its first and last leaves share, with no need to look between.
    ///
    /// A constructor rather than an `extend`: merging a sorted stream
    /// into an existing tree is a different and harder problem, so
    /// there is no API here to misuse for it.
    ///
    /// Nothing is written to the database — every node comes out dirty
    /// with its hash unknown, and [`BinaryTrie::commit`] writes the
    /// tree. The whole input is held in memory as well; that is fine
    /// for the callers this has (mainnet's genesis alloc is under ten
    /// thousand accounts), and truly streaming construction, which
    /// would flush finished subtrees as the fold leaves them behind, is
    /// a later concern.
    ///
    /// # Errors
    ///
    /// The input is validated rather than trusted, and no trie is
    /// returned unless all of it is sound:
    /// - [`BinaryTrieError::EmptyKey`] if any key is empty.
    /// - [`BinaryTrieError::KeyTooLong`] if any key exceeds
    ///   [`MAX_KEY_LENGTH`] bytes.
    /// - [`BinaryTrieError::UnsortedInput`] if a key does not follow
    ///   its predecessor, or repeats it.
    /// - [`BinaryTrieError::PrefixViolation`] if one key's bits are a
    ///   prefix of another's.
    pub fn from_sorted_leaves(
        db: Box<dyn BinaryTrieDB>,
        leaves: Vec<(Vec<u8>, [u8; 32])>,
    ) -> Result<Self, BinaryTrieError> {
        let mut run: Vec<SortedLeaf> = Vec::with_capacity(leaves.len());
        for (key, value) in leaves {
            if key.is_empty() {
                return Err(BinaryTrieError::EmptyKey);
            }
            if key.len() > MAX_KEY_LENGTH {
                return Err(BinaryTrieError::KeyTooLong);
            }
            if let Some(previous) = run.last() {
                Self::check_ascending(&previous.key, &key)?;
            }
            run.push(SortedLeaf {
                bits: bytes_to_bits(&key),
                key,
                value,
            });
        }
        Ok(Self {
            db,
            root: (!run.is_empty()).then(|| NodeRef::loaded(Self::fold(&mut run, 0))),
            pending_removal: BTreeSet::new(),
        })
    }

    /// Check that `next` follows `previous` in bit order and that
    /// neither is a prefix of the other.
    ///
    /// Checking consecutive pairs catches *every* prefix relation, not
    /// only adjacent ones: if `a` is a proper prefix of `b`, then every
    /// key between them in sort order also starts with `a` — one that
    /// did not would differ from `a` inside `a`'s own length, and so
    /// would fall outside the interval entirely. So `a`'s immediate
    /// successor starts with `a`, and this pairwise walk sees it.
    fn check_ascending(previous: &[u8], next: &[u8]) -> Result<(), BinaryTrieError> {
        if previous == next {
            return Err(BinaryTrieError::UnsortedInput);
        }
        // Keys are whole bytes, so a bit-prefix is a byte-prefix. The
        // reversed case is out of order too, but the structural fault
        // is the more specific answer — sorting the input would not
        // make that pair representable.
        if next.starts_with(previous) || previous.starts_with(next) {
            return Err(BinaryTrieError::PrefixViolation);
        }
        if previous > next {
            return Err(BinaryTrieError::UnsortedInput);
        }
        Ok(())
    }

    /// Build the subtree over `run`, a non-empty group of validated
    /// leaves that all agree on their first `depth` bits.
    ///
    /// Consumes each leaf's key on the way past, which is safe because
    /// the recursion partitions `run` into disjoint slices and reaches
    /// every leaf exactly once.
    fn fold(run: &mut [SortedLeaf], depth: usize) -> Node {
        if let [only] = run {
            return Node::Leaf {
                key: std::mem::take(&mut only.key),
                value: only.value,
            };
        }
        // Sorted, so everything between the ends shares whatever the
        // ends share: the run's own prefix is `first`'s bits from
        // `depth` up to the first one `last` disagrees on. Validation
        // rules out the two ways that search could fall off the end —
        // the keys are distinct and neither is a prefix of the other,
        // so they diverge at a bit both of them have.
        let (first, last) = (&run[0].bits, &run[run.len() - 1].bits);
        let split = (depth..first.len())
            .find(|&i| first[i] != last[i])
            .expect("validated leaves are distinct and prefix-free, so they diverge");
        let prefix = first[depth..split].to_vec();

        // `first` sorts below `last`, so the bit they disagree on is 0
        // in one and 1 in the other, and the 0-side is an initial
        // segment of the run: both sides are non-empty.
        let boundary = run.partition_point(|leaf| leaf.bits[split] == 0);
        let (zeros, ones) = run.split_at_mut(boundary);
        Node::Branch {
            prefix,
            left: NodeRef::loaded(Self::fold(zeros, split + 1)),
            right: NodeRef::loaded(Self::fold(ones, split + 1)),
        }
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
        let Self { db, root, .. } = self;
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

    /// Remove `key`, returning the value it held, or `None` if it was
    /// not there.
    ///
    /// Absence is not an error: the spec's state model deletes by
    /// writing zero, and a slot that was already absent is written to
    /// zero as readily as one that was not.
    ///
    /// Takes the tree from one canonical shape to the other: removing
    /// a leaf leaves its parent branch with a single child, which is
    /// not a shape a branch is allowed to have — a branch exists only
    /// where two subtrees diverge. The parent therefore collapses into
    /// its surviving child. See [`BinaryTrie::collapse`].
    ///
    /// # Errors
    ///
    /// [`BinaryTrieError::Backend`] or [`BinaryTrieError::MalformedNode`]
    /// if a node on the path, or the sibling of the removed leaf, could
    /// not be loaded.
    pub fn remove(&mut self, key: &[u8]) -> Result<Option<[u8; 32]>, BinaryTrieError> {
        let bits = bytes_to_bits(key);
        let Self {
            db,
            root,
            pending_removal,
        } = self;
        let Some(node_ref) = root else {
            return Ok(None);
        };
        match Self::remove_at(db.as_ref(), node_ref, &bits, 0, key, pending_removal)? {
            Removal::Absent => Ok(None),
            Removal::Removed(value) => Ok(Some(value)),
            Removal::Vanished(value) => {
                // The root was the matching leaf and nothing is left to
                // take its place.
                pending_removal.insert(BitPath::new());
                *root = None;
                Ok(Some(value))
            }
        }
    }

    /// Remove from the subtree at `node_ref`, invalidating it on the
    /// way back out if anything below it changed — the same discipline
    /// [`BinaryTrie::insert_at`] follows, and for the same reason: the
    /// `&mut` descent visits exactly the ancestors of the change.
    ///
    /// A [`Removal::Absent`] result changed nothing, so it must leave
    /// the cached hash and the dirty flag alone; invalidating there
    /// would make a failed lookup dirty the whole path to the root.
    fn remove_at(
        db: &dyn BinaryTrieDB,
        node_ref: &mut NodeRef,
        bits: &[u8],
        depth: usize,
        key: &[u8],
        pending_removal: &mut BTreeSet<BitPath>,
    ) -> Result<Removal, BinaryTrieError> {
        let result = Self::remove_below(db, node_ref, bits, depth, key, pending_removal);
        if !matches!(result, Ok(Removal::Absent) | Err(_)) {
            node_ref.invalidate();
        }
        result
    }

    /// The removal itself; see [`BinaryTrie::remove_at`], which wraps
    /// it to invalidate the node on the way back out.
    fn remove_below(
        db: &dyn BinaryTrieDB,
        node_ref: &mut NodeRef,
        bits: &[u8],
        depth: usize,
        key: &[u8],
        pending_removal: &mut BTreeSet<BitPath>,
    ) -> Result<Removal, BinaryTrieError> {
        // The descent got here by matching `bits[..depth]`, so those
        // bits are this node's path — the key it is stored under.
        let path = BitPath::from_bits(&bits[..depth]);
        let (bit, value) = match Self::resolve(db, node_ref, &path)? {
            Node::Leaf {
                key: leaf_key,
                value,
            } => {
                return Ok(if leaf_key.as_slice() == key {
                    // The caller unlinks this leaf: only it knows
                    // whether there is a sibling to put in its place.
                    Removal::Vanished(*value)
                } else {
                    Removal::Absent
                });
            }
            Node::Branch {
                prefix,
                left,
                right,
            } => {
                let split = depth + prefix.len();
                if split >= bits.len() || bits[depth..split] != prefix[..] {
                    return Ok(Removal::Absent);
                }
                let bit = bits[split];
                let child = if bit == 0 { left } else { right };
                match Self::remove_at(db, child, bits, split + 1, key, pending_removal)? {
                    // Nothing below vanished, so this branch still has
                    // both its children and keeps its shape.
                    other @ (Removal::Absent | Removal::Removed(_)) => return Ok(other),
                    Removal::Vanished(value) => (bit, value),
                }
            }
        };

        Self::collapse(db, node_ref, &path, bit, pending_removal)?;
        Ok(Removal::Removed(value))
    }

    /// Replace the branch at `node_ref` with its surviving child, the
    /// removed leaf having left it with only one.
    ///
    /// The survivor takes the branch's place *and* absorbs the bits the
    /// branch consumed — its prefix, then the split bit that selected
    /// the survivor — so the same bit string still leads to the same
    /// leaves. A leaf needs no merge: it carries its whole key, and the
    /// path never told it anything the key does not.
    ///
    /// **Path arithmetic, the invariant this rests on.** A child's path
    /// is `parent_path ‖ parent_prefix ‖ split_bit`. The survivor moves
    /// from `path ‖ prefix ‖ bit` up to `path`, and its own prefix
    /// grows by exactly `prefix ‖ bit`. Those two changes cancel: every
    /// node *below* the survivor keeps the path it already had, so a
    /// subtree that is still nothing but a hash stays addressable and
    /// is never rewritten. Only the collapsed node itself moves.
    ///
    /// `bit` is the side the removed leaf was on, so the survivor is
    /// the child on `bit ^ 1`.
    fn collapse(
        db: &dyn BinaryTrieDB,
        node_ref: &mut NodeRef,
        path: &BitPath,
        bit: u8,
        pending_removal: &mut BTreeSet<BitPath>,
    ) -> Result<(), BinaryTrieError> {
        let NodeRef::Loaded { node, .. } = node_ref else {
            unreachable!("resolved on the way in")
        };
        let Node::Branch {
            prefix,
            left,
            right,
        } = node.as_mut()
        else {
            unreachable!("only a branch has a child to lose")
        };
        let removed_path = path.child(prefix, bit);
        let survivor_path = path.child(prefix, bit ^ 1);
        // Load the survivor where it is *now*: the database holds it
        // under its old path, which the move is about to retire.
        let survivor = if bit == 0 { right } else { left };
        Self::resolve(db, survivor, &survivor_path)?;

        // Nothing below can fail from here on, so the branch is never
        // left half-collapsed.
        let Node::Branch {
            mut prefix,
            left,
            right,
        } = std::mem::replace(
            node.as_mut(),
            Node::Leaf {
                key: Vec::new(),
                value: [0; 32],
            },
        )
        else {
            unreachable!("matched a branch above")
        };
        let NodeRef::Loaded { node: survivor, .. } = (if bit == 0 { right } else { left }) else {
            unreachable!("just resolved")
        };
        *node.as_mut() = match *survivor {
            // A leaf carries its whole key, so it loses nothing by
            // moving up: there is no prefix to merge.
            Node::Leaf { key, value } => Node::Leaf { key, value },
            Node::Branch {
                prefix: below,
                left,
                right,
            } => {
                prefix.push(bit ^ 1);
                prefix.extend_from_slice(&below);
                Node::Branch {
                    prefix,
                    left,
                    right,
                }
            }
        };
        // Two paths now hold nodes that are no longer part of the tree.
        // The collapsed branch's own path is not one of them: the
        // survivor occupies it, and being dirty will overwrite it.
        pending_removal.insert(removed_path);
        pending_removal.insert(survivor_path);
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
        let Self { db, root, .. } = self;
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

    /// Whether any key in the trie begins with `prefix`.
    ///
    /// One descent, and it stops at the first node whose whole subtree
    /// lies under `prefix` rather than walking that subtree: a branch
    /// always has two children and a leaf always has a value, so
    /// reaching such a node *is* the answer. So this costs a root-to-node
    /// walk, not a scan — which is the point, since the caller that
    /// needs it asks "does this account hold any storage" for every
    /// account an execution touches.
    ///
    /// # Errors
    ///
    /// - [`BinaryTrieError::EmptyKey`] if `prefix` is empty. The empty
    ///   prefix names the whole trie, so the question it asks is "is
    ///   this trie non-empty" — which [`BinaryTrie::root`] answers
    ///   without reading anything. A caller that reaches here with one
    ///   built an empty prefix by accident, and answering it would hide
    ///   that.
    /// - [`BinaryTrieError::Backend`] or
    ///   [`BinaryTrieError::MalformedNode`] if a node on the path could
    ///   not be loaded.
    pub fn contains_prefix(&mut self, prefix: &KeyPrefix) -> Result<bool, BinaryTrieError> {
        if prefix.is_empty() {
            return Err(BinaryTrieError::EmptyKey);
        }
        let bits = prefix.as_bits();
        let Self { db, root, .. } = self;
        match root {
            None => Ok(false),
            Some(node_ref) => Self::contains_prefix_at(db.as_ref(), node_ref, bits, 0),
        }
    }

    fn contains_prefix_at(
        db: &dyn BinaryTrieDB,
        node_ref: &mut NodeRef,
        bits: &[u8],
        depth: usize,
    ) -> Result<bool, BinaryTrieError> {
        let path = BitPath::from_bits(&bits[..depth]);
        match Self::locate(Self::resolve(db, node_ref, &path)?, bits, depth) {
            Span::Outside => Ok(false),
            Span::Covered => Ok(true),
            Span::Below { split, bit } => {
                let Node::Branch { left, right, .. } = Self::resolve(db, node_ref, &path)? else {
                    unreachable!("only a branch reaches past itself")
                };
                let child = if bit == 0 { left } else { right };
                Self::contains_prefix_at(db, child, bits, split + 1)
            }
        }
    }

    /// Where `node`'s subtree sits relative to `bits`, the prefix a
    /// descent that has already consumed `depth` of those bits is
    /// following.
    ///
    /// The one place the prefix arithmetic lives, so the existence check
    /// and the removal cannot disagree about what a prefix covers.
    fn locate(node: &Node, bits: &[u8], depth: usize) -> Span {
        match node {
            // A leaf carries its whole key, so its own bits settle it
            // outright — no path reasoning needed, and a key shorter
            // than the prefix is simply not under it.
            Node::Leaf { key, .. } => {
                if bits_start_with(key, bits) {
                    Span::Covered
                } else {
                    Span::Outside
                }
            }
            Node::Branch { prefix, .. } => {
                let split = depth + prefix.len();
                if bits.len() <= split {
                    // The prefix runs out at or inside this branch's own
                    // prefix. Every key below shares those bits, so if
                    // what is left of the prefix agrees with them, the
                    // whole subtree is under it.
                    if bits[depth..] == prefix[..bits.len() - depth] {
                        Span::Covered
                    } else {
                        Span::Outside
                    }
                } else if bits[depth..split] != prefix[..] {
                    Span::Outside
                } else {
                    Span::Below {
                        split,
                        bit: bits[split],
                    }
                }
            }
        }
    }

    /// Remove every key beginning with `prefix`, returning how many
    /// leaves went.
    ///
    /// The embedding gathers the state that belongs together under a
    /// common prefix — an account's header stem, or the whole of its
    /// overflow storage — precisely so that clearing it is one
    /// operation. Doing it key by key is not an option for either: the
    /// header stem would be 256 descents, and overflow storage is
    /// unbounded and cannot be enumerated from outside the trie at all.
    ///
    /// Absence is not an error, as it is not for [`BinaryTrie::remove`]:
    /// a prefix nothing lives under removes nothing and returns `0`.
    ///
    /// # Cost
    ///
    /// One descent to the covered node, then a walk of everything below
    /// it — because every node that leaves the tree leaves a path behind
    /// for the next [`BinaryTrie::commit`] to tombstone, and those paths
    /// are only knowable by reading the subtree. That walk is
    /// proportional to what is being deleted, which is the price of
    /// deleting it without leaving the database full of unreachable
    /// nodes.
    ///
    /// # Errors
    ///
    /// The trie is left unchanged on every error, and no path is
    /// tombstoned unless the whole subtree was read successfully:
    /// - [`BinaryTrieError::EmptyKey`] if `prefix` is empty — see
    ///   [`BinaryTrie::contains_prefix`], with the extra force that
    ///   emptying the whole trie by accident is worse than misreading it.
    /// - [`BinaryTrieError::Backend`] or
    ///   [`BinaryTrieError::MalformedNode`] if a node could not be
    ///   loaded.
    pub fn remove_prefix(&mut self, prefix: &KeyPrefix) -> Result<usize, BinaryTrieError> {
        if prefix.is_empty() {
            return Err(BinaryTrieError::EmptyKey);
        }
        let bits = prefix.as_bits();
        let Self {
            db,
            root,
            pending_removal,
        } = self;
        let Some(node_ref) = root else {
            return Ok(0);
        };
        match Self::remove_prefix_at(db.as_ref(), node_ref, bits, 0, pending_removal)? {
            PrefixRemoval::Absent => Ok(0),
            PrefixRemoval::Removed(count) => Ok(count),
            PrefixRemoval::Vanished(count) => {
                // Every key in the trie was under the prefix, and the
                // subtree walk already tombstoned the root's own path
                // along with the rest.
                *root = None;
                Ok(count)
            }
        }
    }

    /// Remove under `bits` from the subtree at `node_ref`, invalidating
    /// it on the way back out if anything below it changed — the same
    /// discipline [`BinaryTrie::remove_at`] follows, and for the same
    /// reason.
    fn remove_prefix_at(
        db: &dyn BinaryTrieDB,
        node_ref: &mut NodeRef,
        bits: &[u8],
        depth: usize,
        pending_removal: &mut BTreeSet<BitPath>,
    ) -> Result<PrefixRemoval, BinaryTrieError> {
        let result = Self::remove_prefix_below(db, node_ref, bits, depth, pending_removal);
        if !matches!(result, Ok(PrefixRemoval::Absent) | Err(_)) {
            node_ref.invalidate();
        }
        result
    }

    /// The prefix removal itself; see [`BinaryTrie::remove_prefix_at`],
    /// which wraps it to invalidate the node on the way back out.
    fn remove_prefix_below(
        db: &dyn BinaryTrieDB,
        node_ref: &mut NodeRef,
        bits: &[u8],
        depth: usize,
        pending_removal: &mut BTreeSet<BitPath>,
    ) -> Result<PrefixRemoval, BinaryTrieError> {
        // The descent got here by matching `bits[..depth]`, so those
        // bits are this node's path — the key it is stored under.
        let path = BitPath::from_bits(&bits[..depth]);
        match Self::locate(Self::resolve(db, node_ref, &path)?, bits, depth) {
            Span::Outside => Ok(PrefixRemoval::Absent),
            Span::Covered => {
                // Read the whole subtree before touching anything: a
                // failure part-way through must not leave the tombstones
                // of nodes that are still in the tree.
                let mut retired = Vec::new();
                let count = Self::collect_subtree(db, node_ref, &path, &mut retired)?;
                pending_removal.extend(retired);
                // The caller unlinks this subtree: only it knows whether
                // there is a sibling to put in its place.
                Ok(PrefixRemoval::Vanished(count))
            }
            Span::Below { split, bit } => {
                let Node::Branch { left, right, .. } = Self::resolve(db, node_ref, &path)? else {
                    unreachable!("only a branch reaches past itself")
                };
                let child = if bit == 0 { left } else { right };
                match Self::remove_prefix_at(db, child, bits, split + 1, pending_removal)? {
                    // Nothing below vanished, so this branch still has
                    // both its children and keeps its shape.
                    other @ (PrefixRemoval::Absent | PrefixRemoval::Removed(_)) => Ok(other),
                    PrefixRemoval::Vanished(count) => {
                        Self::collapse(db, node_ref, &path, bit, pending_removal)?;
                        Ok(PrefixRemoval::Removed(count))
                    }
                }
            }
        }
    }

    /// Push the path of every node in the subtree at `path` onto
    /// `retired`, its own included, and return how many of them are
    /// leaves.
    ///
    /// Loads the subtree to do it, stored references and all: a path is
    /// only knowable from the branch above it, so there is no way to
    /// name the nodes about to be orphaned without reading them.
    ///
    /// Collects into the caller's vector rather than straight into the
    /// pending set so that a failure half-way down leaves nothing
    /// behind — a tombstone for a node still in the tree would delete
    /// live state at the next commit.
    fn collect_subtree(
        db: &dyn BinaryTrieDB,
        node_ref: &mut NodeRef,
        path: &BitPath,
        retired: &mut Vec<BitPath>,
    ) -> Result<usize, BinaryTrieError> {
        let leaves = match Self::resolve(db, node_ref, path)? {
            Node::Leaf { .. } => 1,
            Node::Branch {
                prefix,
                left,
                right,
            } => {
                let (left_path, right_path) = (path.child(prefix, 0), path.child(prefix, 1));
                Self::collect_subtree(db, left, &left_path, retired)?
                    + Self::collect_subtree(db, right, &right_path, retired)?
            }
        };
        retired.push(path.clone());
        Ok(leaves)
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
    /// Removals ride along in the same batch as empty-valued entries —
    /// a tombstone the backend turns into a delete, following the MPT's
    /// `pending_removal` rather than adding a delete method to
    /// [`BinaryTrieDB`]. An empty value cannot be confused for a node:
    /// `decode` rejects empty input outright.
    ///
    /// A path that is being written is never tombstoned. Delete a key
    /// and put it back before committing and the tree ends up the shape
    /// it started, so the very paths the removal retired are the ones
    /// the reinsertion fills — a tombstone for them would erase a live
    /// node.
    ///
    /// # Errors
    ///
    /// [`BinaryTrieError::Backend`] if the write fails. The dirty flags
    /// and the pending removals survive a failed write, so the nodes are
    /// offered again next time.
    pub fn commit(&mut self) -> Result<H256, BinaryTrieError> {
        let mut entries = Vec::new();
        let root = match &self.root {
            None => EMPTY_TRIE_ROOT,
            Some(node_ref) => Self::collect(node_ref, BitPath::new(), &mut entries),
        };
        let written: HashSet<&BitPath> = entries.iter().map(|(path, _)| path).collect();
        let tombstones: Vec<BitPath> = self
            .pending_removal
            .iter()
            .filter(|path| !written.contains(path))
            .cloned()
            .collect();
        drop(written);
        entries.extend(tombstones.into_iter().map(|path| (path, Vec::new())));
        if entries.is_empty() {
            // Nothing was written, so nothing was filtered: every
            // pending removal would have made it into `entries`, and
            // there were none.
            return Ok(root);
        }
        self.db.put_batch(entries)?;
        self.pending_removal.clear();
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

    /// A 34-byte key whose only non-zero byte is the first, set to `i`:
    /// keys that agree on their leading four bits and differ in the
    /// four after them.
    fn wide_key(i: u8) -> Vec<u8> {
        let mut key = vec![0x00; 34];
        key[0] = i;
        key
    }

    /// Sixteen keys differing only in the low nibble of their first
    /// byte: 31 nodes in all, at most 5 of them on any root-to-leaf
    /// path.
    fn wide_entries() -> Vec<(Vec<u8>, [u8; 32])> {
        (0u8..16).map(|i| (wide_key(i), [i; 32])).collect()
    }

    /// Eight keys whose fifth bit is 0 and one whose fifth bit is 1.
    ///
    /// The root branch therefore splits at bit 4 into a 15-node subtree
    /// on the left and the lone key's leaf on the right. Removing that
    /// lone key collapses the root onto a *branch* that is still
    /// nothing but a hash — the shape that makes prefix merging and
    /// path arithmetic observable.
    fn lopsided_entries() -> Vec<(Vec<u8>, [u8; 32])> {
        (0u8..9).map(|i| (wide_key(i), [i; 32])).collect()
    }

    /// The lone right-hand key of [`lopsided_entries`].
    const LOPSIDED_ODD_ONE: u8 = 8;

    /// Path of the leaf holding [`LOPSIDED_ODD_ONE`]: four zero bits of
    /// shared prefix, then the split bit that chose the right side.
    fn lopsided_removed_leaf_path() -> BitPath {
        BitPath::from_bits(&[0, 0, 0, 0, 1])
    }

    /// Path of the surviving left-hand subtree of [`lopsided_entries`],
    /// before the collapse moves it up to the root.
    fn lopsided_survivor_path() -> BitPath {
        BitPath::from_bits(&[0, 0, 0, 0, 0])
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

    /// One step of an insert/delete sequence: a value to insert under
    /// `key`, or `None` to remove it.
    type Step = (Vec<u8>, Option<[u8; 32]>);

    fn ins(i: u8) -> Step {
        (wide_key(i), Some([i; 32]))
    }

    fn del(i: u8) -> Step {
        (wide_key(i), None)
    }

    /// Root reached by running `steps` over a fresh trie.
    fn root_after(steps: &[Step]) -> H256 {
        let mut trie = BinaryTrie::new_temp();
        for (key, value) in steps {
            match value {
                Some(value) => trie.insert(key.clone(), *value).unwrap(),
                None => {
                    trie.remove(key).unwrap();
                }
            }
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
    fn remove_returns_the_value_and_absent_keys_are_none() {
        let mut trie = BinaryTrie::new_temp();
        // Removing from an empty trie is not an error.
        assert_eq!(trie.remove(&wide_key(0)).unwrap(), None);

        trie.insert(wide_key(0), [0; 32]).unwrap();
        trie.insert(wide_key(1), [1; 32]).unwrap();
        trie.insert(wide_key(2), [2; 32]).unwrap();

        assert_eq!(trie.remove(&wide_key(7)).unwrap(), None, "absent key");
        assert_eq!(trie.remove(&wide_key(1)).unwrap(), Some([1; 32]));
        assert_eq!(trie.get(&wide_key(1)).unwrap(), None, "gone after removal");
        assert_eq!(
            trie.remove(&wide_key(1)).unwrap(),
            None,
            "removing twice removes nothing the second time"
        );
        assert_eq!(trie.get(&wide_key(0)).unwrap(), Some([0; 32]));
        assert_eq!(trie.get(&wide_key(2)).unwrap(), Some([2; 32]));
    }

    #[test]
    fn removing_the_only_key_empties_the_trie() {
        let mut trie = BinaryTrie::new_temp();
        trie.insert(vec![0x42; 34], [1; 32]).unwrap();
        assert_ne!(trie.root(), EMPTY_TRIE_ROOT);

        assert_eq!(trie.remove(&[0x42; 34]).unwrap(), Some([1; 32]));
        assert_eq!(trie.root(), EMPTY_TRIE_ROOT);
        assert_eq!(trie.get(&[0x42; 34]).unwrap(), None);
        assert_eq!(trie.commit().unwrap(), EMPTY_TRIE_ROOT);
    }

    #[test]
    fn insert_delete_sequences_converge_on_the_same_root() {
        // Survivor is a branch. Keys 0, 1 and 2 share six leading bits
        // and split at bit 6 into the pair {0, 1} and the singleton
        // {2}, so deleting 2 collapses the root onto the {0, 1} branch.
        // That branch only becomes the canonical root for {0, 1} if it
        // absorbs the six prefix bits and the split bit above it.
        let canonical = root_from_scratch(&[(wide_key(0), [0; 32]), (wide_key(1), [1; 32])]);
        let sequences = [
            vec![ins(0), ins(1), ins(2), del(2)],
            vec![ins(2), ins(1), ins(0), del(2)],
            vec![ins(0), ins(2), del(2), ins(1)],
            // Two collapses in a row: deleting 3 leaves leaf 2 to move
            // up (survivor is a leaf), then deleting 2 collapses the
            // root onto a branch.
            vec![ins(0), ins(1), ins(2), ins(3), del(3), del(2)],
            // Delete and reinsert: the cycle must return the root it
            // started from, tombstones and all.
            vec![ins(0), ins(1), ins(2), del(2), ins(2), del(2)],
        ];
        for (i, steps) in sequences.iter().enumerate() {
            assert_eq!(
                root_after(steps),
                canonical,
                "branch survivor, sequence {i}"
            );
        }

        // Survivor is a leaf: deleting 0 leaves the {0, 1} branch
        // holding only leaf 1, which moves up carrying its whole key.
        let canonical = root_from_scratch(&[(wide_key(1), [1; 32]), (wide_key(2), [2; 32])]);
        let sequences = [
            vec![ins(0), ins(1), ins(2), del(0)],
            vec![ins(2), ins(0), ins(1), del(0)],
            vec![ins(0), ins(1), del(0), ins(2)],
            vec![ins(1), ins(0), ins(2), del(0), ins(0), del(0)],
        ];
        for (i, steps) in sequences.iter().enumerate() {
            assert_eq!(root_after(steps), canonical, "leaf survivor, sequence {i}");
        }

        // A wider set, deleted down to half: eight collapses at varying
        // depths, reached two different ways.
        let evens: Vec<_> = (0u8..16)
            .step_by(2)
            .map(|i| (wide_key(i), [i; 32]))
            .collect();
        let canonical = root_from_scratch(&evens);
        let all_then_odds_out: Vec<Step> = (0u8..16)
            .map(ins)
            .chain((1u8..16).step_by(2).map(del))
            .collect();
        let evens_then_odds_in_and_out: Vec<Step> = (0u8..16)
            .step_by(2)
            .map(ins)
            .chain((1u8..16).step_by(2).map(ins))
            .chain((1u8..16).step_by(2).rev().map(del))
            .collect();
        assert_eq!(root_after(&all_then_odds_out), canonical);
        assert_eq!(root_after(&evens_then_odds_in_and_out), canonical);

        // And over the embedding's real key shapes, including the
        // 66-byte one, which diverges from the rest at the second bit.
        let entries = sample_entries();
        let mut trie = BinaryTrie::new_temp();
        for (key, value) in &entries {
            trie.insert(key.clone(), *value).unwrap();
        }
        let (dropped, kept) = entries.split_at(1);
        assert_eq!(
            trie.remove(&dropped[0].0).unwrap(),
            Some(dropped[0].1),
            "the removed key's value comes back"
        );
        assert_eq!(trie.root(), root_from_scratch(kept));
    }

    #[test]
    fn collapse_preserves_stored_subtrees() {
        let entries = lopsided_entries();
        let (map, root) = commit_entries(&entries);

        let mut trie = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map.clone())), root);
        // The survivor is the root's other child: a whole subtree that
        // has never been loaded. It moves up one level and absorbs the
        // bits the root consumed, which must leave every path below it
        // exactly where the database put it.
        let removed = wide_key(LOPSIDED_ODD_ONE);
        assert_eq!(trie.remove(&removed).unwrap(), Some([LOPSIDED_ODD_ONE; 32]));
        let new_root = trie.commit().unwrap();

        let kept = &entries[..entries.len() - 1];
        let mut reopened = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map)), new_root);
        for (key, value) in kept {
            assert_eq!(
                reopened.get(key).unwrap(),
                Some(*value),
                "key {key:02x?} after the collapse"
            );
        }
        assert_eq!(reopened.get(&removed).unwrap(), None);
        assert_eq!(
            reopened.root(),
            root_from_scratch(kept),
            "the collapsed tree must be the canonical one for what is left"
        );
    }

    #[test]
    fn collapse_writes_are_bounded() {
        let entries = lopsided_entries();
        let (map, root) = commit_entries(&entries);
        let (db, counts) = CountingDB::over(map);

        let mut trie = BinaryTrie::open(Box::new(db), root);
        trie.remove(&wide_key(LOPSIDED_ODD_ONE)).unwrap();
        trie.commit().unwrap();

        // The collapse rewrites one node — the root, now the survivor —
        // and tombstones two paths: the removed leaf's and the
        // survivor's old one. The survivor's 15-node subtree keeps its
        // paths, so none of it is rewritten.
        let bound = 3;
        let written = counts.writes();
        assert!(
            written <= bound,
            "a collapse should write the merged node plus two tombstones \
             = {bound} entries, wrote {written}"
        );
        let node_count = 2 * entries.len() - 1;
        assert!(
            written < node_count,
            "wrote {written} of the tree's {node_count} nodes"
        );
    }

    #[test]
    fn reinserting_before_the_commit_beats_the_tombstones() {
        let entries = lopsided_entries();
        let (map, root) = commit_entries(&entries);

        let mut trie = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map.clone())), root);
        let key = wide_key(LOPSIDED_ODD_ONE);
        // Out and back in before anything is written: the tree returns
        // to the shape it had, so the reinsertion refills the very
        // paths the removal retired. Tombstoning them now would delete
        // live nodes — and the trie would still report the right root,
        // because the root is computed from memory.
        trie.remove(&key).unwrap();
        trie.insert(key.clone(), [0xee; 32]).unwrap();
        let new_root = trie.commit().unwrap();

        let mut reopened = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map)), new_root);
        for (key, value) in &entries[..entries.len() - 1] {
            assert_eq!(reopened.get(key).unwrap(), Some(*value));
        }
        assert_eq!(reopened.get(&key).unwrap(), Some([0xee; 32]));
    }

    #[test]
    fn removed_paths_are_tombstoned() {
        let entries = lopsided_entries();
        let (map, root) = commit_entries(&entries);

        let store = InMemoryBinaryTrieDB::new(map.clone());
        assert!(store.get(&lopsided_removed_leaf_path()).unwrap().is_some());
        assert!(store.get(&lopsided_survivor_path()).unwrap().is_some());

        let mut trie = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map)), root);
        trie.remove(&wide_key(LOPSIDED_ODD_ONE)).unwrap();
        trie.commit().unwrap();

        assert_eq!(
            store.get(&lopsided_removed_leaf_path()).unwrap(),
            None,
            "the removed leaf's path must not still hold a node"
        );
        assert_eq!(
            store.get(&lopsided_survivor_path()).unwrap(),
            None,
            "the survivor moved up, so its old path is garbage"
        );
        assert!(
            store.get(&BitPath::new()).unwrap().is_some(),
            "the survivor's new path is the root's"
        );
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

    /// A set of key/value leaves, in whatever order a test wants them.
    type Entries = Vec<(Vec<u8>, [u8; 32])>;

    /// A fresh, empty in-memory backend, boxed for the constructors.
    fn empty_db() -> Box<dyn BinaryTrieDB> {
        Box::new(InMemoryBinaryTrieDB::new_empty())
    }

    /// `entries` in ascending key order — which, for the prefix-free
    /// keys this trie accepts, is also ascending *bit* order.
    fn sorted(entries: &[(Vec<u8>, [u8; 32])]) -> Entries {
        let mut sorted = entries.to_vec();
        sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
        sorted
    }

    /// `entries` in an order that is neither the input's nor sorted, so
    /// the insertion side of a bulk-versus-insert comparison cannot
    /// accidentally be handed the sorted sequence: every third entry,
    /// then the ones after those, then the ones before.
    fn scrambled(entries: &[(Vec<u8>, [u8; 32])]) -> Entries {
        let mut out: Vec<_> = entries.iter().skip(2).step_by(3).cloned().collect();
        out.extend(entries.iter().skip(1).step_by(3).cloned());
        out.extend(entries.iter().step_by(3).cloned());
        out
    }

    /// 34-byte keys distinguished by their first two bytes, so `n` of
    /// them nest several branches deep while staying prefix-free.
    fn two_byte_entries(n: u16) -> Entries {
        (0..n)
            .map(|i| {
                let mut key = vec![0x00; 34];
                key[0] = (i >> 8) as u8;
                key[1] = i as u8;
                (key, [i as u8; 32])
            })
            .collect()
    }

    /// Named key sets for the bulk-load tests, chosen to exercise the
    /// fold's boundaries: divergence at the very first bit, divergence
    /// only in the last byte after a long shared prefix, the
    /// embedding's mixed 34/66-byte shapes, and enough keys to nest.
    fn bulk_key_sets() -> Vec<(&'static str, Entries)> {
        let first_bit = vec![(vec![0x00; 34], [1u8; 32]), (vec![0x80; 34], [2u8; 32])];
        let deep_shared_prefix: Vec<_> = [0x00u8, 0x01, 0x02, 0x80, 0xff]
            .into_iter()
            .map(|last| {
                let mut key = vec![0x33; 34];
                key[33] = last;
                (key, [last; 32])
            })
            .collect();
        vec![
            ("divergence at the first bit", first_bit),
            ("deep shared prefix", deep_shared_prefix),
            ("wide", wide_entries()),
            ("mixed key lengths", sample_entries()),
            ("two-byte fan-out", two_byte_entries(64)),
        ]
    }

    #[test]
    fn empty_input_builds_an_empty_trie() {
        let mut trie = BinaryTrie::from_sorted_leaves(empty_db(), vec![]).unwrap();
        assert_eq!(trie.root(), EMPTY_TRIE_ROOT);
        assert_eq!(trie.get(&[0xab; 34]).unwrap(), None);
        assert_eq!(trie.commit().unwrap(), EMPTY_TRIE_ROOT);
    }

    #[test]
    fn single_leaf_builds_by_bulk_load() {
        let mut trie =
            BinaryTrie::from_sorted_leaves(empty_db(), vec![(vec![0u8; 34], [0x01; 32])]).unwrap();
        // The same spec vector `single_leaf_matches_spec_vector` pins.
        assert_eq!(
            trie.root().0,
            hex!("4b60a28dce9f3529d103a26e00fadb98514cbd16ce03b7df752426addef9bbc7")
        );
        assert_eq!(trie.get(&[0u8; 34]).unwrap(), Some([0x01; 32]));
    }

    #[test]
    fn bulk_matches_insertion_for_the_same_keys() {
        for (name, entries) in bulk_key_sets() {
            let mut bulk = BinaryTrie::from_sorted_leaves(empty_db(), sorted(&entries)).unwrap();

            // Fed in a different order, to re-confirm that the answer
            // the fold must match is itself order-independent.
            let mut inserted = BinaryTrie::new_temp();
            for (key, value) in scrambled(&entries) {
                inserted.insert(key, value).unwrap();
            }

            assert_ne!(bulk.root(), EMPTY_TRIE_ROOT, "key set {name}");
            assert_eq!(bulk.root(), inserted.root(), "key set {name}");
            for (key, value) in &entries {
                assert_eq!(
                    bulk.get(key).unwrap(),
                    Some(*value),
                    "key set {name}, key {key:02x?}"
                );
            }
        }
    }

    #[test]
    fn rejects_unsorted_input() {
        let descending = vec![(vec![0x80; 34], [1; 32]), (vec![0x00; 34], [2; 32])];
        assert_eq!(
            BinaryTrie::from_sorted_leaves(empty_db(), descending).err(),
            Some(BinaryTrieError::UnsortedInput)
        );
    }

    #[test]
    fn rejects_duplicate_keys() {
        let repeated = vec![(vec![0x42; 34], [1; 32]), (vec![0x42; 34], [2; 32])];
        assert_eq!(
            BinaryTrie::from_sorted_leaves(empty_db(), repeated).err(),
            Some(BinaryTrieError::UnsortedInput)
        );
    }

    #[test]
    fn rejects_prefix_violating_input() {
        // Sorted, distinct, and still impossible: a leaf terminates its
        // path, so no key can lie below another.
        let prefixed = vec![(vec![0xaa], [1; 32]), (vec![0xaa, 0xbb], [2; 32])];
        assert_eq!(
            BinaryTrie::from_sorted_leaves(empty_db(), prefixed).err(),
            Some(BinaryTrieError::PrefixViolation)
        );
        // And the other way round, which is out of order as well; the
        // structural fault is the more specific answer.
        let reversed = vec![(vec![0xaa, 0xbb], [1; 32]), (vec![0xaa], [2; 32])];
        assert_eq!(
            BinaryTrie::from_sorted_leaves(empty_db(), reversed).err(),
            Some(BinaryTrieError::PrefixViolation)
        );
    }

    #[test]
    fn rejects_empty_and_oversized_keys() {
        assert_eq!(
            BinaryTrie::from_sorted_leaves(empty_db(), vec![(vec![], [0; 32])]).err(),
            Some(BinaryTrieError::EmptyKey)
        );
        assert_eq!(
            BinaryTrie::from_sorted_leaves(empty_db(), vec![(vec![0; 8193], [0; 32])]).err(),
            Some(BinaryTrieError::KeyTooLong)
        );
    }

    #[test]
    fn bulk_built_trie_round_trips_through_storage() {
        let entries = sorted(&sample_entries());
        let db = InMemoryBinaryTrieDB::new_empty();
        let map = db.inner();

        let mut trie = BinaryTrie::from_sorted_leaves(Box::new(db), entries.clone()).unwrap();
        let root = trie.commit().unwrap();
        assert_eq!(root, trie.root(), "commit and root must agree");
        assert_eq!(
            root,
            root_from_scratch(&entries),
            "the bulk root must be the canonical one"
        );

        // A fresh handle on the same nodes: every answer below comes
        // from what the commit wrote.
        let mut reopened = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map)), root);
        assert_eq!(reopened.root(), root);
        for (key, value) in &entries {
            assert_eq!(reopened.get(key).unwrap(), Some(*value));
        }
        assert_eq!(reopened.get(&[0x7f; 34]).unwrap(), None);
    }

    // -----------------------------------------------------------------
    // Prefix operations.
    // -----------------------------------------------------------------

    /// Prefix of the `wide_entries` keys whose first byte is below 8:
    /// the four zero bits of the high nibble, then the `8`s bit.
    ///
    /// Deliberately not byte-aligned. This is the shape the embedding
    /// actually asks for — an account's header storage is the sub-index
    /// bytes beginning `01` — and a byte-only prefix could not express
    /// it.
    fn low_half_prefix() -> KeyPrefix {
        KeyPrefix::from_bytes(&[]).and_bits(&[0, 0, 0, 0, 0])
    }

    #[test]
    fn contains_prefix_answers_both_ways() {
        let entries = wide_entries();
        let mut trie = BinaryTrie::new_temp();
        for (key, value) in &entries {
            trie.insert(key.clone(), *value).unwrap();
        }

        // A whole key is a prefix of itself.
        for (key, _) in &entries {
            assert!(trie.contains_prefix(&KeyPrefix::from_bytes(key)).unwrap());
        }
        // One byte of it, which only key 0 begins with.
        assert!(
            trie.contains_prefix(&KeyPrefix::from_bytes(&[0x00]))
                .unwrap()
        );
        // A sub-byte prefix covering a whole subtree, and its complement.
        assert!(trie.contains_prefix(&low_half_prefix()).unwrap());
        assert!(
            trie.contains_prefix(&KeyPrefix::from_bytes(&[]).and_bits(&[0, 0, 0, 0, 1]))
                .unwrap()
        );
        // Nothing lives up here.
        assert!(
            !trie
                .contains_prefix(&KeyPrefix::from_bytes(&[0x80]))
                .unwrap()
        );
        assert!(
            !trie
                .contains_prefix(&KeyPrefix::from_bytes(&[]).and_bits(&[1]))
                .unwrap()
        );
        // Longer than any key: a leaf's key must *begin* with the
        // prefix, so a prefix reaching past it matches nothing.
        assert!(
            !trie
                .contains_prefix(&KeyPrefix::from_bytes(&[0x00; 35]))
                .unwrap()
        );
        // The empty trie has nothing under any prefix.
        assert!(
            !BinaryTrie::new_temp()
                .contains_prefix(&KeyPrefix::from_bytes(&[0x00]))
                .unwrap()
        );
    }

    #[test]
    fn contains_prefix_stops_at_the_covered_node() {
        let entries = wide_entries();
        let (map, root) = commit_entries(&entries);
        let (db, counts) = CountingDB::over(map);

        let mut trie = BinaryTrie::open(Box::new(db), root);
        assert!(trie.contains_prefix(&low_half_prefix()).unwrap());

        // The prefix runs out one bit into the tree: the root branch,
        // then the child whose whole subtree is covered. Answering
        // without walking that subtree is the whole point — a full scan
        // would read all 15 of its nodes.
        let read = counts.reads();
        assert!(
            read <= 2,
            "an existence check should stop at the covered node, read {read}"
        );
    }

    #[test]
    fn prefix_operations_refuse_the_empty_prefix() {
        let mut trie = BinaryTrie::new_temp();
        trie.insert(wide_key(0), [0; 32]).unwrap();
        let empty = KeyPrefix::from_bytes(&[]);
        assert_eq!(trie.contains_prefix(&empty), Err(BinaryTrieError::EmptyKey));
        assert_eq!(trie.remove_prefix(&empty), Err(BinaryTrieError::EmptyKey));
        assert_eq!(
            trie.get(&wide_key(0)).unwrap(),
            Some([0; 32]),
            "the refusal must not have emptied the trie"
        );
    }

    #[test]
    fn remove_prefix_removes_exactly_the_covered_keys() {
        let entries = wide_entries();
        let mut trie = BinaryTrie::new_temp();
        for (key, value) in &entries {
            trie.insert(key.clone(), *value).unwrap();
        }

        assert_eq!(trie.remove_prefix(&low_half_prefix()).unwrap(), 8);

        let kept = &entries[8..];
        for (key, _) in &entries[..8] {
            assert_eq!(trie.get(key).unwrap(), None, "key {key:02x?} survived");
        }
        for (key, value) in kept {
            assert_eq!(trie.get(key).unwrap(), Some(*value));
        }
        assert!(!trie.contains_prefix(&low_half_prefix()).unwrap());
        assert_eq!(
            trie.root(),
            root_from_scratch(kept),
            "what is left must be the canonical tree for it, not merely a \
             self-consistent one"
        );
        // Removing it again finds nothing, and changes nothing.
        assert_eq!(trie.remove_prefix(&low_half_prefix()).unwrap(), 0);
        assert_eq!(trie.root(), root_from_scratch(kept));
    }

    #[test]
    fn remove_prefix_of_a_single_key_matches_remove() {
        let entries = wide_entries();
        let by_prefix = {
            let mut trie = BinaryTrie::new_temp();
            for (key, value) in &entries {
                trie.insert(key.clone(), *value).unwrap();
            }
            assert_eq!(
                trie.remove_prefix(&KeyPrefix::from_bytes(&wide_key(3)))
                    .unwrap(),
                1
            );
            trie.root()
        };
        assert_eq!(
            by_prefix,
            root_after(&[
                ins(0),
                ins(1),
                ins(2),
                ins(3),
                ins(4),
                ins(5),
                ins(6),
                ins(7),
                ins(8),
                ins(9),
                ins(10),
                ins(11),
                ins(12),
                ins(13),
                ins(14),
                ins(15),
                del(3),
            ])
        );
    }

    #[test]
    fn remove_prefix_can_empty_the_trie() {
        let entries = wide_entries();
        let mut trie = BinaryTrie::new_temp();
        for (key, value) in &entries {
            trie.insert(key.clone(), *value).unwrap();
        }
        // Every key begins with a zero high nibble.
        assert_eq!(
            trie.remove_prefix(&KeyPrefix::from_bytes(&[]).and_bits(&[0, 0, 0, 0]))
                .unwrap(),
            entries.len()
        );
        assert_eq!(trie.root(), EMPTY_TRIE_ROOT);
        assert_eq!(trie.commit().unwrap(), EMPTY_TRIE_ROOT);

        // And on a single-leaf trie, where the root is the covered node
        // and has no sibling to take its place.
        let mut trie = BinaryTrie::new_temp();
        trie.insert(vec![0x42; 34], [1; 32]).unwrap();
        assert_eq!(
            trie.remove_prefix(&KeyPrefix::from_bytes(&[0x42])).unwrap(),
            1
        );
        assert_eq!(trie.root(), EMPTY_TRIE_ROOT);
    }

    #[test]
    fn remove_prefix_of_an_absent_prefix_changes_nothing() {
        let entries = wide_entries();
        let mut trie = BinaryTrie::new_temp();
        for (key, value) in &entries {
            trie.insert(key.clone(), *value).unwrap();
        }
        let before = trie.root();
        // Diverges above the tree, inside a branch prefix, and below a
        // leaf respectively.
        for prefix in [
            KeyPrefix::from_bytes(&[0x80]),
            KeyPrefix::from_bytes(&[0x10]),
            KeyPrefix::from_bytes(&[0x00; 35]),
        ] {
            assert_eq!(trie.remove_prefix(&prefix).unwrap(), 0);
            assert_eq!(trie.root(), before);
        }
        assert_eq!(
            BinaryTrie::new_temp()
                .remove_prefix(&KeyPrefix::from_bytes(&[0x00]))
                .unwrap(),
            0,
            "an empty trie has nothing under any prefix"
        );
    }

    /// The bits selecting `two_byte_entries` keys whose second byte is
    /// in `32..64`: a zero first byte, then `001`.
    fn second_byte_upper_prefix() -> KeyPrefix {
        KeyPrefix::from_bytes(&[0x00]).and_bits(&[0, 0, 1])
    }

    #[test]
    fn remove_prefix_survives_a_reopen_with_no_stale_nodes_behind_it() {
        let entries = two_byte_entries(64);
        let (map, root) = commit_entries(&entries);

        let mut trie = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map.clone())), root);
        assert_eq!(trie.remove_prefix(&second_byte_upper_prefix()).unwrap(), 32);
        let new_root = trie.commit().unwrap();

        let kept = &entries[..32];
        assert_eq!(new_root, root_from_scratch(kept));

        // A fresh handle over the same nodes: every answer below comes
        // from what the commit wrote and deleted.
        let mut reopened =
            BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map.clone())), new_root);
        for (key, value) in kept {
            assert_eq!(reopened.get(key).unwrap(), Some(*value), "key {key:02x?}");
        }
        for (key, _) in &entries[32..] {
            assert_eq!(reopened.get(key).unwrap(), None, "key {key:02x?}");
        }
        assert_eq!(reopened.root(), new_root);

        // The removed subtree's nodes are gone from the database, not
        // merely unreachable: a canonical trie over `kept` has
        // `2 * 32 - 1` nodes and the store must hold exactly those.
        assert_eq!(
            map.lock().unwrap().len(),
            2 * kept.len() - 1,
            "the orphaned subtree must not still be on disk"
        );
    }

    #[test]
    fn a_failed_prefix_removal_leaves_the_trie_and_the_tombstones_alone() {
        let entries = wide_entries();
        let (map, root) = commit_entries(&entries);

        // `low_half_prefix` covers the node at `[0; 5]`, whose subtree
        // holds keys 0 through 7: keys 0-3 under `[0; 6]` and keys 4-7
        // under `[0, 0, 0, 0, 0, 1]`. The walk is post-order and goes
        // left first, so taking away the leaf for key 4 — the leftmost
        // of the *right* half — fails it only after it has collected the
        // seven paths of the left half, every one of them a node still
        // very much in the tree. That is the case the two-phase
        // collection exists for; a walk that failed on its first node
        // would not test it at all.
        let victim = BitPath::from_bits(&[0, 0, 0, 0, 0, 1, 0, 0]).to_db_key();
        assert!(map.lock().unwrap().remove(&victim).is_some());
        let before = map.lock().unwrap().len();

        let mut trie = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(map.clone())), root);
        assert!(matches!(
            trie.remove_prefix(&low_half_prefix()),
            Err(BinaryTrieError::Backend(_))
        ));
        assert_eq!(trie.root(), root, "a failed removal must not move the root");

        // And the commit that follows must not tombstone anything: the
        // paths the failed walk collected belong to live nodes.
        trie.commit().unwrap();
        assert_eq!(
            map.lock().unwrap().len(),
            before,
            "a failed removal must leave no tombstones behind"
        );
    }

    #[test]
    fn bulk_load_writes_exactly_the_canonical_node_count() {
        // A canonical binary radix trie over `n` distinct keys has `n`
        // leaves and `n - 1` branches: a branch exists exactly where
        // two subtrees diverge, and `n` leaves diverge `n - 1` times.
        // A fold that is off by a bit somewhere can still produce a
        // well-formed trie, but not one of the right size.
        let entries = two_byte_entries(1000);
        let (db, counts) = CountingDB::over(InMemoryBinaryTrieDB::new_empty().inner());

        let mut trie = BinaryTrie::from_sorted_leaves(Box::new(db), sorted(&entries)).unwrap();
        let root = trie.commit().unwrap();

        assert_eq!(root, root_from_scratch(&entries));
        assert_eq!(
            counts.writes(),
            2 * entries.len() - 1,
            "{} leaves plus {} branches",
            entries.len(),
            entries.len() - 1
        );
        assert_eq!(
            counts.reads(),
            0,
            "bulk construction builds from its input alone"
        );
    }
}
