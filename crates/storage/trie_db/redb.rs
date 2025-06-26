use std::sync::Arc;

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
            .map(|value| value.value().to_vec()))
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
            for (key, value) in key_values {
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
