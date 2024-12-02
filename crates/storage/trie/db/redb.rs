use std::sync::Arc;

use super::TrieDB;
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
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, crate::TrieError> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TABLE)?;
        Ok(table.get(&*key)?.map(|value| value.value().to_vec()))
    }

    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), crate::TrieError> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE)?;
            table.insert(&*key, &*value)?;
        }
        write_txn.commit()?;

        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), crate::TrieError> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE)?;
            for (key, value) in key_values {
                table.insert(&*key, &*value)?;
            }
        }
        write_txn.commit()?;

        Ok(())
    }
}
