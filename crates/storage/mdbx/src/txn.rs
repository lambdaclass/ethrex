use ethrex_mdbx_sys as ffi;

use crate::cursor::Cursor;
use crate::env::ReadTxnPool;
use crate::error::MdbxError;

use std::collections::HashMap;
use std::marker::PhantomData;
use std::ptr;
use std::sync::Arc;

/// Marker type for read-only transactions.
pub struct RO;

/// Marker type for read-write transactions.
pub struct RW;

/// Maximum number of reset RO handles kept in the pool.
const RO_TXN_POOL_CAP: usize = 32;

/// A type-safe MDBX transaction.
///
/// `K` is either [`RO`] or [`RW`], restricting which operations are available.
/// The transaction is aborted on drop if not explicitly committed.
pub struct Transaction<K> {
    txn: *mut ffi::MDBX_txn,
    dbis: Arc<HashMap<&'static str, ffi::MDBX_dbi>>,
    committed: bool,
    /// If set, the RO handle is returned to this pool on drop (via `mdbx_txn_reset`)
    /// instead of being aborted.
    pool: Option<Arc<ReadTxnPool>>,
    _marker: PhantomData<K>,
}

// SAFETY: Compiled with MDBX_TXN_CHECKOWNER=0, allowing cross-thread use.
unsafe impl<K> Send for Transaction<K> {}
unsafe impl<K> Sync for Transaction<K> {}

impl<K> Transaction<K> {
    pub(crate) fn new(
        txn: *mut ffi::MDBX_txn,
        dbis: Arc<HashMap<&'static str, ffi::MDBX_dbi>>,
    ) -> Self {
        Transaction {
            txn,
            dbis,
            committed: false,
            pool: None,
            _marker: PhantomData,
        }
    }

    pub(crate) fn new_pooled(
        txn: *mut ffi::MDBX_txn,
        dbis: Arc<HashMap<&'static str, ffi::MDBX_dbi>>,
        pool: Arc<ReadTxnPool>,
    ) -> Self {
        Transaction {
            txn,
            dbis,
            committed: false,
            pool: Some(pool),
            _marker: PhantomData,
        }
    }

    /// Look up the DBI handle for a table name.
    fn dbi(&self, table: &'static str) -> Result<ffi::MDBX_dbi, MdbxError> {
        self.dbis
            .get(table)
            .copied()
            .ok_or_else(|| MdbxError::Other(-1, format!("unknown table: {table}")))
    }

    /// Point lookup: get a value by key.
    ///
    /// Returns `Ok(None)` if the key is not found.
    /// The returned slice is valid for the lifetime of the transaction.
    pub fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, MdbxError> {
        let dbi = self.dbi(table)?;
        let key_val = ffi::MDBX_val::from_slice(key);
        let mut data_val = ffi::MDBX_val::empty();

        let rc = unsafe { ffi::mdbx_get(self.txn, dbi, &key_val, &mut data_val) };

        match rc {
            ffi::MDBX_SUCCESS => {
                // Copy out of mmap into an owned Vec
                let slice = unsafe { data_val.as_slice() };
                Ok(Some(slice.to_vec()))
            }
            ffi::MDBX_NOTFOUND => Ok(None),
            _ => Err(MdbxError::from_code(rc).unwrap_err()),
        }
    }

    /// Open a cursor on the given table.
    pub fn cursor(&self, table: &'static str) -> Result<Cursor<'_, K>, MdbxError> {
        let dbi = self.dbi(table)?;
        let mut cursor: *mut ffi::MDBX_cursor = ptr::null_mut();
        unsafe {
            MdbxError::from_code(ffi::mdbx_cursor_open(self.txn, dbi, &mut cursor))?;
        }
        Ok(Cursor::new(cursor))
    }

}

impl Transaction<RW> {
    /// Insert or update a key-value pair.
    pub fn put(
        &self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), MdbxError> {
        let dbi = self.dbi(table)?;
        let key_val = ffi::MDBX_val::from_slice(key);
        let mut data_val = ffi::MDBX_val::from_slice(value);

        unsafe {
            MdbxError::from_code(ffi::mdbx_put(
                self.txn,
                dbi,
                &key_val,
                &mut data_val,
                ffi::MDBX_UPSERT,
            ))?;
        }
        Ok(())
    }

    /// Delete a key (and all its values).
    pub fn del(&self, table: &'static str, key: &[u8]) -> Result<(), MdbxError> {
        let dbi = self.dbi(table)?;
        let key_val = ffi::MDBX_val::from_slice(key);

        let rc = unsafe { ffi::mdbx_del(self.txn, dbi, &key_val, ptr::null()) };

        match rc {
            ffi::MDBX_SUCCESS | ffi::MDBX_NOTFOUND => Ok(()),
            _ => Err(MdbxError::from_code(rc).unwrap_err()),
        }
    }

    /// Clear all entries in a table (but keep the table itself).
    pub fn clear_table(&self, table: &'static str) -> Result<(), MdbxError> {
        let dbi = self.dbi(table)?;
        unsafe {
            MdbxError::from_code(ffi::mdbx_drop(self.txn, dbi, false))?;
        }
        Ok(())
    }

    /// Commit the transaction, persisting all changes.
    ///
    /// Consumes the commit flag so that drop won't abort.
    pub fn commit(mut self) -> Result<(), MdbxError> {
        self.committed = true;
        unsafe {
            MdbxError::from_code(ffi::mdbx_txn_commit_ex(self.txn, ptr::null_mut()))?;
        }
        Ok(())
    }
}

impl<K> Drop for Transaction<K> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // If we have a pool, reset the handle and return it instead of aborting.
        if let Some(ref pool) = self.pool {
            let rc = unsafe { ffi::mdbx_txn_reset(self.txn) };
            if rc == ffi::MDBX_SUCCESS {
                if let Ok(mut vec) = pool.lock() {
                    if vec.len() < RO_TXN_POOL_CAP {
                        vec.push(self.txn);
                        return;
                    }
                }
            }
        }
        // Fallback: abort the transaction.
        unsafe {
            ffi::mdbx_txn_abort(self.txn);
        }
    }
}
