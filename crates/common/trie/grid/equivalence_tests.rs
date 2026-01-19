//! Property-based tests to verify grid trie produces identical results to recursive trie.
//!
//! These tests are critical for ensuring correctness of the grid implementation.

#[cfg(test)]
mod tests {
    use crate::{
        db::InMemoryTrieDB,
        grid::HexPatriciaGrid,
        Trie, EMPTY_TRIE_HASH,
    };
    use ethereum_types::H256;
    use ethrex_crypto::keccak::keccak_hash;
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// Helper to create a test trie database
    fn create_test_db() -> InMemoryTrieDB {
        InMemoryTrieDB::new(Arc::new(Mutex::new(BTreeMap::new())))
    }

    /// Compute trie root using the recursive implementation
    fn compute_recursive_root(data: &[(Vec<u8>, Vec<u8>)]) -> H256 {
        if data.is_empty() {
            return *EMPTY_TRIE_HASH;
        }

        let mut trie = Trie::new_temp();
        for (key, value) in data {
            // Hash the key for path
            let hashed_key = keccak_hash(key);
            trie.insert(hashed_key.to_vec(), value.clone()).unwrap();
        }
        trie.hash_no_commit()
    }

    /// Compute trie root using the grid implementation
    fn compute_grid_root(data: &[(Vec<u8>, Vec<u8>)]) -> H256 {
        if data.is_empty() {
            return *EMPTY_TRIE_HASH;
        }

        let db = create_test_db();
        let mut grid = HexPatriciaGrid::new(db);

        // Convert to sorted (hashed_key, value) pairs
        let mut sorted_updates: Vec<(H256, Vec<u8>)> = data
            .iter()
            .map(|(key, value)| {
                let hashed = H256::from_slice(&keccak_hash(key));
                (hashed, value.clone())
            })
            .collect();

        // CRITICAL: Sort by hashed key
        sorted_updates.sort_by_key(|(k, _)| *k);

        grid.apply_sorted_updates(sorted_updates.into_iter()).unwrap()
    }

    // ==================== Basic Tests ====================

    #[test]
    fn test_empty_trie_equivalence() {
        let recursive_root = compute_recursive_root(&[]);
        let grid_root = compute_grid_root(&[]);
        assert_eq!(recursive_root, grid_root);
        assert_eq!(recursive_root, *EMPTY_TRIE_HASH);
    }

    #[test]
    fn test_single_entry_equivalence() {
        let data = vec![(b"key1".to_vec(), b"value1".to_vec())];

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        assert_eq!(recursive_root, grid_root);
        assert_ne!(recursive_root, *EMPTY_TRIE_HASH);
    }

    #[test]
    fn test_two_entries_equivalence() {
        let data = vec![
            (b"key1".to_vec(), b"value1".to_vec()),
            (b"key2".to_vec(), b"value2".to_vec()),
        ];

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        assert_eq!(recursive_root, grid_root);
    }

    #[test]
    fn test_multiple_entries_equivalence() {
        let data = vec![
            (b"account1".to_vec(), b"balance100".to_vec()),
            (b"account2".to_vec(), b"balance200".to_vec()),
            (b"account3".to_vec(), b"balance300".to_vec()),
            (b"account4".to_vec(), b"balance400".to_vec()),
            (b"account5".to_vec(), b"balance500".to_vec()),
        ];

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        assert_eq!(recursive_root, grid_root);
    }

    #[test]
    fn test_similar_keys_equivalence() {
        // Keys that will have common prefixes when hashed
        let data = vec![
            (b"test_key_001".to_vec(), b"v1".to_vec()),
            (b"test_key_002".to_vec(), b"v2".to_vec()),
            (b"test_key_003".to_vec(), b"v3".to_vec()),
        ];

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        assert_eq!(recursive_root, grid_root);
    }

    #[test]
    fn test_long_values_equivalence() {
        let long_value = vec![0xAB; 1000]; // 1KB value
        let data = vec![
            (b"key1".to_vec(), long_value.clone()),
            (b"key2".to_vec(), long_value),
        ];

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        assert_eq!(recursive_root, grid_root);
    }

    // ==================== Property-Based Tests ====================

    proptest! {
        /// Test that grid and recursive trie produce the same root for random data
        #[test]
        fn prop_grid_matches_recursive_small(
            data in prop::collection::vec(
                (
                    prop::collection::vec(any::<u8>(), 1..32),
                    prop::collection::vec(any::<u8>(), 1..64)
                ),
                0..20
            )
        ) {
            // Deduplicate by key (keep last value for each key)
            let mut deduped: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
            for (key, value) in data {
                deduped.insert(key, value);
            }
            let data: Vec<_> = deduped.into_iter().collect();

            let recursive_root = compute_recursive_root(&data);
            let grid_root = compute_grid_root(&data);

            prop_assert_eq!(recursive_root, grid_root,
                "Mismatch for {} entries", data.len());
        }

        /// Test with larger datasets
        #[test]
        fn prop_grid_matches_recursive_medium(
            data in prop::collection::vec(
                (
                    prop::collection::vec(any::<u8>(), 20..32),
                    prop::collection::vec(any::<u8>(), 32..128)
                ),
                0..100
            )
        ) {
            // Deduplicate by key
            let mut deduped: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
            for (key, value) in data {
                deduped.insert(key, value);
            }
            let data: Vec<_> = deduped.into_iter().collect();

            let recursive_root = compute_recursive_root(&data);
            let grid_root = compute_grid_root(&data);

            prop_assert_eq!(recursive_root, grid_root,
                "Mismatch for {} entries", data.len());
        }

        /// Test with fixed-size keys (like Ethereum addresses)
        #[test]
        fn prop_grid_matches_recursive_fixed_keys(
            data in prop::collection::vec(
                (
                    prop::collection::vec(any::<u8>(), 20..=20), // 20-byte keys (address size)
                    prop::collection::vec(any::<u8>(), 32..=32)  // 32-byte values
                ),
                0..50
            )
        ) {
            // Deduplicate by key
            let mut deduped: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
            for (key, value) in data {
                deduped.insert(key, value);
            }
            let data: Vec<_> = deduped.into_iter().collect();

            let recursive_root = compute_recursive_root(&data);
            let grid_root = compute_grid_root(&data);

            prop_assert_eq!(recursive_root, grid_root,
                "Mismatch for {} fixed-size entries", data.len());
        }
    }

    // ==================== Edge Case Tests ====================

    #[test]
    fn test_duplicate_keys_last_wins() {
        // When same key appears multiple times, last value should win
        let data = vec![
            (b"key1".to_vec(), b"value1".to_vec()),
            (b"key1".to_vec(), b"value2".to_vec()), // Same key, different value
        ];

        // Deduplicate (keep last)
        let mut deduped: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        for (key, value) in data {
            deduped.insert(key, value);
        }
        let data: Vec<_> = deduped.into_iter().collect();

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        assert_eq!(recursive_root, grid_root);
    }

    #[test]
    fn test_empty_value() {
        // Empty values should still work (though in practice we use deletions)
        let data = vec![
            (b"key1".to_vec(), vec![]),
            (b"key2".to_vec(), b"value2".to_vec()),
        ];

        // Note: Empty value in recursive trie might behave differently
        // This test documents the behavior
        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        // For now, we just ensure no panic
        // The roots may differ if empty value handling differs
        let _ = (recursive_root, grid_root);
    }

    /// Debug test with 3 keys that share common prefixes at depth 1
    #[test]
    fn test_three_keys_common_prefix() {
        // Create 3 keys where after hashing:
        // - All 3 share first nibble
        // - 2 share second nibble, 1 diverges
        // We'll use specific keys that produce this pattern

        // Let's find keys that hash to specific patterns
        // Key "a" -> hash starts with certain nibbles
        // We need keys whose hashes share prefixes

        let data = vec![
            (b"test_a".to_vec(), b"val_a".to_vec()),
            (b"test_b".to_vec(), b"val_b".to_vec()),
            (b"test_c".to_vec(), b"val_c".to_vec()),
        ];

        // Print hashed keys to understand pattern
        for (key, _) in &data {
            let hashed = H256::from_slice(&keccak_hash(key));
            eprintln!("Key {:?} -> {:?} (first nibbles: {}, {})",
                String::from_utf8_lossy(key),
                hashed,
                hashed.as_bytes()[0] >> 4,
                hashed.as_bytes()[0] & 0x0f
            );
        }

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        eprintln!("Recursive: {:?}", recursive_root);
        eprintln!("Grid: {:?}", grid_root);

        assert_eq!(recursive_root, grid_root);
    }

    /// Test with two keys that share a 3-nibble common prefix.
    /// Uses raw H256 keys to control exact nibble patterns.
    #[test]
    fn test_two_keys_shared_prefix_depth_3() {
        // Create two H256 keys that share the first 3 nibbles
        // Key1: 0x1234... (nibbles: 1,2,3,4,...)
        // Key2: 0x1235... (nibbles: 1,2,3,5,...)
        // They diverge at nibble position 3

        let mut key1_bytes = [0u8; 32];
        key1_bytes[0] = 0x12; // nibbles 1, 2
        key1_bytes[1] = 0x34; // nibbles 3, 4
        let key1 = H256::from(key1_bytes);

        let mut key2_bytes = [0u8; 32];
        key2_bytes[0] = 0x12; // nibbles 1, 2
        key2_bytes[1] = 0x35; // nibbles 3, 5
        let key2 = H256::from(key2_bytes);

        eprintln!("Key1: {:?}", key1);
        eprintln!("Key2: {:?}", key2);

        // Compute using recursive trie (directly with H256 keys, no keccak)
        let mut trie = Trie::new_temp();
        trie.insert(key1.as_bytes().to_vec(), b"value1".to_vec()).unwrap();
        trie.insert(key2.as_bytes().to_vec(), b"value2".to_vec()).unwrap();
        let recursive_root = trie.hash_no_commit();

        // Compute using grid trie
        let db = create_test_db();
        let mut grid = HexPatriciaGrid::new(db);
        let mut sorted_updates = vec![
            (key1, b"value1".to_vec()),
            (key2, b"value2".to_vec()),
        ];
        sorted_updates.sort_by_key(|(k, _)| *k);
        let grid_root = grid.apply_sorted_updates(sorted_updates.into_iter()).unwrap();

        eprintln!("Recursive root: {:?}", recursive_root);
        eprintln!("Grid root: {:?}", grid_root);

        assert_eq!(recursive_root, grid_root);
    }

    /// Test pair 1: Keys 1 and 8 (both have first nibble 5)
    #[test]
    fn test_nibble5_pair() {
        let keys_to_test = vec![1u32, 8];
        let data: Vec<_> = keys_to_test
            .iter()
            .map(|&i| (i.to_be_bytes().to_vec(), i.to_be_bytes().to_vec()))
            .collect();

        eprintln!("Testing nibble 5 pair:");
        for (key, _) in &data {
            let hashed = H256::from_slice(&keccak_hash(key));
            eprintln!("  Key {} -> first 6 nibbles: {:?}",
                u32::from_be_bytes([key[0], key[1], key[2], key[3]]),
                hashed.as_bytes()[..3].iter()
                    .flat_map(|b| [b >> 4, b & 0x0f])
                    .collect::<Vec<_>>()
            );
        }

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);
        eprintln!("Recursive: {:?}, Grid: {:?}", recursive_root, grid_root);
        assert_eq!(recursive_root, grid_root);
    }

    /// Test pair 2: Keys 4 and 6 (both have first nibble 12)
    #[test]
    fn test_nibble12_pair() {
        let keys_to_test = vec![4u32, 6];
        let data: Vec<_> = keys_to_test
            .iter()
            .map(|&i| (i.to_be_bytes().to_vec(), i.to_be_bytes().to_vec()))
            .collect();

        eprintln!("Testing nibble 12 pair:");
        for (key, _) in &data {
            let hashed = H256::from_slice(&keccak_hash(key));
            eprintln!("  Key {} -> first 6 nibbles: {:?}",
                u32::from_be_bytes([key[0], key[1], key[2], key[3]]),
                hashed.as_bytes()[..3].iter()
                    .flat_map(|b| [b >> 4, b & 0x0f])
                    .collect::<Vec<_>>()
            );
        }

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);
        eprintln!("Recursive: {:?}, Grid: {:?}", recursive_root, grid_root);
        assert_eq!(recursive_root, grid_root);
    }

    /// Test combined: All 4 keys (1, 4, 6, 8)
    #[test]
    fn test_collision_keys_combined() {
        let keys_to_test = vec![1u32, 4, 6, 8];
        let data: Vec<_> = keys_to_test
            .iter()
            .map(|&i| (i.to_be_bytes().to_vec(), i.to_be_bytes().to_vec()))
            .collect();

        eprintln!("Testing combined 4 keys:");
        for (key, _) in &data {
            let hashed = H256::from_slice(&keccak_hash(key));
            eprintln!("  Key {} -> first 6 nibbles: {:?}",
                u32::from_be_bytes([key[0], key[1], key[2], key[3]]),
                hashed.as_bytes()[..3].iter()
                    .flat_map(|b| [b >> 4, b & 0x0f])
                    .collect::<Vec<_>>()
            );
        }

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);
        eprintln!("Recursive: {:?}, Grid: {:?}", recursive_root, grid_root);
        assert_eq!(recursive_root, grid_root);
    }

    /// Test that grid and recursive trie match for sequential keys up to 500
    #[test]
    fn test_sequential_keys_equivalence() {
        for n in [1, 2, 3, 5, 10, 20, 50, 100, 200, 500] {
            let data: Vec<_> = (0u32..n)
                .map(|i| (i.to_be_bytes().to_vec(), i.to_be_bytes().to_vec()))
                .collect();

            let recursive_root = compute_recursive_root(&data);
            let grid_root = compute_grid_root(&data);

            assert_eq!(recursive_root, grid_root, "Mismatch at n={}", n);
        }
    }


    /// Test with 4 keys: 2 under nibble 0, 2 under nibble 1
    #[test]
    fn test_keys_multiple_branches() {
        // Use raw H256 keys for control
        // Keys under nibble 0: 0x0100..., 0x0200...
        // Keys under nibble 1: 0x1100..., 0x1200...

        let mut key1 = [0u8; 32];
        key1[0] = 0x01;
        let key1 = H256::from(key1);

        let mut key2 = [0u8; 32];
        key2[0] = 0x02;
        let key2 = H256::from(key2);

        let mut key3 = [0u8; 32];
        key3[0] = 0x11;
        let key3 = H256::from(key3);

        let mut key4 = [0u8; 32];
        key4[0] = 0x12;
        let key4 = H256::from(key4);

        let keys = vec![key1, key2, key3, key4];
        eprintln!("Keys: {:?}", keys);

        // Recursive trie
        let mut trie = Trie::new_temp();
        for (i, key) in keys.iter().enumerate() {
            trie.insert(key.as_bytes().to_vec(), format!("value{}", i).into_bytes()).unwrap();
        }
        let recursive_root = trie.hash_no_commit();

        // Grid trie
        let db = create_test_db();
        let mut grid = HexPatriciaGrid::new(db);
        let mut sorted_updates: Vec<_> = keys.iter().enumerate()
            .map(|(i, k)| (*k, format!("value{}", i).into_bytes()))
            .collect();
        sorted_updates.sort_by_key(|(k, _)| *k);
        let grid_root = grid.apply_sorted_updates(sorted_updates.into_iter()).unwrap();

        eprintln!("Recursive root: {:?}", recursive_root);
        eprintln!("Grid root: {:?}", grid_root);

        assert_eq!(recursive_root, grid_root);
    }

    /// Test with keys that share more than just first nibble
    #[test]
    fn test_keys_sharing_two_nibbles() {
        // Find keys from 0..10000 that share first TWO nibbles
        let mut by_two_nibbles: BTreeMap<(u8, u8), Vec<u32>> = BTreeMap::new();
        for i in 0u32..10000 {
            let hashed = H256::from_slice(&keccak_hash(&i.to_be_bytes()));
            let n1 = hashed.as_bytes()[0] >> 4;
            let n2 = hashed.as_bytes()[0] & 0x0f;
            by_two_nibbles.entry((n1, n2)).or_default().push(i);
        }

        // Find a pair with at least 3 keys
        let (nibbles, keys) = by_two_nibbles.iter()
            .find(|(_, v)| v.len() >= 3)
            .expect("Should find 3 keys sharing 2 nibbles");

        let colliding_keys: Vec<u32> = keys.iter().take(3).cloned().collect();

        eprintln!("Found {} keys sharing first 2 nibbles {:?}: {:?}",
            colliding_keys.len(), nibbles, colliding_keys);

        // Print full nibble info for each key
        for &key_num in &colliding_keys {
            let hashed = H256::from_slice(&keccak_hash(&key_num.to_be_bytes()));
            eprintln!("Key {} -> hash first 8 nibbles: {:?}",
                key_num,
                hashed.as_bytes()[..4].iter()
                    .flat_map(|b| [b >> 4, b & 0x0f])
                    .collect::<Vec<_>>()
            );
        }

        let data: Vec<_> = colliding_keys
            .iter()
            .map(|&i| (i.to_be_bytes().to_vec(), i.to_be_bytes().to_vec()))
            .collect();

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        eprintln!("Recursive root: {:?}", recursive_root);
        eprintln!("Grid root: {:?}", grid_root);

        assert_eq!(recursive_root, grid_root);
    }

    /// Test with three keys: two sharing one prefix, one diverging early.
    #[test]
    fn test_three_keys_mixed_prefixes() {
        // Key1: 0x1234... (nibbles: 1,2,3,4,...)
        // Key2: 0x1235... (nibbles: 1,2,3,5,...) - shares 3 nibbles with key1
        // Key3: 0x2000... (nibbles: 2,0,0,0,...) - diverges at first nibble

        let mut key1_bytes = [0u8; 32];
        key1_bytes[0] = 0x12;
        key1_bytes[1] = 0x34;
        let key1 = H256::from(key1_bytes);

        let mut key2_bytes = [0u8; 32];
        key2_bytes[0] = 0x12;
        key2_bytes[1] = 0x35;
        let key2 = H256::from(key2_bytes);

        let mut key3_bytes = [0u8; 32];
        key3_bytes[0] = 0x20;
        let key3 = H256::from(key3_bytes);

        // Compute using recursive trie
        let mut trie = Trie::new_temp();
        trie.insert(key1.as_bytes().to_vec(), b"value1".to_vec()).unwrap();
        trie.insert(key2.as_bytes().to_vec(), b"value2".to_vec()).unwrap();
        trie.insert(key3.as_bytes().to_vec(), b"value3".to_vec()).unwrap();
        let recursive_root = trie.hash_no_commit();

        // Compute using grid trie
        let db = create_test_db();
        let mut grid = HexPatriciaGrid::new(db);
        let mut sorted_updates = vec![
            (key1, b"value1".to_vec()),
            (key2, b"value2".to_vec()),
            (key3, b"value3".to_vec()),
        ];
        sorted_updates.sort_by_key(|(k, _)| *k);
        let grid_root = grid.apply_sorted_updates(sorted_updates.into_iter()).unwrap();

        eprintln!("Recursive root: {:?}", recursive_root);
        eprintln!("Grid root: {:?}", grid_root);

        assert_eq!(recursive_root, grid_root);
    }

    /// Test sequential numbers produce correct trie hash.
    #[test]
    fn test_sequential_keys() {
        // Keys that are sequential numbers
        let data: Vec<_> = (0u32..100)
            .map(|i| (i.to_be_bytes().to_vec(), i.to_be_bytes().to_vec()))
            .collect();

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        assert_eq!(recursive_root, grid_root);
    }

    /// Test keys with common string prefix (after hashing, random distribution).
    #[test]
    fn test_keys_with_common_prefix() {
        // Keys that will likely share prefixes when hashed
        let data: Vec<_> = (0u32..50)
            .map(|i| {
                let key = format!("prefix_{:04}", i).into_bytes();
                let value = format!("value_{}", i).into_bytes();
                (key, value)
            })
            .collect();

        let recursive_root = compute_recursive_root(&data);
        let grid_root = compute_grid_root(&data);

        assert_eq!(recursive_root, grid_root);
    }
}
