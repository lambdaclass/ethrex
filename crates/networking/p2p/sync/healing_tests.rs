//! Comprehensive tests for healing optimizations
//!
//! Tests cover:
//! - HealingCache functionality and edge cases
//! - Batch operations correctness
//! - node_missing_children_optimized behavior
//! - Concurrent access patterns
//! - Memory bounds and eviction

#[cfg(test)]
mod cache_tests {
    use crate::sync::healing_cache::{HealingCache, PathStatus};
    use ethrex_trie::Nibbles;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_empty_cache_returns_missing() {
        let cache = HealingCache::new();
        let path = Nibbles::from_hex(vec![1, 2, 3, 4, 5, 6, 7, 8]);

        assert_eq!(cache.check_path(&path), PathStatus::DefinitelyMissing);
    }

    #[test]
    fn test_mark_exists_then_check() {
        let cache = HealingCache::new();
        let path = Nibbles::from_hex(vec![0xa, 0xb, 0xc, 0xd]);

        cache.mark_exists(&path);

        // Should be confirmed after marking
        assert_eq!(cache.check_path(&path), PathStatus::ConfirmedExists);
    }

    #[test]
    fn test_batch_mark_and_check() {
        let cache = HealingCache::new();
        let paths: Vec<Nibbles> = (0i32..1000)
            .map(|i| {
                let bytes = i.to_be_bytes();
                Nibbles::from_bytes(&bytes)
            })
            .collect();

        cache.mark_exists_batch(&paths);

        // All paths should be confirmed
        let statuses = cache.check_paths_batch(&paths);
        for status in statuses {
            assert_eq!(status, PathStatus::ConfirmedExists);
        }
    }

    #[test]
    fn test_different_paths_are_independent() {
        let cache = HealingCache::new();
        let path1 = Nibbles::from_hex(vec![1, 2, 3]);
        let path2 = Nibbles::from_hex(vec![4, 5, 6]);
        let path3 = Nibbles::from_hex(vec![7, 8, 9]);

        cache.mark_exists(&path1);

        assert_eq!(cache.check_path(&path1), PathStatus::ConfirmedExists);
        // path2 and path3 should still be missing (or possibly ProbablyExists due to filter)
        let status2 = cache.check_path(&path2);
        let status3 = cache.check_path(&path3);

        // They should not be ConfirmedExists since we didn't add them to LRU
        assert_ne!(status2, PathStatus::ConfirmedExists);
        assert_ne!(status3, PathStatus::ConfirmedExists);
    }

    #[test]
    fn test_empty_path() {
        let cache = HealingCache::new();
        let empty_path = Nibbles::default();

        assert_eq!(cache.check_path(&empty_path), PathStatus::DefinitelyMissing);

        cache.mark_exists(&empty_path);
        assert_eq!(cache.check_path(&empty_path), PathStatus::ConfirmedExists);
    }

    #[test]
    fn test_long_path() {
        let cache = HealingCache::new();
        // Very long path (64 nibbles = 32 bytes, typical for account paths)
        let long_bytes: Vec<u8> = (0..32).collect();
        let long_path = Nibbles::from_bytes(&long_bytes);

        cache.mark_exists(&long_path);
        assert_eq!(cache.check_path(&long_path), PathStatus::ConfirmedExists);
    }

    #[test]
    fn test_stats_tracking() {
        let cache = HealingCache::new();
        let path1 = Nibbles::from_hex(vec![1]);

        // Initial stats should be zero
        let stats = cache.stats();
        assert_eq!(stats.paths_added, 0);

        // Add a path
        cache.mark_exists(&path1);
        let stats = cache.stats();
        assert_eq!(stats.paths_added, 1);
    }

    #[test]
    fn test_clear_stats() {
        let cache = HealingCache::new();
        let path = Nibbles::from_hex(vec![1, 2, 3]);

        cache.mark_exists(&path);
        let stats = cache.stats();
        assert!(stats.paths_added > 0);

        cache.clear();
        let stats = cache.stats();
        assert_eq!(stats.paths_added, 0);
    }

    #[test]
    fn test_clear_cache() {
        let cache = HealingCache::new();
        let paths: Vec<Nibbles> = (0..100)
            .map(|i| Nibbles::from_hex(vec![i as u8]))
            .collect();

        cache.mark_exists_batch(&paths);

        // Verify paths exist
        assert_eq!(cache.check_path(&paths[0]), PathStatus::ConfirmedExists);

        // Clear cache
        cache.clear();

        // Paths should no longer be confirmed (LRU cleared)
        // Note: filter may still return ProbablyExists due to how clear works
        let status = cache.check_path(&paths[0]);
        assert_ne!(status, PathStatus::ConfirmedExists);
    }

    #[test]
    fn test_concurrent_reads() {
        let cache = Arc::new(HealingCache::new());
        let paths: Vec<Nibbles> = (0i32..1000)
            .map(|i| {
                let bytes = i.to_be_bytes();
                Nibbles::from_bytes(&bytes)
            })
            .collect();

        // Pre-populate
        cache.mark_exists_batch(&paths);

        // Spawn multiple reader threads
        let mut handles = vec![];
        for _ in 0..4 {
            let cache_clone = cache.clone();
            let paths_clone = paths.clone();
            let handle = thread::spawn(move || {
                for path in &paths_clone {
                    let status = cache_clone.check_path(path);
                    assert_eq!(status, PathStatus::ConfirmedExists);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }

    #[test]
    fn test_concurrent_writes_and_reads() {
        let cache = Arc::new(HealingCache::new());

        let mut handles = vec![];

        // Writer threads
        for t in 0..2 {
            let cache_clone = cache.clone();
            let handle = thread::spawn(move || {
                for i in 0..500 {
                    let path = Nibbles::from_hex(vec![t as u8, (i % 256) as u8]);
                    cache_clone.mark_exists(&path);
                }
            });
            handles.push(handle);
        }

        // Reader threads
        for t in 0..2 {
            let cache_clone = cache.clone();
            let handle = thread::spawn(move || {
                for i in 0..500 {
                    let path = Nibbles::from_hex(vec![t as u8, (i % 256) as u8]);
                    let _ = cache_clone.check_path(&path);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }

    #[test]
    fn test_lru_eviction() {
        // Create cache with small LRU capacity
        let cache = HealingCache::with_capacity(100, 1_000_000);

        // Add more paths than LRU capacity
        let paths: Vec<Nibbles> = (0..200)
            .map(|i| {
                let bytes = (i as u32).to_be_bytes();
                Nibbles::from_bytes(&bytes)
            })
            .collect();

        for path in &paths {
            cache.mark_exists(path);
        }

        // Recent paths should be in LRU (ConfirmedExists)
        // Older paths may have been evicted (ProbablyExists from filter)
        let recent_status = cache.check_path(&paths[199]);
        assert_eq!(recent_status, PathStatus::ConfirmedExists);

        // First path might have been evicted from LRU
        // but should still be in filter (ProbablyExists)
        let old_status = cache.check_path(&paths[0]);
        assert!(matches!(
            old_status,
            PathStatus::ConfirmedExists | PathStatus::ProbablyExists
        ));
    }

    #[test]
    fn test_prefix_paths_are_independent() {
        let cache = HealingCache::new();

        // Path and its prefix
        let prefix = Nibbles::from_hex(vec![1, 2]);
        let full_path = Nibbles::from_hex(vec![1, 2, 3, 4]);

        cache.mark_exists(&prefix);

        assert_eq!(cache.check_path(&prefix), PathStatus::ConfirmedExists);
        // Full path should not be marked as existing just because prefix is
        let full_status = cache.check_path(&full_path);
        assert_ne!(full_status, PathStatus::ConfirmedExists);
    }

    #[test]
    fn test_batch_operations_preserve_order() {
        let cache = HealingCache::new();

        let paths: Vec<Nibbles> = vec![
            Nibbles::from_hex(vec![1]),
            Nibbles::from_hex(vec![2]),
            Nibbles::from_hex(vec![3]),
        ];

        // Mark only the middle one
        cache.mark_exists(&paths[1]);

        let statuses = cache.check_paths_batch(&paths);

        assert_eq!(statuses.len(), 3);
        // First should be missing
        assert_ne!(statuses[0], PathStatus::ConfirmedExists);
        // Second should be confirmed
        assert_eq!(statuses[1], PathStatus::ConfirmedExists);
        // Third should be missing
        assert_ne!(statuses[2], PathStatus::ConfirmedExists);
    }

    #[test]
    fn test_check_empty_batch() {
        let cache = HealingCache::new();
        let empty: Vec<Nibbles> = vec![];

        let statuses = cache.check_paths_batch(&empty);
        assert!(statuses.is_empty());
    }

    #[test]
    fn test_mark_empty_batch() {
        let cache = HealingCache::new();
        let empty: Vec<Nibbles> = vec![];

        // Should not panic
        cache.mark_exists_batch(&empty);

        let stats = cache.stats();
        assert_eq!(stats.paths_added, 0);
    }

    #[test]
    fn test_duplicate_marks() {
        let cache = HealingCache::new();
        let path = Nibbles::from_hex(vec![1, 2, 3]);

        // Mark same path multiple times
        cache.mark_exists(&path);
        cache.mark_exists(&path);
        cache.mark_exists(&path);

        // Should still be confirmed
        assert_eq!(cache.check_path(&path), PathStatus::ConfirmedExists);

        // Stats should count each mark
        let stats = cache.stats();
        assert_eq!(stats.paths_added, 3);
    }

    #[test]
    fn test_nibbles_vs_bytes_consistency() {
        let cache = HealingCache::new();

        // Create path from hex
        let path_hex = Nibbles::from_hex(vec![0x1, 0x2, 0x3, 0x4]);
        // Create path from raw bytes without leaf flag to match from_hex
        let path_raw = Nibbles::from_raw(&[0x12, 0x34], false);

        // Mark one
        cache.mark_exists(&path_hex);

        // Check both - they should be equivalent since both have same nibbles [1, 2, 3, 4]
        assert_eq!(cache.check_path(&path_hex), PathStatus::ConfirmedExists);
        assert_eq!(cache.check_path(&path_raw), PathStatus::ConfirmedExists);
    }
}

#[cfg(test)]
mod trie_db_batch_tests {
    use ethrex_trie::{InMemoryTrieDB, Nibbles, TrieDB};

    #[test]
    fn test_get_batch_empty() {
        let db = InMemoryTrieDB::default();
        let keys: Vec<Nibbles> = vec![];

        let results = db.get_batch(&keys).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_batch_single() {
        let db = InMemoryTrieDB::default();
        let key = Nibbles::from_hex(vec![1, 2, 3]);
        let value = vec![4, 5, 6];

        db.put(key.clone(), value.clone()).unwrap();

        let results = db.get_batch(&[key]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], Some(value));
    }

    #[test]
    fn test_get_batch_multiple() {
        let db = InMemoryTrieDB::default();

        let pairs: Vec<(Nibbles, Vec<u8>)> = (0..10)
            .map(|i| (Nibbles::from_hex(vec![i as u8]), vec![i as u8 * 2]))
            .collect();

        for (key, value) in &pairs {
            db.put(key.clone(), value.clone()).unwrap();
        }

        let keys: Vec<Nibbles> = pairs.iter().map(|(k, _)| k.clone()).collect();
        let results = db.get_batch(&keys).unwrap();

        assert_eq!(results.len(), pairs.len());
        for (i, result) in results.iter().enumerate() {
            assert_eq!(result, &Some(pairs[i].1.clone()));
        }
    }

    #[test]
    fn test_get_batch_with_missing() {
        let db = InMemoryTrieDB::default();

        // Only add some keys
        let key1 = Nibbles::from_hex(vec![1]);
        let key3 = Nibbles::from_hex(vec![3]);
        db.put(key1.clone(), vec![1]).unwrap();
        db.put(key3.clone(), vec![3]).unwrap();

        let key2 = Nibbles::from_hex(vec![2]);
        let keys = vec![key1, key2, key3];

        let results = db.get_batch(&keys).unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], Some(vec![1]));
        assert_eq!(results[1], None); // key2 not added
        assert_eq!(results[2], Some(vec![3]));
    }

    #[test]
    fn test_exists_batch_empty() {
        let db = InMemoryTrieDB::default();
        let keys: Vec<Nibbles> = vec![];

        let results = db.exists_batch(&keys).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_exists_batch_with_values() {
        let db = InMemoryTrieDB::default();

        let key1 = Nibbles::from_hex(vec![1]);
        let key2 = Nibbles::from_hex(vec![2]);
        let key3 = Nibbles::from_hex(vec![3]);

        db.put(key1.clone(), vec![1]).unwrap();
        db.put(key3.clone(), vec![3]).unwrap();

        let keys = vec![key1, key2, key3];
        let results = db.exists_batch(&keys).unwrap();

        assert_eq!(results.len(), 3);
        assert!(results[0]); // key1 exists
        assert!(!results[1]); // key2 doesn't exist
        assert!(results[2]); // key3 exists
    }

    #[test]
    fn test_exists_single() {
        let db = InMemoryTrieDB::default();

        let key = Nibbles::from_hex(vec![1, 2, 3]);
        assert!(!db.exists(key.clone()).unwrap());

        db.put(key.clone(), vec![1]).unwrap();
        assert!(db.exists(key).unwrap());
    }

    #[test]
    fn test_exists_with_empty_value() {
        let db = InMemoryTrieDB::default();

        let key = Nibbles::from_hex(vec![1]);
        db.put(key.clone(), vec![]).unwrap();

        // Empty value should be treated as non-existent
        assert!(!db.exists(key).unwrap());
    }
}

#[cfg(test)]
mod integration_tests {
    use crate::sync::healing_cache::{HealingCache, PathStatus};
    use ethrex_trie::{InMemoryTrieDB, Nibbles, TrieDB};

    #[test]
    fn test_cache_with_real_trie_operations() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();

        // Simulate healing scenario: add some paths to DB
        let paths: Vec<Nibbles> = (0..10)
            .map(|i| Nibbles::from_hex(vec![i as u8, (i + 1) as u8]))
            .collect();

        for path in &paths {
            db.put(path.clone(), vec![1, 2, 3]).unwrap();
        }

        // Now simulate cache-accelerated lookups
        for path in &paths {
            // First check: should be DefinitelyMissing (not in cache)
            let status = cache.check_path(path);
            assert_eq!(status, PathStatus::DefinitelyMissing);

            // Verify in DB
            assert!(db.exists(path.clone()).unwrap());

            // Mark in cache
            cache.mark_exists(path);

            // Second check: should be ConfirmedExists
            assert_eq!(cache.check_path(path), PathStatus::ConfirmedExists);
        }
    }

    #[test]
    fn test_batch_db_lookup_with_cache() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();

        // Add half the paths to DB
        let all_paths: Vec<Nibbles> = (0..20)
            .map(|i| Nibbles::from_hex(vec![i as u8]))
            .collect();

        for path in &all_paths[0..10] {
            db.put(path.clone(), vec![1]).unwrap();
        }

        // Simulate optimized lookup pattern:
        // 1. Check cache first
        // 2. For ProbablyExists/DefinitelyMissing, batch check DB
        // 3. Update cache with results

        let cache_statuses = cache.check_paths_batch(&all_paths);
        let paths_to_check: Vec<Nibbles> = all_paths
            .iter()
            .zip(cache_statuses.iter())
            .filter(|(_, status)| !matches!(status, PathStatus::ConfirmedExists))
            .map(|(path, _)| path.clone())
            .collect();

        // Batch check in DB
        let db_exists = db.exists_batch(&paths_to_check).unwrap();

        // Update cache with confirmed paths
        let confirmed: Vec<Nibbles> = paths_to_check
            .iter()
            .zip(db_exists.iter())
            .filter(|(_, exists)| **exists)
            .map(|(path, _)| path.clone())
            .collect();

        cache.mark_exists_batch(&confirmed);

        // Verify: first 10 should now be ConfirmedExists
        for path in &all_paths[0..10] {
            assert_eq!(cache.check_path(path), PathStatus::ConfirmedExists);
        }
    }
}

#[cfg(test)]
mod stress_tests {
    use crate::sync::healing_cache::HealingCache;
    use ethrex_trie::Nibbles;
    use std::sync::Arc;
    use std::thread;
    use std::time::Instant;

    #[test]
    fn test_high_throughput() {
        let cache = HealingCache::new();
        let num_ops = 100_000;

        let start = Instant::now();

        // Add paths
        for i in 0..num_ops {
            let path = Nibbles::from_bytes(&(i as u64).to_be_bytes());
            cache.mark_exists(&path);
        }

        // Check paths
        for i in 0..num_ops {
            let path = Nibbles::from_bytes(&(i as u64).to_be_bytes());
            let _ = cache.check_path(&path);
        }

        let elapsed = start.elapsed();
        let ops_per_sec = (num_ops * 2) as f64 / elapsed.as_secs_f64();

        println!(
            "High throughput test: {} ops in {:?} ({:.0} ops/sec)",
            num_ops * 2,
            elapsed,
            ops_per_sec
        );

        // Should be able to do at least 100k ops/sec
        assert!(
            ops_per_sec > 100_000.0,
            "Throughput too low: {} ops/sec",
            ops_per_sec
        );
    }

    #[test]
    fn test_concurrent_stress() {
        let cache = Arc::new(HealingCache::new());
        let num_threads = 8;
        let ops_per_thread = 10_000;

        let start = Instant::now();
        let mut handles = vec![];

        for t in 0..num_threads {
            let cache_clone = cache.clone();
            let handle = thread::spawn(move || {
                for i in 0..ops_per_thread {
                    let path =
                        Nibbles::from_bytes(&((t * ops_per_thread + i) as u64).to_be_bytes());

                    // Alternate between mark and check
                    if i % 2 == 0 {
                        cache_clone.mark_exists(&path);
                    } else {
                        let _ = cache_clone.check_path(&path);
                    }
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().expect("Thread panicked");
        }

        let elapsed = start.elapsed();
        let total_ops = num_threads * ops_per_thread;
        let ops_per_sec = total_ops as f64 / elapsed.as_secs_f64();

        println!(
            "Concurrent stress test: {} ops across {} threads in {:?} ({:.0} ops/sec)",
            total_ops, num_threads, elapsed, ops_per_sec
        );

        // Concurrent ops should still be reasonably fast
        assert!(
            ops_per_sec > 50_000.0,
            "Concurrent throughput too low: {} ops/sec",
            ops_per_sec
        );
    }

    #[test]
    fn test_memory_stability() {
        let cache = HealingCache::new();

        // Add many paths in batches, checking that cache doesn't grow unbounded
        for batch in 0..100 {
            let paths: Vec<Nibbles> = (0..10_000)
                .map(|i| {
                    Nibbles::from_bytes(&((batch * 10_000 + i) as u64).to_be_bytes())
                })
                .collect();

            cache.mark_exists_batch(&paths);

            // LRU should keep cache bounded
            let fill_ratio = cache.lru_fill_ratio();
            assert!(
                fill_ratio <= 1.0,
                "LRU fill ratio exceeded 100%: {}",
                fill_ratio
            );
        }
    }
}

#[cfg(test)]
mod end_to_end_tests {
    use crate::sync::healing_cache::{HealingCache, PathStatus};
    use crate::sync::state_healing::node_missing_children_optimized;
    use ethrex_trie::{InMemoryTrieDB, Nibbles, Node, NodeRef, NodeHash, TrieDB};
    use ethrex_trie::node::{BranchNode, ExtensionNode, LeafNode};
    use ethrex_common::H256;

    fn create_test_branch_with_children(child_indices: &[u8]) -> Node {
        let mut choices: [NodeRef; 16] = Default::default();
        for &idx in child_indices {
            if idx < 16 {
                let hash = NodeHash::Hashed(H256::from_low_u64_be(idx as u64 + 100));
                choices[idx as usize] = NodeRef::Hash(hash);
            }
        }
        Node::Branch(Box::new(BranchNode {
            choices,
            value: vec![],
        }))
    }

    fn create_test_extension(prefix: Vec<u8>, child_hash: H256) -> Node {
        Node::Extension(ExtensionNode {
            prefix: Nibbles::from_hex(prefix),
            child: NodeRef::Hash(NodeHash::Hashed(child_hash)),
        })
    }

    fn create_test_leaf(partial: Vec<u8>, value: Vec<u8>) -> Node {
        Node::Leaf(LeafNode {
            partial: Nibbles::from_hex(partial),
            value,
        })
    }

    #[test]
    fn test_e2e_branch_node_all_children_missing() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let path = Nibbles::from_hex(vec![1, 2]);

        // Create branch with children at indices 0, 5, 10, 15
        let branch = create_test_branch_with_children(&[0, 5, 10, 15]);

        let (missing_count, missing_requests) =
            node_missing_children_optimized(&branch, &path, &db, &cache).unwrap();

        // All 4 children should be missing
        assert_eq!(missing_count, 4);
        assert_eq!(missing_requests.len(), 4);

        // Verify request paths are correct
        let expected_paths: Vec<Nibbles> = vec![
            Nibbles::from_hex(vec![1, 2, 0]),
            Nibbles::from_hex(vec![1, 2, 5]),
            Nibbles::from_hex(vec![1, 2, 10]),
            Nibbles::from_hex(vec![1, 2, 15]),
        ];

        for (req, expected) in missing_requests.iter().zip(expected_paths.iter()) {
            assert_eq!(&req.path, expected);
            assert_eq!(&req.parent_path, &path);
        }
    }

    #[test]
    fn test_e2e_branch_node_some_children_in_db() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let path = Nibbles::from_hex(vec![1, 2]);

        // Create branch with children at indices 0, 5, 10
        let branch = create_test_branch_with_children(&[0, 5, 10]);

        // Add child at index 5 to the DB
        let child_path = Nibbles::from_hex(vec![1, 2, 5]);
        db.put(child_path, vec![1, 2, 3]).unwrap();

        let (missing_count, missing_requests) =
            node_missing_children_optimized(&branch, &path, &db, &cache).unwrap();

        // Only 2 children should be missing (0 and 10)
        assert_eq!(missing_count, 2);
        assert_eq!(missing_requests.len(), 2);
    }

    #[test]
    fn test_e2e_branch_node_all_children_in_cache() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let path = Nibbles::from_hex(vec![1, 2]);

        // Create branch with children at indices 0, 5
        let branch = create_test_branch_with_children(&[0, 5]);

        // Add children to cache
        let child0 = Nibbles::from_hex(vec![1, 2, 0]);
        let child5 = Nibbles::from_hex(vec![1, 2, 5]);
        cache.mark_exists(&child0);
        cache.mark_exists(&child5);

        let (missing_count, missing_requests) =
            node_missing_children_optimized(&branch, &path, &db, &cache).unwrap();

        // No children should be missing (all in cache)
        assert_eq!(missing_count, 0);
        assert_eq!(missing_requests.len(), 0);
    }

    #[test]
    fn test_e2e_extension_node_child_missing() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let path = Nibbles::from_hex(vec![1]);

        let child_hash = H256::from_low_u64_be(12345);
        let extension = create_test_extension(vec![2, 3, 4], child_hash);

        let (missing_count, missing_requests) =
            node_missing_children_optimized(&extension, &path, &db, &cache).unwrap();

        assert_eq!(missing_count, 1);
        assert_eq!(missing_requests.len(), 1);

        // Child path should be parent + prefix
        let expected_child_path = Nibbles::from_hex(vec![1, 2, 3, 4]);
        assert_eq!(missing_requests[0].path, expected_child_path);
        assert_eq!(missing_requests[0].hash, child_hash);
    }

    #[test]
    fn test_e2e_extension_node_child_in_db() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let path = Nibbles::from_hex(vec![1]);

        let child_hash = H256::from_low_u64_be(12345);
        let extension = create_test_extension(vec![2, 3, 4], child_hash);

        // Add child to DB
        let child_path = Nibbles::from_hex(vec![1, 2, 3, 4]);
        db.put(child_path, vec![1, 2, 3]).unwrap();

        let (missing_count, missing_requests) =
            node_missing_children_optimized(&extension, &path, &db, &cache).unwrap();

        assert_eq!(missing_count, 0);
        assert_eq!(missing_requests.len(), 0);
    }

    #[test]
    fn test_e2e_leaf_node_no_children() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let path = Nibbles::from_hex(vec![1, 2, 3]);

        let leaf = create_test_leaf(vec![4, 5, 6], vec![0xde, 0xad, 0xbe, 0xef]);

        let (missing_count, missing_requests) =
            node_missing_children_optimized(&leaf, &path, &db, &cache).unwrap();

        // Leaf nodes have no children
        assert_eq!(missing_count, 0);
        assert_eq!(missing_requests.len(), 0);
    }

    #[test]
    fn test_e2e_cache_updated_after_db_check() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let path = Nibbles::from_hex(vec![1]);

        // Create branch with child at index 0
        let branch = create_test_branch_with_children(&[0]);

        // Add child to DB
        let child_path = Nibbles::from_hex(vec![1, 0]);
        db.put(child_path.clone(), vec![1, 2, 3]).unwrap();

        // First call - child should be found in DB and added to cache
        let (missing_count, _) =
            node_missing_children_optimized(&branch, &path, &db, &cache).unwrap();
        assert_eq!(missing_count, 0);

        // Verify cache was updated
        assert_eq!(cache.check_path(&child_path), PathStatus::ConfirmedExists);
    }

    #[test]
    fn test_e2e_empty_branch_node() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let path = Nibbles::from_hex(vec![1, 2]);

        // Branch with no valid children
        let branch = create_test_branch_with_children(&[]);

        let (missing_count, missing_requests) =
            node_missing_children_optimized(&branch, &path, &db, &cache).unwrap();

        assert_eq!(missing_count, 0);
        assert_eq!(missing_requests.len(), 0);
    }

    #[test]
    fn test_e2e_full_branch_node() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let path = Nibbles::from_hex(vec![1]);

        // Branch with all 16 children
        let branch = create_test_branch_with_children(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

        let (missing_count, missing_requests) =
            node_missing_children_optimized(&branch, &path, &db, &cache).unwrap();

        assert_eq!(missing_count, 16);
        assert_eq!(missing_requests.len(), 16);

        // Verify all paths are unique and correctly formed
        for req in missing_requests.iter() {
            assert_eq!(req.path.len(), 2); // parent (1 nibble) + child index (1 nibble)
        }
    }

    #[test]
    fn test_e2e_deep_path() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();

        // Very deep path (simulating deep trie)
        let deep_path = Nibbles::from_hex((0..60).collect::<Vec<u8>>());

        let branch = create_test_branch_with_children(&[5]);

        let (missing_count, missing_requests) =
            node_missing_children_optimized(&branch, &deep_path, &db, &cache).unwrap();

        assert_eq!(missing_count, 1);
        assert_eq!(missing_requests[0].path.len(), 61);
        assert_eq!(&missing_requests[0].parent_path, &deep_path);
    }

    #[test]
    fn test_e2e_mixed_cache_and_db_hits() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let path = Nibbles::from_hex(vec![1]);

        // Branch with children at 0, 1, 2, 3
        let branch = create_test_branch_with_children(&[0, 1, 2, 3]);

        // Child 0: in cache only
        let child0 = Nibbles::from_hex(vec![1, 0]);
        cache.mark_exists(&child0);

        // Child 1: in DB only
        let child1 = Nibbles::from_hex(vec![1, 1]);
        db.put(child1.clone(), vec![1]).unwrap();

        // Child 2: in both cache and DB
        let child2 = Nibbles::from_hex(vec![1, 2]);
        db.put(child2.clone(), vec![2]).unwrap();
        cache.mark_exists(&child2);

        // Child 3: not in cache or DB
        // (no action needed)

        let (missing_count, missing_requests) =
            node_missing_children_optimized(&branch, &path, &db, &cache).unwrap();

        // Only child 3 should be missing
        assert_eq!(missing_count, 1);
        assert_eq!(missing_requests.len(), 1);
        assert_eq!(missing_requests[0].path, Nibbles::from_hex(vec![1, 3]));
    }
}

#[cfg(test)]
mod performance_comparison_tests {
    use crate::sync::healing_cache::HealingCache;
    use crate::sync::state_healing::node_missing_children_optimized;
    use ethrex_trie::{InMemoryTrieDB, Nibbles, Node, NodeRef, NodeHash, TrieDB};
    use ethrex_trie::node::BranchNode;
    use ethrex_common::H256;
    use std::time::Instant;

    fn create_branch_all_children() -> Node {
        let mut choices: [NodeRef; 16] = Default::default();
        for i in 0..16 {
            let hash = NodeHash::Hashed(H256::from_low_u64_be(i as u64 + 1));
            choices[i] = NodeRef::Hash(hash);
        }
        Node::Branch(Box::new(BranchNode {
            choices,
            value: vec![],
        }))
    }

    #[test]
    fn perf_test_optimized_with_empty_cache() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let iterations = 10_000;

        let branch = create_branch_all_children();

        let start = Instant::now();
        for i in 0..iterations {
            let path = Nibbles::from_hex(vec![(i % 256) as u8, ((i / 256) % 256) as u8]);
            let _ = node_missing_children_optimized(&branch, &path, &db, &cache);
        }
        let elapsed = start.elapsed();

        let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
        println!(
            "Optimized (empty cache): {} iterations in {:?} ({:.0} ops/sec)",
            iterations, elapsed, ops_per_sec
        );

        // Should process at least 10k ops/sec
        assert!(ops_per_sec > 10_000.0, "Performance too low: {:.0} ops/sec", ops_per_sec);
    }

    #[test]
    fn perf_test_optimized_with_warm_cache() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let iterations = 10_000;

        let branch = create_branch_all_children();

        // Warm up cache with common paths
        for i in 0..256 {
            for j in 0..16 {
                let path = Nibbles::from_hex(vec![i as u8, j as u8]);
                cache.mark_exists(&path);
            }
        }

        let start = Instant::now();
        for i in 0..iterations {
            let path = Nibbles::from_hex(vec![(i % 256) as u8]);
            let _ = node_missing_children_optimized(&branch, &path, &db, &cache);
        }
        let elapsed = start.elapsed();

        let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
        println!(
            "Optimized (warm cache): {} iterations in {:?} ({:.0} ops/sec)",
            iterations, elapsed, ops_per_sec
        );

        // With warm cache, should be faster
        assert!(ops_per_sec > 20_000.0, "Performance too low with warm cache: {:.0} ops/sec", ops_per_sec);
    }

    #[test]
    fn perf_test_optimized_with_db_lookups() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();
        let iterations = 5_000;

        let branch = create_branch_all_children();

        // Populate DB with some paths
        for i in 0..256 {
            for j in 0..8 {
                let path = Nibbles::from_hex(vec![i as u8, j as u8]);
                db.put(path, vec![1, 2, 3]).unwrap();
            }
        }

        let start = Instant::now();
        for i in 0..iterations {
            let path = Nibbles::from_hex(vec![(i % 256) as u8]);
            let _ = node_missing_children_optimized(&branch, &path, &db, &cache);
        }
        let elapsed = start.elapsed();

        let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();
        println!(
            "Optimized (with DB): {} iterations in {:?} ({:.0} ops/sec)",
            iterations, elapsed, ops_per_sec
        );

        // With DB lookups, still should be reasonable
        assert!(ops_per_sec > 5_000.0, "Performance too low with DB: {:.0} ops/sec", ops_per_sec);
    }

    #[test]
    fn perf_test_batch_vs_sequential_db_lookup() {
        let db = InMemoryTrieDB::default();

        // Populate DB
        let paths: Vec<Nibbles> = (0..1000)
            .map(|i| Nibbles::from_bytes(&(i as u64).to_be_bytes()))
            .collect();

        for path in &paths {
            db.put(path.clone(), vec![1, 2, 3]).unwrap();
        }

        // Sequential lookups
        let start = Instant::now();
        for _ in 0..10 {
            for path in &paths {
                let _ = db.exists(path.clone());
            }
        }
        let sequential_elapsed = start.elapsed();

        // Batch lookups
        let start = Instant::now();
        for _ in 0..10 {
            let _ = db.exists_batch(&paths);
        }
        let batch_elapsed = start.elapsed();

        let speedup = sequential_elapsed.as_secs_f64() / batch_elapsed.as_secs_f64();

        println!(
            "Sequential: {:?}, Batch: {:?}, Speedup: {:.2}x",
            sequential_elapsed, batch_elapsed, speedup
        );

        // Batch should not be significantly slower
        assert!(speedup > 0.5, "Batch is too slow compared to sequential");
    }

    #[test]
    fn perf_test_cache_hit_ratio_impact() {
        // Test different cache hit ratios
        for hit_ratio in [0.0, 0.25, 0.5, 0.75, 0.9, 1.0] {
            let cache = HealingCache::new();
            let paths: Vec<Nibbles> = (0..1000)
                .map(|i| Nibbles::from_bytes(&(i as u64).to_be_bytes()))
                .collect();

            // Pre-populate cache based on hit ratio
            let num_to_cache = (paths.len() as f64 * hit_ratio) as usize;
            cache.mark_exists_batch(&paths[..num_to_cache]);

            let iterations = 100;
            let start = Instant::now();

            for _ in 0..iterations {
                for path in &paths {
                    let _ = cache.check_path(path);
                }
            }

            let elapsed = start.elapsed();
            let ops_per_sec = (iterations * paths.len()) as f64 / elapsed.as_secs_f64();

            println!(
                "Cache hit ratio {:.0}%: {:.0} ops/sec",
                hit_ratio * 100.0,
                ops_per_sec
            );
        }
    }

    #[test]
    fn perf_test_varying_branch_density() {
        let cache = HealingCache::new();
        let db = InMemoryTrieDB::default();

        // Test branches with different numbers of children
        for num_children in [1, 4, 8, 12, 16] {
            let mut choices: [NodeRef; 16] = Default::default();
            for i in 0..num_children {
                let hash = NodeHash::Hashed(H256::from_low_u64_be(i as u64 + 1));
                choices[i] = NodeRef::Hash(hash);
            }
            let branch = Node::Branch(Box::new(BranchNode {
                choices,
                value: vec![],
            }));

            let iterations = 5_000;
            let start = Instant::now();

            for i in 0..iterations {
                let path = Nibbles::from_hex(vec![(i % 256) as u8]);
                let _ = node_missing_children_optimized(&branch, &path, &db, &cache);
            }

            let elapsed = start.elapsed();
            let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();

            println!(
                "Branch with {} children: {:.0} ops/sec",
                num_children, ops_per_sec
            );
        }
    }
}
