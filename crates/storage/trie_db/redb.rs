use std::sync::Arc;

use ethrex_trie::{TrieDB, TrieError};
use redb::{Database, TableDefinition};

const TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("Trie");

pub struct RedBTrie {
    db: Arc<Database>,
}

impl RedBTrie {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

impl TrieDB for RedBTrie {
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| TrieError::DbError(e.into()))?;
        let table = read_txn
            .open_table(TABLE)
            .map_err(|e| TrieError::DbError(e.into()))?;
        Ok(table
            .get(&*key)
            .map_err(|e| TrieError::DbError(e.into()))?
            .map(|value| value.value().to_vec()))
    }

    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| TrieError::DbError(e.into()))?;
        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|e| TrieError::DbError(e.into()))?;
            table
                .insert(&*key, &*value)
                .map_err(|e| TrieError::DbError(e.into()))?;
        }
        write_txn
            .commit()
            .map_err(|e| TrieError::DbError(e.into()))?;

        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), TrieError> {
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
                    .insert(&*key, &*value)
                    .map_err(|e| TrieError::DbError(e.into()))?;
            }
        }
        write_txn
            .commit()
            .map_err(|e| TrieError::DbError(e.into()))?;

        Ok(())
    }
}
