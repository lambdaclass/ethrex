use ethrex_trie::{TrieDB, TrieError};
use fjall::{Config, Keyspace, PartitionCreateOptions, PartitionHandle};
use std::sync::Arc;

pub struct FjallTrie {
    partition: PartitionHandle,
}

impl FjallTrie {
    pub fn new(partition: PartitionHandle) -> Self {
        Self { partition }
    }
}

impl TrieDB for FjallTrie {
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError> {
        match self.partition.get(&key) {
            Ok(Some(value)) => Ok(Some(value.to_vec())),
            Ok(None) => Ok(None),
            Err(e) => Err(TrieError::DbError(e.into())),
        }
    }

    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError> {
        self.partition
            .insert(&key, &value)
            .map_err(|e| TrieError::DbError(e.into()))
    }

    fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), TrieError> {
        // Create a transaction for batch operations

        for (key, value) in key_values {
            self.partition.insert(&key, &value)
                .map_err(|e| TrieError::DbError(e.into()))?;
        }

        // tx.commit().map_err(|e| TrieError::DbError(e.into()))
        Ok(())
    }
}

// Example of how to create a new FjallTrie
pub fn create_fjall_trie(path: &str, partition_name: &str) -> Result<FjallTrie, fjall::Error> {
    // Create the keyspace with the specified path
    let keyspace = Config::new(path).open()?;

    // Open or create a partition within the keyspace
    let partition = keyspace.open_partition(partition_name, PartitionCreateOptions::default())?;

    Ok(FjallTrie::new(partition))
}
