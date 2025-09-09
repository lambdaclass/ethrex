use ethrex_trie::{NodeHash, error::TrieError};
use libmdbx::{
    RO,
    orm::{Database, Table, Transaction},
};
use smallvec::SmallVec;
use std::{marker::PhantomData, sync::Arc};
/// Libmdbx implementation for the TrieDB trait, with get and put operations.
pub struct LibmdbxLockedTrieDB<T: Table> {
    db: &'static Arc<Database>,
    txn: Transaction<'static, RO>,
    phantom: PhantomData<T>,
}

use ethrex_trie::TrieDB;

impl<T> LibmdbxLockedTrieDB<T>
where
    T: Table<Key = NodeHash, Value = Vec<u8>>,
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
    T: Table<Key = NodeHash, Value = Vec<u8>>,
{
    fn get(
        &self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        node_hash: NodeHash,
    ) -> Result<Option<Vec<u8>>, TrieError> {
        self.txn.get::<T>(node_hash).map_err(TrieError::DbError)
    }

    fn put_batch(
        &self,
        key_values: Vec<(usize, SmallVec<[u8; 32]>, NodeHash, Vec<u8>)>,
    ) -> Result<(), TrieError> {
        Err(TrieError::DbError(anyhow::anyhow!(
            "LockedTrie is read-only"
        )))
    }
}
