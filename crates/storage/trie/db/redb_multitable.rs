use std::sync::Arc;

use redb::{Database, MultimapTableDefinition};

use crate::TrieError;

use super::{utils::node_hash_to_fixed_size, TrieDB};

const STORAGE_TRIE_NODES_TABLE: MultimapTableDefinition<([u8; 32], [u8; 33]), &[u8]> =
    MultimapTableDefinition::new("StorageTrieNodes");

/// RedB implementation for the TrieDB trait for a dupsort table with a fixed primary key.
/// For a dupsort table (A, B)[A] -> C, this trie will have a fixed A and just work on B -> C
/// A will be a fixed-size encoded key set by the user (of generic type SK), B will be a fixed-size encoded NodeHash and C will be an encoded Node
pub struct RedBMultiTableTrieDB {
    db: Arc<Database>,
    fixed_key: [u8; 32],
}

impl RedBMultiTableTrieDB {
    pub fn new(db: Arc<Database>, fixed_key: [u8; 32]) -> Self {
        Self { db, fixed_key }
    }
}

impl TrieDB for RedBMultiTableTrieDB {
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_multimap_table(STORAGE_TRIE_NODES_TABLE)?;

        let values = table.get((self.fixed_key, node_hash_to_fixed_size(key)))?;

        let mut ret = vec![];
        for value in values {
            ret.push(value?.value().to_vec());
        }

        let ret_flattened = ret.concat();

        if ret.is_empty() {
            Ok(None)
        } else {
            Ok(Some(ret_flattened))
        }
    }

    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_multimap_table(STORAGE_TRIE_NODES_TABLE)?;
            table.insert((self.fixed_key, node_hash_to_fixed_size(key)), &*value)?;
        }
        write_txn.commit()?;

        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), TrieError> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_multimap_table(STORAGE_TRIE_NODES_TABLE)?;
            for (key, value) in key_values {
                table.insert((self.fixed_key, node_hash_to_fixed_size(key)), &*value)?;
            }
        }
        write_txn.commit()?;

        Ok(())
    }
}
