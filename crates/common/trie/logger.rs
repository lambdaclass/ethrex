use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use ethrex_rlp::decode::RLPDecode;

use bytes::Bytes;
use crate::{Nibbles, Node, NodeHash, Trie, TrieDB, TrieError};

pub type TrieWitness = Arc<Mutex<HashMap<NodeHash, Node>>>;

pub struct TrieLogger {
    inner_db: Box<dyn TrieDB>,
    witness: TrieWitness,
}

impl TrieLogger {
    pub fn get_witness(&self) -> Result<HashMap<NodeHash, Node>, TrieError> {
        let lock = self.witness.lock().map_err(|_| TrieError::LockError)?;
        Ok(lock.clone())
    }

    pub fn open_trie(trie: Trie) -> (TrieWitness, Trie) {
        let root = trie.hash_no_commit();
        let db = trie.db;
        let witness = Arc::new(Mutex::new(HashMap::new()));
        let logger = TrieLogger {
            inner_db: db,
            witness: witness.clone(),
        };
        (witness, Trie::open(Box::new(logger), root))
    }
}

impl TrieDB for TrieLogger {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let result = self.inner_db.get(key)?;
        if let Some(result) = result.as_ref()
            && let Ok(decoded) = Node::decode(result)
        {
            let mut lock = self.witness.lock().map_err(|_| TrieError::LockError)?;
            lock.insert(decoded.compute_hash(), decoded);
        }
        Ok(result)
    }

    fn put(&self, key: Nibbles, value: Bytes) -> Result<(), TrieError> {
        self.inner_db.put(key, value)
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Bytes)>) -> Result<(), TrieError> {
        self.inner_db.put_batch(key_values)
    }
}
