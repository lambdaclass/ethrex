#![allow(clippy::unwrap_used)]

use std::marker::PhantomData;
use std::sync::Arc;

use ethrex_common::H256;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::TrieError;
use ethrex_trie::{NodeHash, TrieDB};
use reth_db::cursor::DbCursorRW;
use reth_db::transaction::DbTx;
use reth_db::transaction::DbTxMut;
use reth_db::{Database, DatabaseEnv};
use reth_db_api::table::Table as RethTable;

use crate::store_db::mdbx_fork::StateTrieNodes;
use crate::store_db::mdbx_fork::StorageTriesNodes;

pub struct MDBXTrieDB<T: RethTable> {
    db: Arc<DatabaseEnv>,
    phantom: PhantomData<T>,
}

impl<T> MDBXTrieDB<T>
where
    T: RethTable,
{
    pub fn new(db: Arc<DatabaseEnv>) -> Self {
        Self {
            db,
            phantom: PhantomData,
        }
    }
}

impl TrieDB for MDBXTrieDB<StateTrieNodes> {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = self.db.tx().map_err(|e| TrieError::DbError(e.into()))?;
        let node_hash_bytes = key.as_ref().to_vec();
        tx.get::<StateTrieNodes>(node_hash_bytes)
            .map_err(|e| TrieError::DbError(e.into()))
    }

    fn put(&self, key: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        let tx = self.db.tx_mut().map_err(|e| TrieError::DbError(e.into()))?;
        let node_hash_bytes = key.as_ref().to_vec();
        tx.put::<StateTrieNodes>(node_hash_bytes, value)
            .map_err(|e| TrieError::DbError(e.into()))?;
        tx.commit().map_err(|e| TrieError::DbError(e.into()))?;
        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let txn = self.db.tx_mut().map_err(|e| TrieError::DbError(e.into()))?;
        for (k, v) in key_values {
            let node_hash_bytes = k.as_ref().to_vec();
            txn.put::<StateTrieNodes>(node_hash_bytes, v)
                .map_err(|e| TrieError::DbError(e.into()))?;
        }
        txn.commit().map_err(|e| TrieError::DbError(e.into()))?;
        Ok(())
    }
}

pub struct MDBXTrieWithFixedKey<T: RethTable> {
    db: Arc<DatabaseEnv>,
    phantom: PhantomData<T>,
    fixed_key: H256,
}

impl<T> MDBXTrieWithFixedKey<T>
where
    T: RethTable,
{
    pub fn new(db: Arc<DatabaseEnv>, fixed_key: H256) -> Self {
        Self {
            fixed_key,
            db,
            phantom: PhantomData,
        }
    }
}

impl TrieDB for MDBXTrieWithFixedKey<StorageTriesNodes> {
    fn get(&self, subkey: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = self.db.tx().unwrap();
        let key = (self.fixed_key, subkey).encode_to_vec();
        let value = tx.get::<StorageTriesNodes>(key).unwrap();
        Ok(value)
    }

    fn put(&self, subkey: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        let tx = self.db.tx_mut().unwrap();
        let key = (self.fixed_key, subkey).encode_to_vec();
        tx.put::<StorageTriesNodes>(key, value).unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let tx = self.db.tx_mut().unwrap();

        let mut cursor = tx.cursor_write::<StorageTriesNodes>().unwrap();

        for (subkey, value) in key_values {
            let key = (self.fixed_key, subkey).encode_to_vec();
            cursor.upsert(key, value).unwrap();
        }

        tx.commit().unwrap();
        Ok(())
    }
}
