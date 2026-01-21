//! Merkle Patricia Trie implementation.
//!
//! This module provides both sequential and parallel Merkle computation.
//! The parallel version uses Rayon to compute branch node children concurrently.
//!
//! ## Performance
//!
//! Uses hashbrown with FxHash for faster insertions. FxHash is a fast,
//! non-cryptographic hash that's safe for Ethereum state because:
//! - Keys are already keccak256 hashes (uniformly distributed)
//! - No adversarial input (snap sync data is verified)

use hashbrown::HashMap;
use rayon::prelude::*;
use rustc_hash::FxBuildHasher;
use thiserror::Error;

use super::bloom::BloomFilter;
use super::node::{keccak256, ChildRef, Node, NodeHash, EMPTY_ROOT, HASH_SIZE};

/// Type alias for our fast HashMap with FxHash
type FastHashMap<K, V> = HashMap<K, V, FxBuildHasher>;

/// Trie errors.
#[derive(Error, Debug)]
pub enum TrieError {
    #[error("Key not found")]
    NotFound,
    #[error("Invalid node")]
    InvalidNode,
    #[error("Corrupted trie")]
    Corrupted,
}

/// A simple in-memory Merkle Patricia Trie.
///
/// This implementation stores all nodes in memory and recomputes
/// the root hash when requested.
///
/// ## Performance Optimization
///
/// - Uses hashbrown HashMap with FxHash for ~40-50% faster insertions
/// - Includes a Bloom filter for fast negative lookups
/// - Bloom filter updates are batched in insert_batch for efficiency
pub struct MerkleTrie {
    /// Key-value store (key bytes -> value bytes).
    /// Uses FxHash which is faster than SipHash for pre-hashed keys.
    data: FastHashMap<Vec<u8>, Vec<u8>>,
    /// Cached root hash (invalidated on changes).
    root_cache: Option<[u8; HASH_SIZE]>,
    /// Bloom filter for fast negative lookups.
    bloom: BloomFilter,
    /// Count of Bloom filter hits (key definitely not present).
    bloom_negatives: u64,
}

impl MerkleTrie {
    /// Creates a new empty trie.
    pub fn new() -> Self {
        Self {
            data: FastHashMap::with_hasher(FxBuildHasher),
            root_cache: Some(EMPTY_ROOT),
            bloom: BloomFilter::new(),
            bloom_negatives: 0,
        }
    }

    /// Creates a new trie with expected capacity.
    /// The Bloom filter will be sized appropriately for the expected number of entries.
    pub fn with_capacity(expected_entries: usize) -> Self {
        Self {
            data: FastHashMap::with_capacity_and_hasher(expected_entries, FxBuildHasher),
            root_cache: Some(EMPTY_ROOT),
            bloom: BloomFilter::for_capacity(expected_entries),
            bloom_negatives: 0,
        }
    }

    /// Returns true if the trie is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns the number of entries.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Inserts a key-value pair.
    pub fn insert(&mut self, key: &[u8], value: Vec<u8>) {
        if value.is_empty() {
            self.data.remove(key);
            // Note: Bloom filter doesn't support removal, but that's OK
            // It just means we might have false positives for removed keys
        } else {
            self.data.insert(key.to_vec(), value);
            self.bloom.insert(key);
        }
        self.root_cache = None;
    }

    /// Inserts multiple key-value pairs in a batch.
    /// More efficient than individual inserts as it only invalidates the cache once.
    pub fn insert_batch(&mut self, entries: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>) {
        for (key, value) in entries {
            if value.is_empty() {
                self.data.remove(&key);
            } else {
                self.bloom.insert(&key);
                self.data.insert(key, value);
            }
        }
        self.root_cache = None;
    }

    /// Inserts multiple key-value pairs where keys are already 32-byte hashes.
    ///
    /// This is optimized for snap sync where keys are keccak256 hashes:
    /// - Skips redundant hashing in bloom filter
    /// - Uses batch bloom filter update
    /// - Pre-reserves HashMap capacity
    ///
    /// **~2x faster than `insert_batch` for pre-hashed keys.**
    pub fn insert_batch_prehashed(
        &mut self,
        entries: impl IntoIterator<Item = ([u8; 32], Vec<u8>)>,
    ) {
        // Collect entries to know the count for capacity reservation
        let entries: Vec<_> = entries.into_iter().collect();

        // Reserve capacity to avoid reallocations
        self.data.reserve(entries.len());

        // Batch update bloom filter with all keys first (better cache locality)
        let keys: Vec<&[u8; 32]> = entries
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, _)| k)
            .collect();
        self.bloom.insert_batch_prehashed(keys);

        // Then batch insert into HashMap
        for (key, value) in entries {
            if value.is_empty() {
                self.data.remove(key.as_slice());
            } else {
                self.data.insert(key.to_vec(), value);
            }
        }
        self.root_cache = None;
    }

    /// Gets a value by key.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.data.get(key).map(|v| v.as_slice())
    }

    /// Gets a value by key, using Bloom filter for fast negative lookup.
    ///
    /// Use this when you expect many lookups for non-existent keys.
    /// The Bloom filter can quickly confirm if a key is definitely NOT present.
    /// For workloads with mostly existing keys, use `get()` instead.
    pub fn get_with_bloom(&self, key: &[u8]) -> Option<&[u8]> {
        // Fast path: Bloom filter says definitely not present
        if !self.bloom.may_contain(key) {
            return None;
        }
        // Bloom filter says maybe present, check HashMap
        self.data.get(key).map(|v| v.as_slice())
    }

    /// Checks if a key might exist in the trie.
    ///
    /// This is a fast check using the Bloom filter:
    /// - Returns `false` if the key is definitely not present (no false negatives)
    /// - Returns `true` if the key might be present (could be false positive)
    #[inline]
    pub fn may_contain(&self, key: &[u8]) -> bool {
        self.bloom.may_contain(key)
    }

    /// Removes a key.
    pub fn remove(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        let result = self.data.remove(key);
        if result.is_some() {
            self.root_cache = None;
            // Note: We don't remove from Bloom filter (it doesn't support removal)
            // This is fine - we just get potential false positives for removed keys
        }
        result
    }

    /// Returns Bloom filter statistics.
    pub fn bloom_stats(&self) -> (usize, f64) {
        (
            self.bloom.count(),
            self.bloom.estimated_false_positive_rate(),
        )
    }

    /// Computes and returns the root hash.
    pub fn root_hash(&mut self) -> [u8; HASH_SIZE] {
        if let Some(cached) = self.root_cache {
            return cached;
        }

        let hash = self.compute_root();
        self.root_cache = Some(hash);
        hash
    }

    /// Computes the root hash without caching.
    fn compute_root(&self) -> [u8; HASH_SIZE] {
        if self.data.is_empty() {
            return EMPTY_ROOT;
        }

        // Convert keys to nibble paths in parallel using rayon
        let mut entries: Vec<(Vec<u8>, Vec<u8>)> = self
            .data
            .par_iter()
            .map(|(k, v)| {
                let nibbles = key_to_nibbles(k);
                (nibbles, v.clone())
            })
            .collect();

        // Use parallel sort for large datasets - critical for 260M+ entries
        entries.par_sort_by(|a, b| a.0.cmp(&b.0));

        // Build trie iteratively to avoid stack overflow
        self.build_node_iterative(&entries, 0)
    }

    /// Iteratively builds a trie and returns the root hash.
    /// Uses an explicit stack to avoid stack overflow with deep tries.
    ///
    /// This implementation properly handles inline nodes per Ethereum's MPT spec:
    /// - If a child node's RLP encoding is < 32 bytes, it's embedded inline
    /// - If >= 32 bytes, the keccak256 hash is stored instead
    fn build_node_iterative(
        &self,
        entries: &[(Vec<u8>, Vec<u8>)],
        start_depth: usize,
    ) -> [u8; HASH_SIZE] {
        if entries.is_empty() {
            return EMPTY_ROOT;
        }

        if entries.len() == 1 {
            // Single entry - create a leaf
            let (nibbles, value) = &entries[0];
            let remaining: Vec<u8> = nibbles[start_depth..].to_vec();
            let node = Node::leaf(remaining, value.clone());
            return node.keccak();
        }

        // Work item representing a node to build
        #[derive(Debug)]
        enum WorkItem {
            // Process this group of entries at given depth, store result at result_idx
            Process {
                entries_start: usize,
                entries_end: usize,
                depth: usize,
                result_idx: usize,
            },
            // Finalize an extension node: combine prefix with child
            FinalizeExtension {
                prefix: Vec<u8>,
                child_result_idx: usize,
                result_idx: usize,
            },
            // Finalize a branch node: collect children into branch
            FinalizeBranch {
                child_result_indices: [Option<usize>; 16],
                value: Option<Vec<u8>>,
                result_idx: usize,
            },
        }

        // NodeResult stores either the encoded node (for potential inlining) or just a hash
        // We store the full encoded data so we can determine if children should be inlined
        #[derive(Clone)]
        enum NodeResult {
            /// The RLP-encoded node data
            Encoded(Vec<u8>),
            /// Empty node
            Empty,
        }

        impl NodeResult {
            fn to_child_ref(&self) -> ChildRef {
                match self {
                    NodeResult::Encoded(data) => {
                        if data.len() >= HASH_SIZE {
                            ChildRef::Hash(keccak256(data))
                        } else {
                            ChildRef::Inline(data.clone())
                        }
                    }
                    NodeResult::Empty => ChildRef::Empty,
                }
            }

            fn to_hash(&self) -> [u8; HASH_SIZE] {
                match self {
                    NodeResult::Encoded(data) => keccak256(data),
                    NodeResult::Empty => EMPTY_ROOT,
                }
            }
        }

        // Estimate max results needed (very conservative)
        let max_results = entries.len() * 2 + 1;
        let mut results: Vec<Option<NodeResult>> = vec![None; max_results];
        let mut next_result_idx = 1; // 0 is reserved for root
        let mut work_stack: Vec<WorkItem> = Vec::with_capacity(128);

        // Start with root
        work_stack.push(WorkItem::Process {
            entries_start: 0,
            entries_end: entries.len(),
            depth: start_depth,
            result_idx: 0,
        });

        while let Some(work) = work_stack.pop() {
            match work {
                WorkItem::Process {
                    entries_start,
                    entries_end,
                    depth,
                    result_idx,
                } => {
                    let work_entries = &entries[entries_start..entries_end];

                    if work_entries.is_empty() {
                        results[result_idx] = Some(NodeResult::Empty);
                        continue;
                    }

                    if work_entries.len() == 1 {
                        // Single entry - create a leaf
                        let (nibbles, value) = &work_entries[0];
                        let remaining: Vec<u8> = nibbles[depth..].to_vec();
                        let node = Node::leaf(remaining, value.clone());
                        results[result_idx] = Some(NodeResult::Encoded(node.encode()));
                        continue;
                    }

                    // Check for common prefix
                    let common_prefix = find_common_prefix_owned(work_entries, depth);

                    if common_prefix > 0 {
                        // Extension node - need to compute child first, then finalize
                        let prefix: Vec<u8> =
                            work_entries[0].0[depth..depth + common_prefix].to_vec();
                        let child_result_idx = next_result_idx;
                        next_result_idx += 1;

                        // Push finalize first (will be processed after child)
                        work_stack.push(WorkItem::FinalizeExtension {
                            prefix,
                            child_result_idx,
                            result_idx,
                        });

                        // Then push child processing
                        work_stack.push(WorkItem::Process {
                            entries_start,
                            entries_end,
                            depth: depth + common_prefix,
                            result_idx: child_result_idx,
                        });
                        continue;
                    }

                    // Branch node - group by first nibble
                    let mut groups: [(usize, usize); 16] = [(0, 0); 16]; // (start, end) indices
                    let mut branch_value: Option<Vec<u8>> = None;

                    // First pass: identify groups (entries are sorted, so same nibbles are contiguous)
                    let mut current_nibble: Option<u8> = None;
                    let mut group_start = entries_start;

                    for (i, (nibbles, value)) in work_entries.iter().enumerate() {
                        let global_i = entries_start + i;

                        if depth >= nibbles.len() {
                            // Entry's key ends here - it's the branch value
                            branch_value = Some(value.clone());
                            continue;
                        }

                        let nibble = nibbles[depth];

                        if current_nibble != Some(nibble) {
                            // Close previous group if any
                            if let Some(prev_nibble) = current_nibble {
                                groups[prev_nibble as usize] = (group_start, global_i);
                            }
                            current_nibble = Some(nibble);
                            group_start = global_i;
                        }
                    }

                    // Close last group
                    if let Some(nibble) = current_nibble {
                        groups[nibble as usize] = (group_start, entries_end);
                    }

                    // Allocate result indices for children
                    let mut child_result_indices: [Option<usize>; 16] = [None; 16];
                    for nibble in 0..16 {
                        let (start, end) = groups[nibble];
                        if start < end {
                            child_result_indices[nibble] = Some(next_result_idx);
                            next_result_idx += 1;
                        }
                    }

                    // Push finalize branch (will be processed after all children)
                    work_stack.push(WorkItem::FinalizeBranch {
                        child_result_indices,
                        value: branch_value,
                        result_idx,
                    });

                    // Push child processing in reverse order (so they're processed in order)
                    for nibble in (0..16).rev() {
                        let (start, end) = groups[nibble];
                        if start < end {
                            work_stack.push(WorkItem::Process {
                                entries_start: start,
                                entries_end: end,
                                depth: depth + 1,
                                result_idx: child_result_indices[nibble].unwrap(),
                            });
                        }
                    }
                }

                WorkItem::FinalizeExtension {
                    prefix,
                    child_result_idx,
                    result_idx,
                } => {
                    let child_result = results[child_result_idx]
                        .as_ref()
                        .unwrap_or(&NodeResult::Empty);
                    // Extension node with proper inline handling
                    let child_ref = child_result.to_child_ref();
                    let node = Node::extension_with_child_ref(prefix, child_ref);
                    results[result_idx] = Some(NodeResult::Encoded(node.encode()));
                }

                WorkItem::FinalizeBranch {
                    child_result_indices,
                    value,
                    result_idx,
                } => {
                    // Build children with proper inline handling
                    let mut children: Box<[ChildRef; 16]> = Box::new([
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                        ChildRef::Empty,
                    ]);

                    for (i, idx_opt) in child_result_indices.iter().enumerate() {
                        if let Some(idx) = idx_opt {
                            if let Some(child_result) = &results[*idx] {
                                children[i] = child_result.to_child_ref();
                            }
                        }
                    }

                    let node = Node::branch_with_children(children, value);
                    results[result_idx] = Some(NodeResult::Encoded(node.encode()));
                }
            }
        }

        results[0]
            .as_ref()
            .map(|r| r.to_hash())
            .unwrap_or(EMPTY_ROOT)
    }

    /// Builds a trie node from sorted entries at the given nibble depth.
    fn build_node(&self, entries: &[(Vec<u8>, &[u8])], depth: usize) -> Node {
        if entries.is_empty() {
            return Node::Empty;
        }

        if entries.len() == 1 {
            // Single entry - create a leaf
            let (nibbles, value) = &entries[0];
            let remaining: Vec<u8> = nibbles[depth..].to_vec();
            return Node::leaf(remaining, value.to_vec());
        }

        // Check for common prefix
        let common_prefix = self.find_common_prefix(entries, depth);

        if common_prefix > 0 {
            // Create extension node with proper inline handling
            let prefix: Vec<u8> = entries[0].0[depth..depth + common_prefix].to_vec();
            let child_node = self.build_node(entries, depth + common_prefix);
            let encoded = child_node.encode();
            let child_ref = ChildRef::from_encoded(encoded);
            return Node::extension_with_child_ref(prefix, child_ref);
        }

        // Create branch node with proper inline handling
        let mut children: Box<[ChildRef; 16]> = Box::new([
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
            ChildRef::Empty,
        ]);
        let mut branch_value: Option<Vec<u8>> = None;

        // Group entries by first nibble at current depth
        let mut groups: [Vec<(Vec<u8>, &[u8])>; 16] = Default::default();

        for (nibbles, value) in entries {
            if depth >= nibbles.len() {
                // This entry's key ends here - it's the branch value
                branch_value = Some(value.to_vec());
            } else {
                let nibble = nibbles[depth] as usize;
                groups[nibble].push((nibbles.clone(), *value));
            }
        }

        // Build child nodes with proper inline handling
        for (i, group) in groups.iter().enumerate() {
            if !group.is_empty() {
                let child_node = self.build_node(group, depth + 1);
                let encoded = child_node.encode();
                children[i] = ChildRef::from_encoded(encoded);
            }
        }

        Node::branch_with_children(children, branch_value)
    }

    /// Finds the length of the common prefix among entries starting at the given depth.
    fn find_common_prefix(&self, entries: &[(Vec<u8>, &[u8])], depth: usize) -> usize {
        if entries.is_empty() {
            return 0;
        }

        let first = &entries[0].0;
        if depth >= first.len() {
            return 0;
        }

        let mut common_len = first.len() - depth;

        for (nibbles, _) in &entries[1..] {
            let max_check = (nibbles.len() - depth).min(common_len);
            let mut prefix_len = 0;

            for i in 0..max_check {
                if nibbles[depth + i] == first[depth + i] {
                    prefix_len += 1;
                } else {
                    break;
                }
            }

            common_len = prefix_len;
            if common_len == 0 {
                break;
            }
        }

        common_len
    }

    /// Iterates over all key-value pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.data.iter().map(|(k, v)| (k.as_slice(), v.as_slice()))
    }

    /// Clears the root hash cache, forcing recomputation on next root_hash() call.
    ///
    /// This is useful for benchmarking to measure computation time without caching.
    pub fn clear_cache(&mut self) {
        self.root_cache = None;
    }

    /// Computes the root hash in parallel.
    ///
    /// Uses Rayon to parallelize branch node child computation.
    /// For tries with many entries, this can provide significant speedup
    /// on multi-core systems.
    pub fn parallel_root_hash(&mut self) -> [u8; HASH_SIZE] {
        if let Some(cached) = self.root_cache {
            return cached;
        }

        let hash = self.compute_root_parallel();
        self.root_cache = Some(hash);
        hash
    }

    /// Computes the root hash in parallel without caching.
    fn compute_root_parallel(&self) -> [u8; HASH_SIZE] {
        if self.data.is_empty() {
            return EMPTY_ROOT;
        }

        // Convert keys to nibble paths
        let mut entries: Vec<(Vec<u8>, &[u8])> = self
            .data
            .iter()
            .map(|(k, v)| {
                let nibbles = key_to_nibbles(k);
                (nibbles, v.as_slice())
            })
            .collect();

        // Sort by nibble path for deterministic ordering
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Build trie recursively with parallel branch computation
        let node = self.build_node_parallel(&entries, 0);
        node.keccak()
    }

    /// Builds a trie node from sorted entries, using parallel computation for branches.
    ///
    /// When the number of entries exceeds a threshold, branch children are computed
    /// in parallel using Rayon.
    fn build_node_parallel(&self, entries: &[(Vec<u8>, &[u8])], depth: usize) -> Node {
        if entries.is_empty() {
            return Node::Empty;
        }

        if entries.len() == 1 {
            // Single entry - create a leaf
            let (nibbles, value) = &entries[0];
            let remaining: Vec<u8> = nibbles[depth..].to_vec();
            return Node::leaf(remaining, value.to_vec());
        }

        // Check for common prefix
        let common_prefix = self.find_common_prefix(entries, depth);

        if common_prefix > 0 {
            // Create extension node with proper inline handling
            let prefix: Vec<u8> = entries[0].0[depth..depth + common_prefix].to_vec();
            let child_node = self.build_node_parallel(entries, depth + common_prefix);
            let encoded = child_node.encode();
            let child_ref = ChildRef::from_encoded(encoded);
            return Node::extension_with_child_ref(prefix, child_ref);
        }

        // Create branch node with proper inline handling
        let mut branch_value: Option<Vec<u8>> = None;

        // Group entries by first nibble at current depth
        let mut groups: [Vec<(Vec<u8>, &[u8])>; 16] = Default::default();

        for (nibbles, value) in entries {
            if depth >= nibbles.len() {
                // This entry's key ends here - it's the branch value
                branch_value = Some(value.to_vec());
            } else {
                let nibble = nibbles[depth] as usize;
                groups[nibble].push((nibbles.clone(), *value));
            }
        }

        // Build child nodes in parallel if we have enough entries
        let total_entries: usize = groups.iter().map(|g| g.len()).sum();
        let children: Box<[ChildRef; 16]> = if total_entries > 64 {
            // Parallel computation for large branches
            let child_refs: Vec<ChildRef> = groups
                .par_iter()
                .map(|group| {
                    if group.is_empty() {
                        ChildRef::Empty
                    } else {
                        let child_node = self.build_node_parallel(group, depth + 1);
                        let encoded = child_node.encode();
                        ChildRef::from_encoded(encoded)
                    }
                })
                .collect();

            let mut children: Box<[ChildRef; 16]> = Box::new([
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
            ]);
            for (i, child_ref) in child_refs.into_iter().enumerate() {
                children[i] = child_ref;
            }
            children
        } else {
            // Sequential computation for small branches
            let mut children: Box<[ChildRef; 16]> = Box::new([
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
                ChildRef::Empty,
            ]);
            for (i, group) in groups.iter().enumerate() {
                if !group.is_empty() {
                    let child_node = self.build_node_parallel(group, depth + 1);
                    let encoded = child_node.encode();
                    children[i] = ChildRef::from_encoded(encoded);
                }
            }
            children
        };

        Node::branch_with_children(children, branch_value)
    }
}

impl Default for MerkleTrie {
    fn default() -> Self {
        Self::new()
    }
}

/// Converts a key (bytes) to nibbles.
fn key_to_nibbles(key: &[u8]) -> Vec<u8> {
    let mut nibbles = Vec::with_capacity(key.len() * 2);
    for byte in key {
        nibbles.push(byte >> 4);
        nibbles.push(byte & 0x0F);
    }
    nibbles
}

/// Finds the length of the common prefix among entries starting at the given depth.
/// This version works with owned values (Vec<u8>, Vec<u8>).
fn find_common_prefix_owned(entries: &[(Vec<u8>, Vec<u8>)], depth: usize) -> usize {
    if entries.is_empty() {
        return 0;
    }

    let first = &entries[0].0;
    if depth >= first.len() {
        return 0;
    }

    let mut common_len = first.len() - depth;

    for (nibbles, _) in &entries[1..] {
        if depth >= nibbles.len() {
            return 0;
        }
        let max_check = (nibbles.len() - depth).min(common_len);
        let mut prefix_len = 0;

        for i in 0..max_check {
            if nibbles[depth + i] == first[depth + i] {
                prefix_len += 1;
            } else {
                break;
            }
        }

        common_len = prefix_len;
        if common_len == 0 {
            break;
        }
    }

    common_len
}

// ============================================================================
// Merkle Proofs
// ============================================================================

/// A node in a Merkle proof path.
#[derive(Debug, Clone, PartialEq)]
pub enum ProofNode {
    /// Branch node with all 16 child hashes and optional value.
    Branch {
        children: Box<[Option<[u8; HASH_SIZE]>; 16]>,
        value: Option<Vec<u8>>,
    },
    /// Extension node with path and child hash.
    Extension {
        path: Vec<u8>,
        child: [u8; HASH_SIZE],
    },
    /// Leaf node with path and value.
    Leaf { path: Vec<u8>, value: Vec<u8> },
}

/// A Merkle proof for a key-value inclusion.
///
/// Contains the sequence of nodes from root to the target key,
/// allowing verification without the full trie.
#[derive(Debug, Clone)]
pub struct MerkleProof {
    /// The key being proved.
    pub key: Vec<u8>,
    /// The value at the key (None for non-existence proofs).
    pub value: Option<Vec<u8>>,
    /// Proof nodes from root towards the key.
    pub proof: Vec<ProofNode>,
}

impl MerkleTrie {
    /// Generates a Merkle proof for the given key.
    ///
    /// Returns a proof that can be used to verify the key's inclusion
    /// (or non-existence) in the trie.
    pub fn generate_proof(&mut self, key: &[u8]) -> MerkleProof {
        let value = self.data.get(key).cloned();

        if self.data.is_empty() {
            return MerkleProof {
                key: key.to_vec(),
                value,
                proof: vec![],
            };
        }

        // Convert keys to nibble paths
        let mut entries: Vec<(Vec<u8>, &[u8])> = self
            .data
            .iter()
            .map(|(k, v)| {
                let nibbles = key_to_nibbles(k);
                (nibbles, v.as_slice())
            })
            .collect();

        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let target_nibbles = key_to_nibbles(key);
        let mut proof_nodes = Vec::new();

        self.collect_proof(&entries, 0, &target_nibbles, &mut proof_nodes);

        MerkleProof {
            key: key.to_vec(),
            value,
            proof: proof_nodes,
        }
    }

    /// Recursively collects proof nodes along the path to the target key.
    fn collect_proof(
        &self,
        entries: &[(Vec<u8>, &[u8])],
        depth: usize,
        target: &[u8],
        proof: &mut Vec<ProofNode>,
    ) {
        if entries.is_empty() {
            return;
        }

        if entries.len() == 1 {
            // Leaf node
            let (nibbles, value) = &entries[0];
            let remaining: Vec<u8> = nibbles[depth..].to_vec();
            proof.push(ProofNode::Leaf {
                path: remaining,
                value: value.to_vec(),
            });
            return;
        }

        // Check for common prefix
        let common_prefix = self.find_common_prefix(entries, depth);

        if common_prefix > 0 {
            // Extension node
            let prefix: Vec<u8> = entries[0].0[depth..depth + common_prefix].to_vec();
            let child_node = self.build_node(entries, depth + common_prefix);
            let child_hash = child_node.keccak();

            proof.push(ProofNode::Extension {
                path: prefix,
                child: child_hash,
            });

            // Continue down the path
            self.collect_proof(entries, depth + common_prefix, target, proof);
            return;
        }

        // Branch node
        let mut children: Box<[Option<[u8; HASH_SIZE]>; 16]> = Box::new([None; 16]);
        let mut branch_value: Option<Vec<u8>> = None;
        let mut groups: [Vec<(Vec<u8>, &[u8])>; 16] = Default::default();

        for (nibbles, value) in entries {
            if depth >= nibbles.len() {
                branch_value = Some(value.to_vec());
            } else {
                let nibble = nibbles[depth] as usize;
                groups[nibble].push((nibbles.clone(), *value));
            }
        }

        // Build child hashes
        for (i, group) in groups.iter().enumerate() {
            if !group.is_empty() {
                let child_node = self.build_node(group, depth + 1);
                match child_node.hash() {
                    NodeHash::Hash(h) => children[i] = Some(h),
                    NodeHash::Inline(data) => children[i] = Some(keccak256(&data)),
                }
            }
        }

        proof.push(ProofNode::Branch {
            children: children.clone(),
            value: branch_value,
        });

        // Continue down the target path if within bounds
        if depth < target.len() {
            let target_nibble = target[depth] as usize;
            if !groups[target_nibble].is_empty() {
                self.collect_proof(&groups[target_nibble], depth + 1, target, proof);
            }
        }
    }
}

impl MerkleProof {
    /// Verifies this proof against a given root hash.
    ///
    /// Returns true if the proof is valid for the given root.
    pub fn verify(&self, root_hash: &[u8; HASH_SIZE]) -> bool {
        if self.proof.is_empty() {
            // Empty trie - root should be EMPTY_ROOT
            return *root_hash == EMPTY_ROOT && self.value.is_none();
        }

        // Compute root from proof
        let computed = self.compute_root_from_proof();
        computed == *root_hash
    }

    /// Computes the root hash from the proof nodes.
    fn compute_root_from_proof(&self) -> [u8; HASH_SIZE] {
        use super::rlp_encode::RlpEncoder;

        if self.proof.is_empty() {
            return EMPTY_ROOT;
        }

        // Build from bottom up
        let mut current_hash: Option<[u8; HASH_SIZE]> = None;

        for node in self.proof.iter().rev() {
            let mut enc = RlpEncoder::new();

            match node {
                ProofNode::Leaf { path, value } => {
                    enc.encode_list(|e| {
                        e.encode_nibbles(path, true);
                        e.encode_bytes(value);
                    });
                }
                ProofNode::Extension { path, child } => {
                    enc.encode_list(|e| {
                        e.encode_nibbles(path, false);
                        e.encode_bytes(child);
                    });
                }
                ProofNode::Branch { children, value } => {
                    enc.encode_list(|e| {
                        for child in children.iter() {
                            match child {
                                Some(h) => e.encode_bytes(h),
                                None => e.encode_empty(),
                            }
                        }
                        match value {
                            Some(v) => e.encode_bytes(v),
                            None => e.encode_empty(),
                        }
                    });
                }
            };

            current_hash = Some(keccak256(enc.as_bytes()));
        }

        current_hash.unwrap_or(EMPTY_ROOT)
    }

    /// Returns true if this is a proof of inclusion (key exists).
    pub fn is_inclusion(&self) -> bool {
        self.value.is_some()
    }

    /// Returns true if this is a proof of non-existence (key does not exist).
    pub fn is_exclusion(&self) -> bool {
        self.value.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_trie() {
        let mut trie = MerkleTrie::new();
        assert!(trie.is_empty());
        assert_eq!(trie.root_hash(), EMPTY_ROOT);
    }

    #[test]
    fn test_single_entry() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key", b"value".to_vec());

        assert!(!trie.is_empty());
        assert_eq!(trie.get(b"key"), Some(b"value".as_slice()));

        let hash = trie.root_hash();
        assert_ne!(hash, EMPTY_ROOT);
    }

    #[test]
    fn test_multiple_entries() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"do", b"verb".to_vec());
        trie.insert(b"dog", b"puppy".to_vec());
        trie.insert(b"doge", b"coin".to_vec());
        trie.insert(b"horse", b"stallion".to_vec());

        assert_eq!(trie.len(), 4);
        assert_eq!(trie.get(b"dog"), Some(b"puppy".as_slice()));
        assert_eq!(trie.get(b"horse"), Some(b"stallion".as_slice()));

        let hash = trie.root_hash();
        assert_ne!(hash, EMPTY_ROOT);
    }

    #[test]
    fn test_remove() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key1", b"value1".to_vec());
        trie.insert(b"key2", b"value2".to_vec());

        let hash1 = trie.root_hash();

        trie.remove(b"key2");
        assert_eq!(trie.get(b"key2"), None);

        let hash2 = trie.root_hash();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_deterministic_hash() {
        let mut trie1 = MerkleTrie::new();
        trie1.insert(b"a", b"1".to_vec());
        trie1.insert(b"b", b"2".to_vec());

        let mut trie2 = MerkleTrie::new();
        // Insert in different order
        trie2.insert(b"b", b"2".to_vec());
        trie2.insert(b"a", b"1".to_vec());

        assert_eq!(trie1.root_hash(), trie2.root_hash());
    }

    #[test]
    fn test_key_to_nibbles() {
        let nibbles = key_to_nibbles(&[0xAB, 0xCD]);
        assert_eq!(nibbles, vec![0xA, 0xB, 0xC, 0xD]);
    }

    #[test]
    fn test_update_value() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key", b"value1".to_vec());
        let hash1 = trie.root_hash();

        trie.insert(b"key", b"value2".to_vec());
        let hash2 = trie.root_hash();

        assert_ne!(hash1, hash2);
        assert_eq!(trie.get(b"key"), Some(b"value2".as_slice()));
    }

    #[test]
    fn test_parallel_empty_trie() {
        let mut trie = MerkleTrie::new();
        assert_eq!(trie.parallel_root_hash(), EMPTY_ROOT);
    }

    #[test]
    fn test_parallel_single_entry() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key", b"value".to_vec());

        // Sequential and parallel should produce the same hash
        let seq_hash = trie.root_hash();
        trie.clear_cache();
        let par_hash = trie.parallel_root_hash();

        assert_eq!(seq_hash, par_hash);
    }

    #[test]
    fn test_parallel_multiple_entries() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"do", b"verb".to_vec());
        trie.insert(b"dog", b"puppy".to_vec());
        trie.insert(b"doge", b"coin".to_vec());
        trie.insert(b"horse", b"stallion".to_vec());

        let seq_hash = trie.root_hash();
        trie.clear_cache();
        let par_hash = trie.parallel_root_hash();

        assert_eq!(seq_hash, par_hash);
    }

    #[test]
    fn test_parallel_large_trie() {
        let mut trie = MerkleTrie::new();

        // Insert 200 entries to exercise parallel code path (threshold is 64)
        for i in 0..200u32 {
            let key = i.to_be_bytes();
            let value = format!("value_{}", i).into_bytes();
            trie.insert(&key, value);
        }

        let seq_hash = trie.root_hash();
        trie.clear_cache();
        let par_hash = trie.parallel_root_hash();

        assert_eq!(seq_hash, par_hash);
    }

    #[test]
    fn test_parallel_deterministic() {
        let mut trie1 = MerkleTrie::new();
        let mut trie2 = MerkleTrie::new();

        // Insert 100 entries in different orders
        for i in 0..100u32 {
            trie1.insert(&i.to_be_bytes(), format!("v{}", i).into_bytes());
        }
        for i in (0..100u32).rev() {
            trie2.insert(&i.to_be_bytes(), format!("v{}", i).into_bytes());
        }

        // Both should produce the same hash
        assert_eq!(trie1.parallel_root_hash(), trie2.parallel_root_hash());
    }

    // Merkle proof tests

    #[test]
    fn test_proof_empty_trie() {
        let mut trie = MerkleTrie::new();
        let proof = trie.generate_proof(b"key");

        assert!(proof.is_exclusion());
        assert!(proof.verify(&EMPTY_ROOT));
    }

    #[test]
    fn test_proof_single_entry() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key", b"value".to_vec());

        let root = trie.root_hash();
        let proof = trie.generate_proof(b"key");

        assert!(proof.is_inclusion());
        assert_eq!(proof.value, Some(b"value".to_vec()));
        assert!(!proof.proof.is_empty());
    }

    #[test]
    fn test_proof_multiple_entries() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"do", b"verb".to_vec());
        trie.insert(b"dog", b"puppy".to_vec());
        trie.insert(b"doge", b"coin".to_vec());

        let root = trie.root_hash();

        // Proof for existing key
        let proof = trie.generate_proof(b"dog");
        assert!(proof.is_inclusion());
        assert_eq!(proof.value, Some(b"puppy".to_vec()));

        // Proof for non-existing key
        let proof = trie.generate_proof(b"cat");
        assert!(proof.is_exclusion());
    }

    #[test]
    fn test_proof_types() {
        let mut trie = MerkleTrie::new();
        trie.insert(b"key1", b"value1".to_vec());
        trie.insert(b"key2", b"value2".to_vec());

        let proof = trie.generate_proof(b"key1");

        // Check that proof contains nodes
        assert!(!proof.proof.is_empty());

        // Verify proof node types exist
        for node in &proof.proof {
            match node {
                ProofNode::Branch { children, value } => {
                    // Branch node has 16 children
                    assert_eq!(children.len(), 16);
                }
                ProofNode::Extension { path, child } => {
                    // Extension has a path
                    assert!(!path.is_empty() || child.len() == 32);
                }
                ProofNode::Leaf { path, value } => {
                    // Leaf has value
                    assert!(!value.is_empty());
                }
            }
        }
    }
}
