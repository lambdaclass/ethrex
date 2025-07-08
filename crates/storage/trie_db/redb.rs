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
                let node_data = guard.value().to_vec();

                // 🔍 LOG CRÍTICO: Verificar longitud antes de truncate
                tracing::debug!(
                    node_hash = hex::encode(node_hash_to_fixed_size(key)),
                    data_length = node_data.len(),
                    data_hex = hex::encode(&node_data),
                    "[REDB GET] Raw node data before truncate"
                );

                if node_data.len() < 8 {
                    tracing::error!(
                        node_hash = hex::encode(node_hash_to_fixed_size(key)),
                        data_length = node_data.len(),
                        data_hex = hex::encode(&node_data),
                        "[REDB GET] Node data too short! Expected at least 8 bytes"
                    );
                    return vec![]; // Devolver vacío en lugar de crashear
                }

                let mut result = node_data;
                result.truncate(result.len() - 8);

                tracing::debug!(
                    node_hash = hex::encode(node_hash_to_fixed_size(key)),
                    final_length = result.len(),
                    final_hex = hex::encode(&result),
                    "[REDB GET] Final node data after truncate"
                );

                result
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
