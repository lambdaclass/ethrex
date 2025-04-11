use std::{marker::PhantomData, sync::Arc};

use super::utils::node_hash_to_fixed_size;
use ethrex_trie::error::TrieError;
use ethrex_trie::TrieDB;
use libmdbx::orm::{Database, Table};

/// Libmdbx implementation for the TrieDB trait for a table with a fixed primary key.
/// For a table (A, B) -> C, this trie will have a fixed A and just work on B -> C
/// A will be a fixed-size encoded key set by the user (of type [u8;32]), B will be a fixed-size encoded NodeHash and C will be an encoded Node
pub struct LibmdbxFixedKeyTrieDB<T>
where
    T: Table<Key = ([u8; 32], [u8; 33]), Value = Vec<u8>>,
{
    db: Arc<Database>,
    fixed_key: [u8; 32],
    phantom: PhantomData<T>,
}

impl<T> LibmdbxFixedKeyTrieDB<T>
where
    T: Table<Key = ([u8; 32], [u8; 33]), Value = Vec<u8>>,
{
    pub fn new(db: Arc<Database>, fixed_key: [u8; 32]) -> Self {
        Self {
            db,
            fixed_key,
            phantom: PhantomData,
        }
    }
}

impl<T> TrieDB for LibmdbxFixedKeyTrieDB<T>
where
    T: Table<Key = ([u8; 32], [u8; 33]), Value = Vec<u8>>,
{
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError> {
        let txn = self.db.begin_read().map_err(TrieError::DbError)?;
        txn.get::<T>((self.fixed_key, node_hash_to_fixed_size(key)))
            .map_err(TrieError::DbError)
    }

    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError> {
        let txn = self.db.begin_readwrite().map_err(TrieError::DbError)?;
        txn.upsert::<T>((self.fixed_key, node_hash_to_fixed_size(key)), value)
            .map_err(TrieError::DbError)?;
        txn.commit().map_err(TrieError::DbError)
    }

    fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), TrieError> {
        let txn = self.db.begin_readwrite().map_err(TrieError::DbError)?;
        for (key, value) in key_values {
            txn.upsert::<T>((self.fixed_key, node_hash_to_fixed_size(key)), value)
                .map_err(TrieError::DbError)?;
        }
        txn.commit().map_err(TrieError::DbError)
    }
}

#[cfg(test)]
mod test {
    use crate::trie_db::test_utils::libmdbx::new_db;

    use super::*;
    use libmdbx::table;

    table!(
        /// (Key + NodeHash) to Node table
        ( Nodes )  ([u8;32], [u8;33]) => Vec<u8>
    );

    #[test]
    fn simple_addition() {
        let inner_db = new_db::<Nodes>();
        let db = LibmdbxFixedKeyTrieDB::<Nodes>::new(inner_db, [5; 32]);
        assert_eq!(db.get("hello".into()).unwrap(), None);
        db.put("hello".into(), "value".into()).unwrap();
        assert_eq!(db.get("hello".into()).unwrap(), Some("value".into()));
    }

    #[test]
    fn different_keys() {
        let inner_db = new_db::<Nodes>();
        let db_a = LibmdbxFixedKeyTrieDB::<Nodes>::new(inner_db.clone(), [5; 32]);
        let db_b = LibmdbxFixedKeyTrieDB::<Nodes>::new(inner_db, [7; 32]);
        db_a.put("hello".into(), "hello!".into()).unwrap();
        db_b.put("hello".into(), "go away!".into()).unwrap();
        assert_eq!(db_a.get("hello".into()).unwrap(), Some("hello!".into()));
        assert_eq!(db_b.get("hello".into()).unwrap(), Some("go away!".into()));
    }
}
