use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use ethrex_rlp::decode::RLPDecode;
use ethrex_trie::{Node, NodeHash, TrieDB, TrieError};
use redb::{Database, TableDefinition};

const TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("Trie");

pub struct RedBTrie {
    db: Arc<Database>,
    record_witness: bool,
    witness: Arc<Mutex<HashSet<Vec<u8>>>>,
}

impl RedBTrie {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            record_witness: false,
            witness: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl TrieDB for RedBTrie {
    fn record_witness(&mut self) {
        self.record_witness = true;
    }

    fn witness(&self) -> HashSet<Vec<u8>> {
        let lock = self.witness.lock().unwrap();
        lock.clone()
    }

    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| TrieError::DbError(e.into()))?;
        let table = read_txn
            .open_table(TABLE)
            .map_err(|e| TrieError::DbError(e.into()))?;
        let value = table
            .get(key.as_ref())
            .map_err(|e| TrieError::DbError(e.into()))?
            .map(|value| value.value().to_vec());
        if !self.record_witness {
            return Ok(value);
        }
        if let Some(value) = value.as_ref() {
            if let Ok(decoded) = Node::decode(value) {
                let mut lock = self.witness.lock().map_err(|_| TrieError::LockError)?;
                lock.insert(decoded.encode_raw());
            }
        }
        Ok(value)
    }

    fn put(&self, key: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| TrieError::DbError(e.into()))?;
        {
            let mut table = write_txn
                .open_table(TABLE)
                .map_err(|e| TrieError::DbError(e.into()))?;
            table
                .insert(key.as_ref(), &*value)
                .map_err(|e| TrieError::DbError(e.into()))?;
        }
        write_txn
            .commit()
            .map_err(|e| TrieError::DbError(e.into()))?;

        Ok(())
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
