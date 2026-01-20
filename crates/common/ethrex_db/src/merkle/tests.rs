//! Property-based tests for Merkle trie.

#[cfg(test)]
mod proptest_tests {
    use proptest::prelude::*;
    use crate::merkle::{MerkleTrie, Node, RlpEncoder, keccak256};
    use crate::merkle::node::EMPTY_ROOT;

    proptest! {
        #[test]
        fn trie_deterministic_root(
            entries in proptest::collection::vec(
                (proptest::collection::vec(any::<u8>(), 1..32),
                 proptest::collection::vec(any::<u8>(), 1..64)),
                1..20
            )
        ) {
            use std::collections::HashMap;

            // Deduplicate entries to ensure consistent final state regardless of order
            // (last write wins, so with duplicates, order would matter)
            let unique_entries: HashMap<Vec<u8>, Vec<u8>> = entries.into_iter().collect();
            let entries: Vec<_> = unique_entries.into_iter().collect();

            let mut trie1 = MerkleTrie::new();
            let mut trie2 = MerkleTrie::new();

            // Insert in original order
            for (key, value) in &entries {
                trie1.insert(key, value.clone());
            }

            // Insert in reverse order
            for (key, value) in entries.iter().rev() {
                trie2.insert(key, value.clone());
            }

            // Root hashes should be the same
            assert_eq!(trie1.root_hash(), trie2.root_hash());
        }

        #[test]
        fn trie_insert_get(
            key in proptest::collection::vec(any::<u8>(), 1..32),
            value in proptest::collection::vec(any::<u8>(), 1..64)
        ) {
            let mut trie = MerkleTrie::new();
            trie.insert(&key, value.clone());

            assert_eq!(trie.get(&key), Some(value.as_slice()));
        }

        #[test]
        fn trie_remove_returns_empty_root(
            key in proptest::collection::vec(any::<u8>(), 1..32),
            value in proptest::collection::vec(any::<u8>(), 1..64)
        ) {
            let mut trie = MerkleTrie::new();
            trie.insert(&key, value);
            trie.remove(&key);

            assert_eq!(trie.root_hash(), EMPTY_ROOT);
            assert!(trie.is_empty());
        }

        #[test]
        fn trie_update_changes_root(
            key in proptest::collection::vec(any::<u8>(), 1..32),
            value1 in proptest::collection::vec(any::<u8>(), 1..64),
            value2 in proptest::collection::vec(any::<u8>(), 1..64)
        ) {
            prop_assume!(value1 != value2);

            let mut trie = MerkleTrie::new();
            trie.insert(&key, value1);
            let hash1 = trie.root_hash();

            trie.insert(&key, value2);
            let hash2 = trie.root_hash();

            assert_ne!(hash1, hash2);
        }

        #[test]
        fn trie_multiple_keys_all_retrievable(
            entries in proptest::collection::vec(
                (proptest::collection::vec(any::<u8>(), 1..16),
                 proptest::collection::vec(any::<u8>(), 1..32)),
                1..50
            )
        ) {
            use std::collections::HashMap;
            let mut trie = MerkleTrie::new();
            let mut expected: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

            // Insert all entries (last value for each key wins)
            for (key, value) in &entries {
                trie.insert(key, value.clone());
                expected.insert(key.clone(), value.clone());
            }

            // Verify all are retrievable
            for (key, value) in &expected {
                prop_assert_eq!(trie.get(key), Some(value.as_slice()));
            }
        }

        #[test]
        fn trie_remove_specific_key_others_remain(
            entries in proptest::collection::vec(
                (proptest::collection::vec(any::<u8>(), 1..16),
                 proptest::collection::vec(any::<u8>(), 1..32)),
                2..10
            ),
            remove_idx in any::<usize>()
        ) {
            use std::collections::HashMap;
            let mut trie = MerkleTrie::new();
            let mut expected: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

            // Insert all entries
            for (key, value) in &entries {
                trie.insert(key, value.clone());
                expected.insert(key.clone(), value.clone());
            }

            if !expected.is_empty() {
                // Remove one key
                let keys: Vec<_> = expected.keys().cloned().collect();
                let key_to_remove = &keys[remove_idx % keys.len()];
                trie.remove(key_to_remove);
                expected.remove(key_to_remove);

                // Verify removed key is gone
                prop_assert!(trie.get(key_to_remove).is_none());

                // Verify others remain
                for (key, value) in &expected {
                    prop_assert_eq!(trie.get(key), Some(value.as_slice()));
                }
            }
        }

        #[test]
        fn trie_iter_returns_all_entries(
            entries in proptest::collection::vec(
                (proptest::collection::vec(any::<u8>(), 1..16),
                 proptest::collection::vec(any::<u8>(), 1..32)),
                1..20
            )
        ) {
            use std::collections::HashMap;
            let mut trie = MerkleTrie::new();
            let mut expected: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

            for (key, value) in &entries {
                trie.insert(key, value.clone());
                expected.insert(key.clone(), value.clone());
            }

            // Collect all entries from iterator
            let iter_entries: HashMap<Vec<u8>, Vec<u8>> = trie.iter()
                .map(|(k, v)| (k.to_vec(), v.to_vec()))
                .collect();

            prop_assert_eq!(iter_entries.len(), expected.len());
            for (key, value) in &expected {
                prop_assert_eq!(iter_entries.get(key), Some(value));
            }
        }

        #[test]
        fn trie_root_hash_stable_without_changes(
            entries in proptest::collection::vec(
                (proptest::collection::vec(any::<u8>(), 1..16),
                 proptest::collection::vec(any::<u8>(), 1..32)),
                1..10
            )
        ) {
            let mut trie = MerkleTrie::new();
            for (key, value) in &entries {
                trie.insert(key, value.clone());
            }

            // Call root_hash multiple times
            let hash1 = trie.root_hash();
            let hash2 = trie.root_hash();
            let hash3 = trie.root_hash();

            prop_assert_eq!(hash1, hash2);
            prop_assert_eq!(hash2, hash3);
        }

        #[test]
        fn trie_different_key_different_root(
            key1 in proptest::collection::vec(any::<u8>(), 1..16),
            key2 in proptest::collection::vec(any::<u8>(), 1..16),
            value in proptest::collection::vec(any::<u8>(), 1..32)
        ) {
            prop_assume!(key1 != key2);

            let mut trie1 = MerkleTrie::new();
            trie1.insert(&key1, value.clone());

            let mut trie2 = MerkleTrie::new();
            trie2.insert(&key2, value);

            prop_assert_ne!(trie1.root_hash(), trie2.root_hash());
        }

        #[test]
        fn rlp_encode_bytes_roundtrip(data in proptest::collection::vec(any::<u8>(), 0..256)) {
            let mut encoder = RlpEncoder::new();
            encoder.encode_bytes(&data);
            let encoded = encoder.as_bytes();

            // Verify it starts with correct prefix
            if data.is_empty() {
                assert_eq!(encoded, &[0x80]);
            } else if data.len() == 1 && data[0] < 0x80 {
                assert_eq!(encoded, &data[..]);
            } else if data.len() < 56 {
                assert_eq!(encoded[0], 0x80 + data.len() as u8);
                assert_eq!(&encoded[1..], &data[..]);
            }
        }

        #[test]
        fn rlp_encode_u64_valid(value in any::<u64>()) {
            let mut encoder = RlpEncoder::new();
            encoder.encode_u64(value);
            let encoded = encoder.as_bytes();

            // Verify encoding is non-empty
            prop_assert!(!encoded.is_empty());

            // For small values, encoding is just the value
            if value < 128 {
                prop_assert_eq!(encoded.len(), 1);
                prop_assert_eq!(encoded[0], value as u8);
            }
        }

        #[test]
        fn node_hash_deterministic(
            path in proptest::collection::vec(0u8..16u8, 0..32),
            value in proptest::collection::vec(any::<u8>(), 1..64)
        ) {
            let node1 = Node::leaf(path.clone(), value.clone());
            let node2 = Node::leaf(path, value);

            assert_eq!(node1.keccak(), node2.keccak());
        }

        #[test]
        fn keccak256_deterministic(data in proptest::collection::vec(any::<u8>(), 0..256)) {
            let hash1 = keccak256(&data);
            let hash2 = keccak256(&data);

            prop_assert_eq!(hash1, hash2);
        }

        #[test]
        fn keccak256_different_inputs_different_hashes(
            data1 in proptest::collection::vec(any::<u8>(), 1..64),
            data2 in proptest::collection::vec(any::<u8>(), 1..64)
        ) {
            prop_assume!(data1 != data2);

            let hash1 = keccak256(&data1);
            let hash2 = keccak256(&data2);

            prop_assert_ne!(hash1, hash2);
        }

        #[test]
        fn node_extension_deterministic(
            path in proptest::collection::vec(0u8..16u8, 1..32),
            child_hash in proptest::collection::vec(any::<u8>(), 32..=32)
        ) {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&child_hash);

            let node1 = Node::extension(path.clone(), hash);
            let node2 = Node::extension(path, hash);

            assert_eq!(node1.keccak(), node2.keccak());
        }
    }
}
