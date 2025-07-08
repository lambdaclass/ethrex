use std::sync::Arc;

use redb::{Database, MultimapTableDefinition};

use ethrex_trie::{NodeHash, TrieDB, TrieError};

use super::utils::node_hash_to_fixed_size;

const STORAGE_TRIE_NODES_TABLE: MultimapTableDefinition<([u8; 32], [u8; 33]), &[u8]> =
    MultimapTableDefinition::new("StorageTrieNodes");

/// RedB implementation for the TrieDB trait for a dupsort table with a fixed primary key.
/// For a dupsort table (A, B)[A] -> C, this trie will have a fixed A and just work on B -> C
/// A will be a fixed-size encoded key set by the user (of generic type SK), B will be a fixed-size encoded NodeHash and C will be an encoded Node
pub struct RedBMultiTableTrieDB {
    db: Arc<Database>,
    fixed_key: [u8; 32],
}

impl RedBMultiTableTrieDB {
    pub fn new(db: Arc<Database>, fixed_key: [u8; 32]) -> Self {
        Self { db, fixed_key }
    }
}

impl TrieDB for RedBMultiTableTrieDB {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        tracing::debug!(
            hashed_address = hex::encode(self.fixed_key),
            node_hash = hex::encode(node_hash_to_fixed_size(key)),
            "[QUERYING STORAGE TRIE NODE]",
        );
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| TrieError::DbError(e.into()))?;
        let table = read_txn
            .open_multimap_table(STORAGE_TRIE_NODES_TABLE)
            .map_err(|e| TrieError::DbError(e.into()))?;

        let mut iter = table
            .get((self.fixed_key, node_hash_to_fixed_size(key)))
            .map_err(|e| TrieError::DbError(e.into()))?;

        let guard = iter
            .next_back()
            .transpose()
            .map_err(|e| TrieError::DbError(e.into()))?;
        let mut data = guard.map(|g| g.value().to_vec()).unwrap_or_default();

        if data.is_empty() {
            return Ok(None);
        }

        // Remove the last 8 bytes that contain the block number
        data.truncate(data.len() - 8);
        Ok(Some(data))
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| TrieError::DbError(e.into()))?;
        {
            let mut table = write_txn
                .open_multimap_table(STORAGE_TRIE_NODES_TABLE)
                .map_err(|e| TrieError::DbError(e.into()))?;

            for (key, mut value) in key_values {
                // Add 8 extra bytes to store the block number
                value.extend_from_slice(&[0u8; 8]);
                table
                    .insert((self.fixed_key, node_hash_to_fixed_size(key)), &*value)
                    .map_err(|e| TrieError::DbError(e.into()))?;
            }
        }
        write_txn.commit().map_err(|e| TrieError::DbError(e.into()))
    }
}
