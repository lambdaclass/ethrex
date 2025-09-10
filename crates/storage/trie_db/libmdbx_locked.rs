use ethrex_trie::{Nibbles, error::TrieError};
use libmdbx::{
    RO,
    orm::{Database, Table, Transaction},
};
use std::{marker::PhantomData, sync::Arc};
/// Libmdbx implementation for the TrieDB trait, with get and put operations.
pub struct LibmdbxLockedTrieDB<T: Table> {
    db: &'static Arc<Database>,
    txn: Transaction<'static, RO>,
    phantom: PhantomData<T>,
}

use ethrex_trie::TrieDB;

use crate::trie_db::utils::nibbles_to_fixed_size;

impl<T> LibmdbxLockedTrieDB<T>
where
    T: Table<Key = [u8; 33], Value = Vec<u8>>,
{
    pub fn new(db: Arc<Database>) -> Result<Self, TrieError> {
        let db = Box::leak(Box::new(db));
        let txn = db.begin_read().map_err(TrieError::DbError)?;
        Ok(Self {
            db,
            txn,
            phantom: PhantomData,
        })
    }
}

impl<T> Drop for LibmdbxLockedTrieDB<T>
where
    T: Table,
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

impl<T> TrieDB for LibmdbxLockedTrieDB<T>
where
    T: Table<Key = [u8; 33], Value = Vec<u8>>,
{
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        self.txn.get::<T>(nibbles_to_fixed_size(key)).map_err(TrieError::DbError)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        Err(TrieError::DbError(anyhow::anyhow!(
            "LockedTrie is read-only"
        )))
    }
}
