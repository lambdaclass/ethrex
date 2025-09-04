use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use ethrex_rlp::decode::RLPDecode;

use crate::{Node, NodeHash, NodeRef, Trie, TrieDB, TrieError};

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

    pub fn open_trie(trie: Trie, root: NodeRef) -> (TrieWitness, Trie) {
        let db = trie.db;
        let witness = Arc::new(Mutex::new(HashSet::new()));
        let logger = TrieLogger {
            inner_db: db,
            witness: witness.clone(),
        };
        (witness, Trie::open(Box::new(logger), root))
    }
}

impl TrieDB for TrieLogger {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let result = self.inner_db.get(key)?;
        if let Some(result) = result.as_ref() {
            if let Ok(decoded) = Node::decode(result) {
                let mut lock = self.witness.lock().map_err(|_| TrieError::LockError)?;
                lock.insert(decoded.encode_raw());
            };
        }
        Ok(result)
    }

    fn put(&self, key: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        {
            let mut lock = self.witness.lock().map_err(|_| TrieError::LockError)?;
            if let Ok(decoded) = Node::decode(&value) {
                lock.insert(decoded.encode_raw());
            };
        }
        self.inner_db.put(key, value)
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        {
            let mut lock = self.witness.lock().map_err(|_| TrieError::LockError)?;
            for (_, value) in key_values.iter() {
                if let Ok(decoded) = Node::decode(value) {
                    lock.insert(decoded.encode_raw());
                };
            }
        }
        self.inner_db.put_batch(key_values)
    }
}
