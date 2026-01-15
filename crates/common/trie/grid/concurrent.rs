//! Concurrent grid trie for parallel state root computation.
//!
//! This module provides 16-way parallelism by partitioning updates
//! by their first nibble and processing each partition independently.

use ethereum_types::H256;
use rayon::prelude::*;

use crate::{db::TrieDB, error::TrieError, EMPTY_TRIE_HASH};

use super::{hex_patricia_grid::HexPatriciaGrid, NIBBLE_COUNT};

/// Concurrent Patricia grid for parallel state root computation.
///
/// Partitions updates by their first nibble (0-15) and processes
/// each partition in parallel using rayon.
pub struct ConcurrentPatriciaGrid<DB: TrieDB + Clone + Send + Sync> {
    /// Database factory for creating per-shard databases
    db: DB,
}

impl<DB: TrieDB + Clone + Send + Sync + 'static> ConcurrentPatriciaGrid<DB> {
    /// Create a new concurrent grid with the given database.
    pub fn new(db: DB) -> Self {
        Self { db }
    }

    /// Apply sorted updates in parallel across 16 shards.
    ///
    /// # CRITICAL: Updates must be sorted by key!
    ///
    /// The updates are partitioned by their first nibble and each
    /// partition is processed in parallel.
    ///
    /// # Arguments
    /// * `updates` - Iterator of (hashed_key, value) pairs in sorted order.
    ///
    /// # Returns
    /// The computed state root hash.
    pub fn apply_sorted_updates_parallel<I>(&mut self, updates: I) -> Result<H256, TrieError>
    where
        I: Iterator<Item = (H256, Vec<u8>)>,
    {
        // Partition updates by first nibble
        let mut partitions: [Vec<(H256, Vec<u8>)>; NIBBLE_COUNT] =
            std::array::from_fn(|_| Vec::new());

        for (key, value) in updates {
            let first_nibble = (key.as_bytes()[0] >> 4) as usize;
            partitions[first_nibble].push((key, value));
        }

        // Check if we have enough updates to benefit from parallelism
        let non_empty_count = partitions.iter().filter(|p| !p.is_empty()).count();

        if non_empty_count < 2 {
            // Not worth parallelizing, use single-threaded path
            return self.apply_sequential(partitions);
        }

        // Process each partition in parallel
        // Each shard processes keys that all share the same first nibble
        let results: Vec<Result<Option<H256>, TrieError>> = partitions
            .into_par_iter()
            .enumerate()
            .map(|(nibble, partition)| {
                if partition.is_empty() {
                    return Ok(None);
                }

                // Create grid for this shard
                let mut grid = HexPatriciaGrid::new(self.db.clone());

                // Apply updates for this partition
                grid.apply_sorted_updates(partition.into_iter())?;

                // Extract the subtrie hash at the first nibble position
                // Since all keys in this partition share first nibble = `nibble`,
                // the grid's root has a single child at that position
                let subtrie_hash = grid.get_child_hash_at_nibble(nibble as u8)?;

                Ok(subtrie_hash)
            })
            .collect();

        // Collect results and merge
        let mut shard_hashes: [Option<H256>; NIBBLE_COUNT] = [None; NIBBLE_COUNT];
        for (i, result) in results.into_iter().enumerate() {
            shard_hashes[i] = result?;
        }

        // Merge shard results into final root hash
        self.merge_shard_results(shard_hashes)
    }

    /// Apply updates sequentially (fallback for small update sets).
    fn apply_sequential(
        &mut self,
        partitions: [Vec<(H256, Vec<u8>)>; NIBBLE_COUNT],
    ) -> Result<H256, TrieError> {
        // Flatten and re-sort
        let mut all_updates: Vec<(H256, Vec<u8>)> =
            partitions.into_iter().flatten().collect();
        all_updates.sort_by_key(|(k, _)| *k);

        if all_updates.is_empty() {
            return Ok(*EMPTY_TRIE_HASH);
        }

        let mut grid = HexPatriciaGrid::new(self.db.clone());
        grid.apply_sorted_updates(all_updates.into_iter())
    }

    /// Merge 16 shard results into a single root hash.
    ///
    /// This creates a branch node at the root level with children
    /// for each non-empty shard.
    fn merge_shard_results(
        &self,
        shard_hashes: [Option<H256>; NIBBLE_COUNT],
    ) -> Result<H256, TrieError> {
        use crate::nibbles::Nibbles;
        use crate::node::{BranchNode, ExtensionNode, NodeRef};
        use crate::node_hash::NodeHash;

        let mut has_any = false;
        let mut choices = BranchNode::EMPTY_CHOICES;
        let mut single_nibble: Option<usize> = None;

        for (i, hash_opt) in shard_hashes.iter().enumerate() {
            if let Some(hash) = hash_opt {
                if *hash != *EMPTY_TRIE_HASH {
                    choices[i] = NodeRef::Hash(NodeHash::from(*hash));
                    has_any = true;
                    single_nibble = Some(i);
                }
            }
        }

        if !has_any {
            return Ok(*EMPTY_TRIE_HASH);
        }

        // Count non-empty children
        let child_count = choices.iter().filter(|c| c.is_valid()).count();

        if child_count == 0 {
            return Ok(*EMPTY_TRIE_HASH);
        }

        if child_count == 1 {
            // Single child - create extension node with the nibble as prefix
            let nibble = single_nibble.unwrap();
            let child_hash = shard_hashes[nibble].unwrap();
            let prefix = Nibbles::from_hex(vec![nibble as u8]);
            let child_ref = NodeRef::Hash(NodeHash::from(child_hash));
            let ext = ExtensionNode::new(prefix, child_ref);
            return Ok(ext.compute_hash().finalize());
        }

        // Multiple children - create branch node
        let branch = BranchNode::new(choices);
        Ok(branch.compute_hash().finalize())
    }
}

/// Partition updates by first nibble for parallel processing.
#[allow(dead_code)]
pub struct UpdatesByNibble {
    /// Updates for each nibble (0-15), sorted by key within each partition
    partitions: [Vec<(H256, Vec<u8>)>; NIBBLE_COUNT],
}

#[allow(dead_code)]
impl UpdatesByNibble {
    /// Create a new empty partitioner.
    pub fn new() -> Self {
        Self {
            partitions: std::array::from_fn(|_| Vec::new()),
        }
    }

    /// Add an update (automatically routes to correct partition).
    pub fn add(&mut self, key: H256, value: Vec<u8>) {
        let first_nibble = (key.as_bytes()[0] >> 4) as usize;
        self.partitions[first_nibble].push((key, value));
    }

    /// Sort all partitions by key (required for grid algorithm).
    pub fn sort(&mut self) {
        for partition in &mut self.partitions {
            partition.sort_by_key(|(k, _)| *k);
        }
    }

    /// Take updates for a specific nibble.
    pub fn take_partition(&mut self, nibble: usize) -> Vec<(H256, Vec<u8>)> {
        std::mem::take(&mut self.partitions[nibble])
    }

    /// Check if parallelization would be beneficial.
    pub fn should_parallelize(&self) -> bool {
        self.partitions.iter().filter(|p| !p.is_empty()).count() >= 2
    }

    /// Get total number of updates across all partitions.
    pub fn len(&self) -> usize {
        self.partitions.iter().map(|p| p.len()).sum()
    }

    /// Check if there are no updates.
    pub fn is_empty(&self) -> bool {
        self.partitions.iter().all(|p| p.is_empty())
    }

    /// Consume and return all partitions.
    pub fn into_partitions(self) -> [Vec<(H256, Vec<u8>)>; NIBBLE_COUNT] {
        self.partitions
    }
}

impl Default for UpdatesByNibble {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::InMemoryTrieDB;
    use ethrex_crypto::keccak::keccak_hash;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    fn create_test_db() -> InMemoryTrieDB {
        InMemoryTrieDB::new(Arc::new(Mutex::new(BTreeMap::new())))
    }

    #[test]
    fn test_updates_by_nibble_partitioning() {
        let mut updates = UpdatesByNibble::new();

        // Create keys with different first nibbles
        // First nibble comes from byte[0] >> 4
        let key1 = {
            let mut bytes = [0u8; 32];
            bytes[0] = 0x10; // First nibble = 1
            H256::from(bytes)
        };
        let key2 = {
            let mut bytes = [0u8; 32];
            bytes[0] = 0x20; // First nibble = 2
            H256::from(bytes)
        };
        let key3 = {
            let mut bytes = [0u8; 32];
            bytes[0] = 0x11; // First nibble = 1
            H256::from(bytes)
        };

        updates.add(key1, vec![1]);
        updates.add(key2, vec![2]);
        updates.add(key3, vec![3]);

        assert_eq!(updates.len(), 3);
        // Keys are in partitions 1 and 2, so should parallelize
        assert!(updates.should_parallelize());
    }

    #[test]
    fn test_updates_by_nibble_single_partition() {
        let mut updates = UpdatesByNibble::new();

        // All keys with same first nibble
        let key1 = {
            let mut bytes = [0u8; 32];
            bytes[0] = 0x10; // First nibble = 1
            H256::from(bytes)
        };
        let key2 = {
            let mut bytes = [0u8; 32];
            bytes[0] = 0x1F; // First nibble = 1
            H256::from(bytes)
        };

        updates.add(key1, vec![1]);
        updates.add(key2, vec![2]);

        assert_eq!(updates.len(), 2);
        assert!(!updates.should_parallelize());
    }

    #[test]
    fn test_concurrent_matches_sequential_small() {
        let db = create_test_db();

        // Generate test data with keys that spread across multiple nibbles
        let mut updates: Vec<(H256, Vec<u8>)> = (0u32..50)
            .map(|i| {
                let key = i.to_be_bytes();
                let hashed = H256::from_slice(&keccak_hash(&key));
                (hashed, key.to_vec())
            })
            .collect();
        updates.sort_by_key(|(k, _)| *k);

        // Sequential
        let mut seq_grid = HexPatriciaGrid::new(db.clone());
        let seq_root = seq_grid.apply_sorted_updates(updates.clone().into_iter()).unwrap();

        // Concurrent
        let mut conc_grid = ConcurrentPatriciaGrid::new(db);
        let conc_root = conc_grid.apply_sorted_updates_parallel(updates.into_iter()).unwrap();

        assert_eq!(seq_root, conc_root, "Sequential and concurrent roots must match");
    }

    #[test]
    fn test_concurrent_matches_sequential_medium() {
        let db = create_test_db();

        // Generate 500 keys
        let mut updates: Vec<(H256, Vec<u8>)> = (0u32..500)
            .map(|i| {
                let key = i.to_be_bytes();
                let hashed = H256::from_slice(&keccak_hash(&key));
                (hashed, key.to_vec())
            })
            .collect();
        updates.sort_by_key(|(k, _)| *k);

        // Sequential
        let mut seq_grid = HexPatriciaGrid::new(db.clone());
        let seq_root = seq_grid.apply_sorted_updates(updates.clone().into_iter()).unwrap();

        // Concurrent
        let mut conc_grid = ConcurrentPatriciaGrid::new(db);
        let conc_root = conc_grid.apply_sorted_updates_parallel(updates.into_iter()).unwrap();

        assert_eq!(seq_root, conc_root, "Sequential and concurrent roots must match for 500 keys");
    }

    #[test]
    fn test_concurrent_matches_sequential_large() {
        let db = create_test_db();

        // Generate 2000 keys
        let mut updates: Vec<(H256, Vec<u8>)> = (0u32..2000)
            .map(|i| {
                let key = i.to_be_bytes();
                let hashed = H256::from_slice(&keccak_hash(&key));
                (hashed, key.to_vec())
            })
            .collect();
        updates.sort_by_key(|(k, _)| *k);

        // Sequential
        let mut seq_grid = HexPatriciaGrid::new(db.clone());
        let seq_root = seq_grid.apply_sorted_updates(updates.clone().into_iter()).unwrap();

        // Concurrent
        let mut conc_grid = ConcurrentPatriciaGrid::new(db);
        let conc_root = conc_grid.apply_sorted_updates_parallel(updates.into_iter()).unwrap();

        assert_eq!(seq_root, conc_root, "Sequential and concurrent roots must match for 2000 keys");
    }

    #[test]
    fn test_concurrent_single_partition_fallback() {
        let db = create_test_db();

        // Create keys that all fall into the same first nibble partition
        // Keys starting with 0x5X will all have first nibble = 5
        let mut updates: Vec<(H256, Vec<u8>)> = (0u8..20)
            .map(|i| {
                let mut bytes = [0u8; 32];
                bytes[0] = 0x50 | (i & 0x0F); // First nibble always 5
                bytes[1] = i;
                let key = H256::from(bytes);
                (key, vec![i])
            })
            .collect();
        updates.sort_by_key(|(k, _)| *k);

        // Sequential
        let mut seq_grid = HexPatriciaGrid::new(db.clone());
        let seq_root = seq_grid.apply_sorted_updates(updates.clone().into_iter()).unwrap();

        // Concurrent (should fallback to sequential since all in one partition)
        let mut conc_grid = ConcurrentPatriciaGrid::new(db);
        let conc_root = conc_grid.apply_sorted_updates_parallel(updates.into_iter()).unwrap();

        assert_eq!(seq_root, conc_root, "Single partition fallback must match");
    }
}
