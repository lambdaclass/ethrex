use std::sync::Arc;

use crate::trie_db::utils::node_hash_to_fixed_size;
use ethrex_trie::{NodeHash, TrieDB, TrieError};
use redb::{Database, TableDefinition};

const TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("StateTrieNodes");

pub struct RedBTrie {
    db: Arc<Database>,
}

impl RedBTrie {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

impl TrieDB for RedBTrie {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        tracing::debug!(
            node_hash = hex::encode(node_hash_to_fixed_size(key)),
            "[QUERYING STATE TRIE NODE]",
        );
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| TrieError::DbError(e.into()))?;
        let table = read_txn
            .open_table(TABLE)
            .map_err(|e| TrieError::DbError(e.into()))?;

        Ok(table
            .get(key.as_ref())
            .map_err(|e| TrieError::DbError(e.into()))?
            .map(|guard| {
                let mut node_data = guard.value().to_vec();
                // Nodes are stored with 8 extra bytes at the end to store the block number
                // Remove the last 8 bytes that contain the block number
                node_data.truncate(node_data.len() - 8);
                node_data
            }))
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| TrieError::DbError(e.into()))?;
        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|e| TrieError::DbError(e.into()))?;
            for (key, mut value) in key_values {
                // Add 8 extra bytes to store the block number
                value.extend_from_slice(&[0u8; 8]);
                table
                    .insert(key.as_ref(), &*value)
                    .map_err(|e| TrieError::DbError(e.into()))?;
            }
        }
        write_txn
            .commit()
            .map_err(|e| TrieError::DbError(e.into()))?;

        Ok(())
    }
}
