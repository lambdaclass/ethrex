use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use crate::{Nibbles, Node, NodeHandle, Trie, TrieDB, TrieError};

pub type TrieWitness = Arc<Mutex<HashSet<Vec<u8>>>>;

pub struct TrieLogger {
    inner_db: Box<dyn TrieDB>,
    witness: TrieWitness,
}

impl TrieLogger {
    pub fn get_witness(&self) -> Result<HashSet<Vec<u8>>, TrieError> {
        let lock = self.witness.lock().map_err(|_| TrieError::LockError)?;
        Ok(lock.clone())
    }

    pub fn open_trie(trie: Trie) -> (TrieWitness, Trie) {
        let root = trie.hash_no_commit();
        let db = trie.db;
        let witness = Arc::new(Mutex::new(HashSet::new()));
        let logger = TrieLogger {
            inner_db: db,
            witness: witness.clone(),
        };
        (
            witness,
            Trie::open(Box::new(logger), root, trie.root.handle),
        )
    }
}

impl TrieDB for TrieLogger {
    fn get(&self, key: NodeHandle) -> Result<Option<Node>, TrieError> {
        let result = self.inner_db.get(key)?;
        if let Some(result) = result.as_ref() {
            let mut lock = self.witness.lock().map_err(|_| TrieError::LockError)?;
            lock.insert(result.encode_raw());
        }
        Ok(result)
    }
    fn get_path(&self, path: Nibbles) -> Result<Option<Node>, TrieError> {
        let result = self.inner_db.get_path(path)?;
        if let Some(result) = result.as_ref() {
            let mut lock = self.witness.lock().map_err(|_| TrieError::LockError)?;
            lock.insert(result.encode_raw());
        }
        Ok(result)
    }
}
