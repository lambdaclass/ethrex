use std::{marker::PhantomData, sync::Arc};

use super::utils::nibbles_to_fixed_size;
use ethrex_trie::TrieDB;
use ethrex_trie::{Nibbles, error::TrieError};
use libmdbx::RO;
use libmdbx::orm::{Database, DupSort, Encodable, Transaction};

/// Libmdbx implementation for the TrieDB trait for a dupsort table with a fixed primary key.
/// For a dupsort table (A, B)[A] -> C, this trie will have a fixed A and just work on B -> C
/// A will be a fixed-size encoded key set by the user (of generic type SK), B will be a fixed-size encoded Nibbles and C will be an encoded Node
pub struct LibmdbxLockedDupsortTrieDB<T, SK>
where
    T: DupSort<Key = (SK, [u8; 33]), SeekKey = SK, Value = Vec<u8>>,
    SK: Clone + Encodable,
{
    db: &'static Arc<Database>,
    txn: Transaction<'static, RO>,
    fixed_key: SK,
    phantom: PhantomData<T>,
}

impl<T, SK> LibmdbxLockedDupsortTrieDB<T, SK>
where
    T: DupSort<Key = (SK, [u8; 33]), SeekKey = SK, Value = Vec<u8>>,
    SK: Clone + Encodable,
{
    pub fn new(db: Arc<Database>, fixed_key: T::SeekKey) -> Result<Self, TrieError> {
        let db = Box::leak(Box::new(db));
        let txn = db.begin_read().map_err(TrieError::DbError)?;
        Ok(Self {
            db,
            txn,
            fixed_key,
            phantom: PhantomData,
        })
    }
}

impl<T, SK> Drop for LibmdbxLockedDupsortTrieDB<T, SK>
where
    T: DupSort<Key = (SK, [u8; 33]), SeekKey = SK, Value = Vec<u8>>,
    SK: Clone + Encodable,
{
    fn drop(&mut self) {
        // The struct needs a Transaction referencing a Database object
        unsafe {
            drop(Box::from_raw(
                self.db as *const Arc<Database> as *mut Arc<Database>,
            ));
        }
    }
}

impl<T, SK> TrieDB for LibmdbxLockedDupsortTrieDB<T, SK>
where
    T: DupSort<Key = (SK, [u8; 33]), SeekKey = SK, Value = Vec<u8>>,
    SK: Clone + Encodable,
{
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        self.txn
            .get::<T>((self.fixed_key.clone(), nibbles_to_fixed_size(key)))
            .map_err(TrieError::DbError)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        Err(TrieError::DbError(anyhow::anyhow!(
            "LockedTrie is read-only"
        )))
    }
}
