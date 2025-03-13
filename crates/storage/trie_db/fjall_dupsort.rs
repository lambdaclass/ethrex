use super::utils::node_hash_to_fixed_size;
use ethrex_trie::error::TrieError;
use ethrex_trie::TrieDB;
use fjall::{Keyspace, PartitionHandle};
use std::{marker::PhantomData, sync::Arc};

/// Fjall implementation for the TrieDB trait for a partition with a composite key
/// This trie will use a composite key of (fixed prefix + node hash) -> node value
/// The fixed prefix allows multiple logical tries to share the same physical partition
pub struct FjallDupsortTrieDB<SK>
where
    SK: Clone + AsRef<[u8]>,
{
    partition: PartitionHandle,
    fixed_key: SK,
}

impl<SK> FjallDupsortTrieDB<SK>
where
    SK: Clone + AsRef<[u8]>,
{
    pub fn new(partition: PartitionHandle, fixed_key: SK) -> Self {
        Self {
            partition,
            fixed_key,
        }
    }

    // Helper to create composite key from fixed key and node hash
    fn make_composite_key(&self, node_hash: Vec<u8>) -> Vec<u8> {
        let mut key = Vec::with_capacity(self.fixed_key.as_ref().len() + 33);
        key.extend_from_slice(self.fixed_key.as_ref());
        key.extend_from_slice(&node_hash_to_fixed_size(node_hash));
        key
    }
}

impl<SK> TrieDB for FjallDupsortTrieDB<SK>
where
    SK: Clone + AsRef<[u8]>,
{
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError> {
        let composite_key = self.make_composite_key(key);

        match self.partition.get(&composite_key) {
            Ok(Some(value)) => Ok(Some(value.to_vec())),
            Ok(None) => Ok(None),
            Err(e) => Err(TrieError::DbError(e.into())),
        }
    }

    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError> {
        let composite_key = self.make_composite_key(key);

        self.partition
            .insert(&composite_key, &value)
            .map_err(|e| TrieError::DbError(e.into()))
    }
fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), TrieError> {
    // Fjall doesn't have a transaction() method on PartitionHandle
    // We'll just loop through each item and insert them individually
    for (key, value) in key_values {
        let composite_key = self.make_composite_key(key);
        self.partition
            .insert(&composite_key, &value)
            .map_err(|e| TrieError::DbError(e.into()))?;
    }

    Ok(())
}

}

#[cfg(test)]
mod test {
    use super::*;
    use fjall::{Config, PartitionCreateOptions};

    fn new_partition() -> PartitionHandle {
        let temp_dir = tempfile::tempdir().unwrap();
        let keyspace = Config::new(temp_dir.path()).open().unwrap();
        keyspace
            .open_partition("nodes", PartitionCreateOptions::default())
            .unwrap()
    }

    #[test]
    fn simple_addition() {
        let partition = new_partition();
        let db = FjallDupsortTrieDB::<[u8; 32]>::new(partition, [5; 32]);

        assert_eq!(db.get("hello".into()).unwrap(), None);
        db.put("hello".into(), "value".into()).unwrap();
        assert_eq!(db.get("hello".into()).unwrap(), Some("value".into()));
    }

    #[test]
    fn different_keys() {
        let partition = new_partition();
        let db_a = FjallDupsortTrieDB::<[u8; 32]>::new(partition.clone(), [5; 32]);
        let db_b = FjallDupsortTrieDB::<[u8; 32]>::new(partition, [7; 32]);

        db_a.put("hello".into(), "hello!".into()).unwrap();
        db_b.put("hello".into(), "go away!".into()).unwrap();

        assert_eq!(db_a.get("hello".into()).unwrap(), Some("hello!".into()));
        assert_eq!(db_b.get("hello".into()).unwrap(), Some("go away!".into()));
    }
}
