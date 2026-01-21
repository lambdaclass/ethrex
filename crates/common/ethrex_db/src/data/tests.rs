//! Property-based tests for data structures.

#[cfg(test)]
mod proptest_tests {
    use crate::data::{NibblePath, SlottedArray};
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn nibble_path_from_bytes_roundtrip(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let path = NibblePath::from_bytes(&bytes);
            assert_eq!(path.len(), bytes.len() * 2);

            // Verify each nibble
            for (i, byte) in bytes.iter().enumerate() {
                assert_eq!(path.get(i * 2), byte >> 4);
                assert_eq!(path.get(i * 2 + 1), byte & 0x0F);
            }
        }

        #[test]
        fn nibble_path_slice_from(bytes in proptest::collection::vec(any::<u8>(), 1..32), start in 0usize..64) {
            let path = NibblePath::from_bytes(&bytes);
            let start = start % path.len().max(1);

            let sliced = path.slice_from(start);
            assert_eq!(sliced.len(), path.len() - start);

            for i in 0..sliced.len() {
                assert_eq!(sliced.get(i), path.get(start + i));
            }
        }

        #[test]
        fn nibble_path_common_prefix_symmetric(
            bytes1 in proptest::collection::vec(any::<u8>(), 0..32),
            bytes2 in proptest::collection::vec(any::<u8>(), 0..32)
        ) {
            let path1 = NibblePath::from_bytes(&bytes1);
            let path2 = NibblePath::from_bytes(&bytes2);

            // Common prefix should be symmetric
            assert_eq!(path1.common_prefix_len(&path2), path2.common_prefix_len(&path1));
        }

        #[test]
        fn nibble_path_common_prefix_bounds(
            bytes1 in proptest::collection::vec(any::<u8>(), 0..32),
            bytes2 in proptest::collection::vec(any::<u8>(), 0..32)
        ) {
            let path1 = NibblePath::from_bytes(&bytes1);
            let path2 = NibblePath::from_bytes(&bytes2);

            let common = path1.common_prefix_len(&path2);
            // Common prefix can't exceed the shorter path
            prop_assert!(common <= path1.len());
            prop_assert!(common <= path2.len());
        }

        #[test]
        fn nibble_path_identical_full_common_prefix(bytes in proptest::collection::vec(any::<u8>(), 0..32)) {
            let path1 = NibblePath::from_bytes(&bytes);
            let path2 = NibblePath::from_bytes(&bytes);

            // Identical paths should have full common prefix
            assert_eq!(path1.common_prefix_len(&path2), path1.len());
        }

        #[test]
        fn nibble_path_slice_preserves_values(bytes in proptest::collection::vec(any::<u8>(), 2..32)) {
            let path = NibblePath::from_bytes(&bytes);

            // Slice at various points
            for start in 0..path.len() {
                let sliced = path.slice_from(start);
                for i in 0..sliced.len() {
                    prop_assert_eq!(sliced.get(i), path.get(start + i));
                }
            }
        }

        #[test]
        fn slotted_array_insert_get(
            keys in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..8), 1..10),
            values in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..32), 1..10)
        ) {
            use std::collections::HashMap;
            let mut arr = SlottedArray::new();

            // Track the last value for each key (later inserts overwrite)
            let mut expected: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

            // Insert entries
            for (key, value) in keys.iter().zip(values.iter()) {
                let path = NibblePath::from_bytes(key);
                if arr.try_insert(&path, value) {
                    expected.insert(key.clone(), value.clone());
                }
            }

            // Verify entries match the last successfully inserted value for each key
            for (key, expected_value) in &expected {
                let path = NibblePath::from_bytes(key);
                let retrieved = arr.get(&path);
                assert_eq!(retrieved, Some(expected_value.clone()));
            }
        }

        #[test]
        fn slotted_array_delete(
            key in proptest::collection::vec(any::<u8>(), 1..8),
            value in proptest::collection::vec(any::<u8>(), 1..32)
        ) {
            let mut arr = SlottedArray::new();
            let path = NibblePath::from_bytes(&key);

            // Insert
            if arr.try_insert(&path, &value) {
                assert!(arr.get(&path).is_some());

                // Delete
                assert!(arr.delete(&path));
                assert!(arr.get(&path).is_none());
            }
        }

        #[test]
        fn slotted_array_no_duplicates(
            key in proptest::collection::vec(any::<u8>(), 1..8),
            value1 in proptest::collection::vec(any::<u8>(), 1..16),
            value2 in proptest::collection::vec(any::<u8>(), 1..16)
        ) {
            let mut arr = SlottedArray::new();
            let path = NibblePath::from_bytes(&key);

            if arr.try_insert(&path, &value1) {
                // Delete and re-insert with different value
                arr.delete(&path);
                if arr.try_insert(&path, &value2) {
                    let retrieved = arr.get(&path);
                    assert_eq!(retrieved, Some(value2));
                }
            }
        }

        #[test]
        fn slotted_array_update_overwrites(
            key in proptest::collection::vec(any::<u8>(), 1..8),
            value1 in proptest::collection::vec(any::<u8>(), 1..16),
            value2 in proptest::collection::vec(any::<u8>(), 1..16)
        ) {
            let mut arr = SlottedArray::new();
            let path = NibblePath::from_bytes(&key);

            // Insert first value
            if arr.try_insert(&path, &value1) {
                // Insert second value (should update)
                if arr.try_insert(&path, &value2) {
                    // Should retrieve the second value
                    let retrieved = arr.get(&path);
                    assert_eq!(retrieved, Some(value2));
                }
            }
        }

        #[test]
        fn slotted_array_delete_nonexistent_returns_false(
            key in proptest::collection::vec(any::<u8>(), 1..8)
        ) {
            let mut arr = SlottedArray::new();
            let path = NibblePath::from_bytes(&key);

            // Deleting from empty array should return false
            assert!(!arr.delete(&path));
        }

        #[test]
        fn slotted_array_multiple_deletes(
            seed in any::<u64>()
        ) {
            use std::collections::HashMap;
            let mut arr = SlottedArray::new();
            let mut inserted: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

            // Generate unique keys based on seed
            for i in 0..5 {
                let key = vec![(seed.wrapping_add(i)) as u8, ((seed.wrapping_add(i)) >> 8) as u8];
                let value = vec![i as u8; 8];
                let path = NibblePath::from_bytes(&key);
                if arr.try_insert(&path, &value) {
                    inserted.insert(key, value);
                }
            }

            if inserted.len() < 2 {
                return Ok(()); // Not enough entries to test
            }

            // Delete first half of inserted keys
            let keys_to_delete: Vec<_> = inserted.keys().take(inserted.len() / 2).cloned().collect();
            for key in &keys_to_delete {
                let path = NibblePath::from_bytes(key);
                arr.delete(&path);
            }

            // Verify deleted keys are gone
            for key in &keys_to_delete {
                let path = NibblePath::from_bytes(key);
                prop_assert!(arr.get(&path).is_none(), "Deleted key should be gone");
            }

            // Verify remaining keys still exist
            for (key, value) in &inserted {
                if !keys_to_delete.contains(key) {
                    let path = NibblePath::from_bytes(key);
                    prop_assert_eq!(arr.get(&path), Some(value.clone()), "Remaining key should exist");
                }
            }
        }
    }
}
