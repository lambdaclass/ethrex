mod hash;
#[cfg(test)]
mod tests;
mod update;

use std::collections::HashMap;

use ethereum_types::H256;

use crate::EMPTY_TRIE_HASH;
use crate::error::TrieError;
use crate::nibbles::Nibbles;
use crate::node_hash::NodeHash;

/// Trait for on-demand node loading from the database.
pub trait SparseTrieProvider: Send + Sync {
    fn get_node(&self, path: &Nibbles) -> Result<Option<Vec<u8>>, TrieError>;
}

/// Blanket implementation: any TrieDB automatically works as a SparseTrieProvider.
impl<T: crate::db::TrieDB + ?Sized> SparseTrieProvider for T {
    fn get_node(&self, path: &Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        self.get(path.clone())
    }
}

/// Wrapper to use `&dyn TrieDB` as a `SparseTrieProvider`.
///
/// Rust cannot coerce `&dyn TrieDB` → `&dyn SparseTrieProvider` even with a
/// blanket impl. This wrapper bridges the gap for callers that only have a
/// trait object.
pub struct TrieDBProvider<'a>(pub &'a dyn crate::db::TrieDB);

impl SparseTrieProvider for TrieDBProvider<'_> {
    fn get_node(&self, path: &Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        self.0.get(path.clone())
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
        key: Nibbles,
        hash: Option<NodeHash>,
    },
    /// An extension node storing a shared prefix.
    Extension {
        key: Nibbles,
        hash: Option<NodeHash>,
    },
    /// A branch node with a bitmask of which children exist.
    Branch {
        state_mask: u16,
        hash: Option<NodeHash>,
    },
}

impl SparseNode {
    fn is_hash(&self) -> bool {
        matches!(self, SparseNode::Hash(_))
    }
}

/// Reusable buffers for stack-based hash computation.
#[derive(Default)]
struct SubtrieBuffers {
    rlp_buf: Vec<u8>,
}

/// A subtrie in the SparseTrie, containing nodes indexed by path.
pub struct SparseSubtrie {
    /// Root path of this subtrie (kept for debugging).
    #[allow(dead_code)]
    path: Nibbles,
    /// Path-indexed node storage (path → SparseNode).
    nodes: HashMap<Vec<u8>, SparseNode>,
    /// Leaf full_path → RLP-encoded value (separate from leaf node metadata).
    values: HashMap<Vec<u8>, Vec<u8>>,
    /// Reusable buffers for hash computation.
    buffers: SubtrieBuffers,
}

impl SparseSubtrie {
    fn new(path: Nibbles) -> Self {
        Self {
            path,
            nodes: HashMap::new(),
            values: HashMap::new(),
            buffers: SubtrieBuffers::default(),
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
    modified: Vec<Vec<u8>>,
    /// Whether the set has been sorted (for prefix-based lookup).
    sorted: bool,
}

impl PrefixSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a path as modified.
    pub fn insert(&mut self, path: &Nibbles) {
        self.modified.push(path.as_ref().to_vec());
        self.sorted = false;
    }

    /// Check if any path in the set is a prefix of the given path, or vice versa.
    pub fn contains(&mut self, path: &[u8]) -> bool {
        if !self.sorted {
            self.modified.sort();
            self.modified.dedup();
            self.sorted = true;
        }

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
            self.upper.nodes.insert(Vec::new(), SparseNode::Empty);
            return Ok(());
        }

        // Load root from DB
        let root_rlp = provider.get_node(&Nibbles::default())?.ok_or_else(|| {
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
        update::update_leaf(&mut self.upper, &mut self.lower, full_path, value, provider)
    }

    /// Remove a leaf at the given full path.
    pub fn remove_leaf(
        &mut self,
        full_path: Nibbles,
        provider: &dyn SparseTrieProvider,
    ) -> Result<(), TrieError> {
        self.prefix_set.insert(&full_path);
        update::remove_leaf(&mut self.upper, &mut self.lower, full_path, provider)
    }

    /// Compute the root hash of the trie, using rayon to parallelize
    /// hashing of lower subtries.
    pub fn root(&mut self) -> Result<H256, TrieError> {
        hash::compute_root(&mut self.upper, &mut self.lower, &mut self.prefix_set)
    }

    /// Collect modified nodes as (path, RLP-encoded node) pairs
    /// for persistence to the database.
    ///
    /// Produces two kinds of entries per the old Trie's commit format:
    /// 1. Node entries: `(position_path, RLP-encoded node)` → goes to NODES table
    /// 2. Leaf value entries: `(full_path, raw_value)` → goes to FKV table
    ///
    /// Hash nodes (blinded nodes that were never expanded) are skipped, as they
    /// are already persisted in the database.
    pub fn collect_updates(&self) -> Vec<(Nibbles, Vec<u8>)> {
        let mut updates = Vec::new();

        let collect_subtrie = |subtrie: &SparseSubtrie, updates: &mut Vec<(Nibbles, Vec<u8>)>| {
            for (path_data, node) in &subtrie.nodes {
                if let Some(rlp) =
                    hash::encode_node(node, &subtrie.values, &subtrie.nodes, path_data)
                {
                    updates.push((Nibbles::from_hex(path_data.clone()), rlp));
                }
            }
            for (path_data, value) in &subtrie.values {
                updates.push((Nibbles::from_hex(path_data.clone()), value.clone()));
            }
        };

        // Collect from upper subtrie
        collect_subtrie(&self.upper, &mut updates);

        // Collect from lower subtries
        for lower in &self.lower {
            match lower {
                LowerSubtrie::Revealed(s) | LowerSubtrie::Blind(Some(s)) => {
                    collect_subtrie(s, &mut updates);
                }
                LowerSubtrie::Blind(None) => {}
            }
        }

        updates
    }

    /// Collect node removals — paths that were in the trie but are now gone.
    /// Returns paths with empty Vec<u8> to signal deletion.
    pub fn collect_removals(&self) -> Vec<(Nibbles, Vec<u8>)> {
        // Removals are tracked implicitly: nodes that existed in the DB
        // but are no longer in our node maps need to be removed.
        // For now, the caller handles this via the old approach of tracking
        // pending_removal in the Trie struct. We'll integrate this later.
        Vec::new()
    }
}

impl Default for SparseTrie {
    fn default() -> Self {
        Self::new()
    }
}
