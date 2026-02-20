mod hash;
#[cfg(test)]
mod tests;
mod update;

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use ethereum_types::H256;

use crate::EMPTY_TRIE_HASH;
use crate::error::TrieError;
use crate::nibbles::Nibbles;
use crate::node_hash::NodeHash;

/// Stack-allocated path buffer for trie nibble paths.
/// Ethereum state trie paths are at most 64 nibbles (keccak hash) + 2 for
/// routing, so 66 bytes fits on the stack without heap allocation.
pub(super) type PathVec = SmallVec<[u8; 66]>;

/// Trait for on-demand node loading from the database.
pub trait SparseTrieProvider: Send + Sync {
    fn get_node(&self, path: &[u8]) -> Result<Option<Vec<u8>>, TrieError>;
}

/// Blanket implementation: any TrieDB automatically works as a SparseTrieProvider.
impl<T: crate::db::TrieDB + ?Sized> SparseTrieProvider for T {
    fn get_node(&self, path: &[u8]) -> Result<Option<Vec<u8>>, TrieError> {
        self.get(Nibbles::from_hex(path.to_vec()))
    }
}

/// Wrapper to use `&dyn TrieDB` as a `SparseTrieProvider`.
///
/// Rust cannot coerce `&dyn TrieDB` → `&dyn SparseTrieProvider` even with a
/// blanket impl. This wrapper bridges the gap for callers that only have a
/// trait object.
pub struct TrieDBProvider<'a>(pub &'a dyn crate::db::TrieDB);

impl SparseTrieProvider for TrieDBProvider<'_> {
    fn get_node(&self, path: &[u8]) -> Result<Option<Vec<u8>>, TrieError> {
        self.0.get(Nibbles::from_hex(path.to_vec()))
    }
}

/// A node in the sparse trie, stored by path in a flat HashMap.
#[derive(Debug, Clone)]
pub enum SparseNode {
    /// Empty node (no data).
    Empty,
    /// A blinded node whose contents haven't been loaded from DB yet,
    /// or a propagated hash from a lower subtrie.
    Hash(NodeHash),
    /// A leaf node storing the remaining key suffix.
    Leaf {
        key: PathVec,
        hash: Option<NodeHash>,
    },
    /// An extension node storing a shared prefix.
    Extension {
        key: PathVec,
        hash: Option<NodeHash>,
    },
    /// A branch node with a bitmask of which children exist.
    Branch {
        state_mask: u16,
        hash: Option<NodeHash>,
    },
}

/// Reusable buffers for stack-based hash computation.
#[derive(Default)]
struct SubtrieBuffers {
    rlp_buf: Vec<u8>,
    /// Reusable buffer for building child paths in branch node hashing.
    child_path_buf: PathVec,
    /// Reusable buffer for compact (hex-prefix) encoding.
    compact_buf: Vec<u8>,
}

/// A subtrie in the SparseTrie, containing nodes indexed by path.
pub struct SparseSubtrie {
    /// Root path of this subtrie (kept for debugging).
    #[allow(dead_code)]
    path: Nibbles,
    /// Path-indexed node storage (path → SparseNode).
    nodes: FxHashMap<PathVec, SparseNode>,
    /// Leaf full_path → RLP-encoded value (separate from leaf node metadata).
    values: FxHashMap<PathVec, Vec<u8>>,
    /// Paths of nodes modified since last collect_updates (for dirty-only output).
    dirty_nodes: FxHashSet<PathVec>,
    /// Paths of values modified since last collect_updates (for dirty-only output).
    dirty_values: FxHashSet<PathVec>,
    /// Reusable buffers for hash computation.
    buffers: SubtrieBuffers,
    /// Cached RLP encodings from the last hashing pass.
    /// Avoids double-encoding between `root()` and `collect_updates()`.
    rlp_cache: FxHashMap<PathVec, Vec<u8>>,
}

impl SparseSubtrie {
    fn new(path: Nibbles) -> Self {
        Self {
            path,
            nodes: FxHashMap::default(),
            values: FxHashMap::default(),
            dirty_nodes: FxHashSet::default(),
            dirty_values: FxHashSet::default(),
            buffers: SubtrieBuffers::default(),
            rlp_cache: FxHashMap::default(),
        }
    }

    fn new_empty() -> Self {
        Self::new(Nibbles::default())
    }
}

/// State of a lower subtrie partition.
enum LowerSubtrie {
    /// Not yet loaded from DB. May have a subtrie that was partially loaded.
    Blind(Option<SparseSubtrie>),
    /// Fully revealed and ready for modifications.
    Revealed(SparseSubtrie),
}

/// Tracks which paths have been modified and need hash recomputation.
#[derive(Default)]
pub struct PrefixSet {
    /// Set of modified paths (stored as raw nibble vecs for efficiency).
    modified: Vec<PathVec>,
    /// Whether the set has been sorted (for prefix-based lookup).
    sorted: bool,
}

impl PrefixSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a path as modified.
    pub fn insert(&mut self, path: &Nibbles) {
        self.modified.push(PathVec::from_slice(path.as_ref()));
        self.sorted = false;
    }

    /// Sort the prefix set if not already sorted. Call before parallel reads.
    pub fn ensure_sorted(&mut self) {
        if !self.sorted {
            self.modified.sort();
            self.modified.dedup();
            self.sorted = true;
        }
    }

    /// Check if any path in the set is a prefix of the given path, or vice versa.
    /// Must call `ensure_sorted()` first if the set has been modified.
    pub fn contains(&self, path: &[u8]) -> bool {
        debug_assert!(
            self.sorted,
            "PrefixSet must be sorted before calling contains()"
        );

        // Binary search for a prefix match
        let idx = self.modified.partition_point(|p| p.as_slice() < path);

        // Check if the element at idx starts with path (path is prefix of element)
        if idx < self.modified.len() && self.modified[idx].starts_with(path) {
            return true;
        }

        // Check if path starts with any element before idx
        // We need to check the element just before idx
        if idx > 0 && path.starts_with(&self.modified[idx - 1]) {
            return true;
        }

        false
    }

    pub fn is_empty(&self) -> bool {
        self.modified.is_empty()
    }

    pub fn clear(&mut self) {
        self.modified.clear();
        self.sorted = false;
    }
}

/// Two-tier sparse trie for parallel hash computation.
///
/// The trie is split into an upper subtrie (nodes with path depth < 2 nibbles)
/// and 256 lower subtries partitioned by the first 2 nibbles of the path.
/// This allows parallel hashing of the lower subtries via rayon.
pub struct SparseTrie {
    /// Upper subtrie: nodes with path depth < 2.
    upper: SparseSubtrie,
    /// Lower subtries: partitioned by first 2 nibbles (16 * 16 = 256).
    lower: Vec<LowerSubtrie>,
    /// Tracks which paths need hash recomputation.
    prefix_set: PrefixSet,
    /// Tracks full paths of leaves that have been removed.
    /// These produce `(path, vec![])` deletion markers in the layer cache,
    /// preventing stale reads via the FKV shortcut in `Trie::get()`.
    removed_leaves: FxHashSet<PathVec>,
}

impl SparseTrie {
    /// Create a new empty SparseTrie.
    pub fn new() -> Self {
        let mut lower = Vec::with_capacity(256);
        for _ in 0..256 {
            lower.push(LowerSubtrie::Blind(None));
        }
        Self {
            upper: SparseSubtrie::new_empty(),
            lower,
            prefix_set: PrefixSet::new(),
            removed_leaves: FxHashSet::default(),
        }
    }

    /// Reveal a node at the given path by decoding its RLP bytes and storing
    /// the decoded SparseNode in the appropriate subtrie.
    pub fn reveal_node(&mut self, path: Nibbles, rlp: &[u8]) -> Result<(), TrieError> {
        update::reveal_node_into(&mut self.upper, &mut self.lower, path, rlp)
    }

    /// Reveal the root node from the database.
    pub fn reveal_root(
        &mut self,
        root_hash: H256,
        provider: &dyn SparseTrieProvider,
    ) -> Result<(), TrieError> {
        if root_hash == *EMPTY_TRIE_HASH {
            // Empty trie, insert an empty node at root
            self.upper.nodes.insert(PathVec::new(), SparseNode::Empty);
            return Ok(());
        }

        // Load root from DB
        let root_rlp = provider.get_node(&[])?.ok_or_else(|| {
            TrieError::InconsistentTree(Box::new(
                crate::error::InconsistentTreeError::RootNotFound(root_hash),
            ))
        })?;

        self.reveal_node(Nibbles::default(), &root_rlp)?;
        Ok(())
    }

    /// Update or insert a leaf value at the given full path.
    pub fn update_leaf(
        &mut self,
        full_path: Nibbles,
        value: Vec<u8>,
        provider: &dyn SparseTrieProvider,
    ) -> Result<(), TrieError> {
        self.prefix_set.insert(&full_path);
        // Cancel any prior removal for this path (handles remove-then-reinsert)
        self.removed_leaves.remove(full_path.as_ref());
        update::update_leaf(&mut self.upper, &mut self.lower, full_path, value, provider)
    }

    /// Remove a leaf at the given full path.
    pub fn remove_leaf(
        &mut self,
        full_path: Nibbles,
        provider: &dyn SparseTrieProvider,
    ) -> Result<(), TrieError> {
        self.prefix_set.insert(&full_path);
        // Track removal so collect_updates produces a deletion marker.
        // This prevents stale reads via the FKV shortcut in Trie::get().
        self.removed_leaves
            .insert(PathVec::from_slice(full_path.as_ref()));
        update::remove_leaf(&mut self.upper, &mut self.lower, full_path, provider)
    }

    /// Compute the root hash of the trie, using rayon to parallelize
    /// hashing of lower subtries.
    pub fn root(&mut self) -> Result<H256, TrieError> {
        let result = hash::compute_root(&mut self.upper, &mut self.lower);
        self.prefix_set.clear();
        result
    }

    /// Compute the root hash without internal rayon parallelism.
    /// Use when this trie is already inside a parallel context (e.g., storage
    /// tries being hashed concurrently) to avoid nested rayon overhead.
    pub fn root_sequential(&mut self) -> Result<H256, TrieError> {
        let result = hash::compute_root_sequential(&mut self.upper, &mut self.lower);
        self.prefix_set.clear();
        result
    }

    /// Collect modified nodes as (path, RLP-encoded node) pairs
    /// for persistence to the database.
    ///
    /// Produces three kinds of entries:
    /// 1. Node entries: `(position_path, RLP-encoded node)` → goes to NODES table
    /// 2. Leaf value entries: `(full_path, raw_value)` → goes to FKV table
    /// 3. Deletion entries: `(full_path, vec![])` → marks removed leaves in FKV
    ///
    /// Hash nodes (blinded nodes that were never expanded) are skipped, as they
    /// are already persisted in the database.
    ///
    /// Deletion entries are critical for the layer cache: without them,
    /// `Trie::get()` would find stale values from prior blocks via the
    /// FKV shortcut.
    pub fn collect_updates(&mut self) -> Vec<(Nibbles, Vec<u8>)> {
        let mut updates = Vec::new();

        // Helper: drain dirty sets and move values out to avoid cloning.
        let mut collect_subtrie = |subtrie: &mut SparseSubtrie| {
            // Drain dirty_nodes so we can call rlp_cache.remove() without borrow conflict.
            let dirty_nodes: Vec<PathVec> = subtrie.dirty_nodes.drain().collect();
            for path_data in dirty_nodes {
                // Try cached RLP first (move out), fall back to encode_node
                if let Some(rlp) = subtrie.rlp_cache.remove(path_data.as_slice()) {
                    updates.push((Nibbles::from_hex(path_data.to_vec()), rlp));
                } else if let Some(node) = subtrie.nodes.get(path_data.as_slice())
                    && let Some(rlp) =
                        hash::encode_node(node, &subtrie.values, &subtrie.nodes, &path_data)
                {
                    updates.push((Nibbles::from_hex(path_data.to_vec()), rlp));
                }
            }
            // Drain dirty_values and move values out to avoid cloning.
            let dirty_values: Vec<PathVec> = subtrie.dirty_values.drain().collect();
            for path_data in dirty_values {
                if let Some(value) = subtrie.values.remove(path_data.as_slice()) {
                    updates.push((Nibbles::from_hex(path_data.to_vec()), value));
                }
            }
        };

        // Collect from upper subtrie
        collect_subtrie(&mut self.upper);

        // Collect from lower subtries
        for lower in &mut self.lower {
            match lower {
                LowerSubtrie::Revealed(s) | LowerSubtrie::Blind(Some(s)) => {
                    collect_subtrie(s);
                }
                LowerSubtrie::Blind(None) => {}
            }
        }

        // Append deletion markers for removed leaves.
        // These correspond to the old Trie's `pending_removal` entries.
        for path in self.removed_leaves.drain() {
            updates.push((Nibbles::from_hex(path.to_vec()), vec![]));
        }

        updates
    }
}

impl Default for SparseTrie {
    fn default() -> Self {
        Self::new()
    }
}
