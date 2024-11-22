use super::TrieDB;
use crate::error::TrieError;

/// Used for small/pruned tries that don't have a database and just cache their nodes.
pub struct NullTrieDB;

impl TrieDB for NullTrieDB {
    fn get(&self, _key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError> {
        Ok(None)
    }

    fn put(&self, _key: Vec<u8>, _value: Vec<u8>) -> Result<(), TrieError> {
        Ok(())
    }
}
