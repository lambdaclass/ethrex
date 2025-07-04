use std::{marker::PhantomData, sync::Arc};

use super::utils::node_hash_to_fixed_size;
use ethrex_trie::TrieDB;
use ethrex_trie::{NodeHash, error::TrieError};
use libmdbx::orm::{Database, DupSort, Encodable};

/// Libmdbx implementation for the TrieDB trait for a dupsort table with a fixed primary key.
/// For a dupsort table (A, B)[A] -> C, this trie will have a fixed A and just work on B -> C
/// A will be a fixed-size encoded key set by the user (of generic type SK), B will be a fixed-size encoded NodeHash and C will be an encoded Node
pub struct LibmdbxDupsortTrieDB<T, SK>
where
    T: DupSort<Key = (SK, [u8; 33]), SeekKey = SK, Value = Vec<u8>>,
    SK: Clone + Encodable,
{
    db: Arc<Database>,
    fixed_key: SK,
    phantom: PhantomData<T>,
}

impl<T, SK> LibmdbxDupsortTrieDB<T, SK>
where
    T: DupSort<Key = (SK, [u8; 33]), SeekKey = SK, Value = Vec<u8>>,
    SK: Clone + Encodable,
{
    pub fn new(db: Arc<Database>, fixed_key: T::SeekKey) -> Self {
        Self {
            db,
            fixed_key,
            phantom: PhantomData,
        }
    }
}

#[async_trait::async_trait]
impl<T, SK> TrieDB for LibmdbxDupsortTrieDB<T, SK>
where
    T: DupSort<Key = (SK, [u8; 33]), SeekKey = SK, Value = Vec<u8>>,
    SK: Clone + Encodable + 'static,
{
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let txn = self.db.begin_read().map_err(TrieError::DbError)?;
        txn.get::<T>((self.fixed_key.clone(), node_hash_to_fixed_size(key)))
            .map_err(TrieError::DbError)
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let txn = self.db.begin_readwrite().map_err(TrieError::DbError)?;
        for (key, value) in key_values {
            txn.upsert::<T>(
                (self.fixed_key.clone(), node_hash_to_fixed_size(key)),
                value,
            )
            .map_err(TrieError::DbError)?;
        }
        txn.commit().map_err(TrieError::DbError)
    }

    async fn put_batch_async(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let db = self.db.clone();
        let fixed_key = self.fixed_key.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(TrieError::DbError)?;
            for (key, value) in key_values {
                txn.upsert::<T>((fixed_key.clone(), node_hash_to_fixed_size(key)), value)
                    .map_err(TrieError::DbError)?;
            }
            txn.commit().map_err(TrieError::DbError)
        })
        .await
        .map_err(|e| TrieError::DbError(e.into()))?
    }
}

#[cfg(test)]
mod test {
    use crate::trie_db::test_utils::libmdbx::new_db;

    use super::*;
    use libmdbx::{dupsort, table};

    dupsort!(
        /// (Key + NodeHash) to Node table
        ( Nodes )  ([u8;32], [u8;33])[[u8;32]] => Vec<u8>
    );

    #[test]
    fn simple_addition() {
        let inner_db = new_db::<Nodes>();
        let db = LibmdbxDupsortTrieDB::<Nodes, [u8; 32]>::new(inner_db, [5; 32]);
        let key = NodeHash::from_encoded_raw(b"hello");
        assert_eq!(db.get(key).unwrap(), None);
        db.put(key, "value".into()).unwrap();
        assert_eq!(db.get(key).unwrap(), Some("value".into()));
    }

    #[test]
    fn different_keys() {
        let inner_db = new_db::<Nodes>();
        let db_a = LibmdbxDupsortTrieDB::<Nodes, [u8; 32]>::new(inner_db.clone(), [5; 32]);
        let db_b = LibmdbxDupsortTrieDB::<Nodes, [u8; 32]>::new(inner_db, [7; 32]);
        let key = NodeHash::from_encoded_raw(b"hello");
        db_a.put(key, "hello!".into()).unwrap();
        db_b.put(key, "go away!".into()).unwrap();
        assert_eq!(db_a.get(key).unwrap(), Some("hello!".into()));
        assert_eq!(db_b.get(key).unwrap(), Some("go away!".into()));
    }
}
