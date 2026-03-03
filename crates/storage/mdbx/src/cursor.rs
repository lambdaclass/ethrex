use ethrex_mdbx_sys as ffi;

use crate::error::MdbxError;
use crate::txn::RW;

use std::marker::PhantomData;

/// Result of a cursor get operation: optional (key, value) pair.
type CursorResult = Result<Option<(Vec<u8>, Vec<u8>)>, MdbxError>;

/// A cursor for iterating over key-value pairs in a table.
///
/// The cursor borrows the transaction it was opened on. The lifetime `'txn`
/// ensures the cursor cannot outlive its transaction.
pub struct Cursor<'txn, K> {
    cursor: *mut ffi::MDBX_cursor,
    _marker: PhantomData<&'txn K>,
}

// SAFETY: Compiled with MDBX_TXN_CHECKOWNER=0.
unsafe impl<K> Send for Cursor<'_, K> {}

impl<'txn, K> Cursor<'txn, K> {
    pub(crate) fn new(cursor: *mut ffi::MDBX_cursor) -> Self {
        Cursor {
            cursor,
            _marker: PhantomData,
        }
    }

    /// Seek to the first key >= `key` using SET_RANGE.
    pub fn seek_range(&mut self, key: &[u8]) -> CursorResult {
        let mut key_val = ffi::MDBX_val::from_slice(key);
        let mut data_val = ffi::MDBX_val::empty();

        let rc = unsafe {
            ffi::mdbx_cursor_get(
                self.cursor,
                &mut key_val,
                &mut data_val,
                ffi::MDBX_SET_RANGE,
            )
        };

        match rc {
            ffi::MDBX_SUCCESS => unsafe {
                Ok(Some((
                    key_val.as_slice().to_vec(),
                    data_val.as_slice().to_vec(),
                )))
            },
            ffi::MDBX_NOTFOUND => Ok(None),
            _ => Err(MdbxError::from_code(rc).unwrap_err()),
        }
    }

    /// Move to the next key-value pair.
    pub fn move_next(&mut self) -> CursorResult {
        let mut key_val = ffi::MDBX_val::empty();
        let mut data_val = ffi::MDBX_val::empty();

        let rc = unsafe {
            ffi::mdbx_cursor_get(self.cursor, &mut key_val, &mut data_val, ffi::MDBX_NEXT)
        };

        match rc {
            ffi::MDBX_SUCCESS => unsafe {
                Ok(Some((
                    key_val.as_slice().to_vec(),
                    data_val.as_slice().to_vec(),
                )))
            },
            ffi::MDBX_NOTFOUND => Ok(None),
            _ => Err(MdbxError::from_code(rc).unwrap_err()),
        }
    }

    /// Create a prefix iterator starting from `prefix`.
    ///
    /// Seeks to the first key >= `prefix`, then yields entries while the key
    /// starts with `prefix`. Stops when the prefix no longer matches or the
    /// table is exhausted.
    pub fn prefix_iter(self, prefix: &[u8]) -> PrefixIterator<'txn, K> {
        PrefixIterator {
            cursor: self,
            prefix: prefix.to_vec(),
            started: false,
            done: false,
        }
    }
}

impl Cursor<'_, RW> {
    /// Insert or update a key-value pair via cursor.
    ///
    /// More efficient than `Transaction::put` when writing multiple entries
    /// to the same table, because the cursor maintains position in the B-tree.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), MdbxError> {
        let key_val = ffi::MDBX_val::from_slice(key);
        let mut data_val = ffi::MDBX_val::from_slice(value);
        unsafe {
            MdbxError::from_code(ffi::mdbx_cursor_put(
                self.cursor,
                &key_val,
                &mut data_val,
                ffi::MDBX_UPSERT,
            ))?;
        }
        Ok(())
    }

    /// Delete the entry at the current cursor position.
    ///
    /// The cursor must be positioned on a valid entry (e.g. after a successful
    /// `seek_range` or `move_next`).
    pub fn del(&mut self) -> Result<(), MdbxError> {
        unsafe {
            MdbxError::from_code(ffi::mdbx_cursor_del(self.cursor, 0))?;
        }
        Ok(())
    }
}

impl<K> Drop for Cursor<'_, K> {
    fn drop(&mut self) {
        unsafe {
            ffi::mdbx_cursor_close(self.cursor);
        }
    }
}

/// An iterator over all key-value pairs whose keys start with a given prefix.
pub struct PrefixIterator<'txn, K> {
    cursor: Cursor<'txn, K>,
    prefix: Vec<u8>,
    started: bool,
    done: bool,
}

impl<K> Iterator for PrefixIterator<'_, K> {
    type Item = Result<(Box<[u8]>, Box<[u8]>), MdbxError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        let result = if !self.started {
            self.started = true;
            self.cursor.seek_range(&self.prefix)
        } else {
            self.cursor.move_next()
        };

        match result {
            Ok(Some((key, val))) if key.starts_with(&self.prefix) => Some(Ok((
                key.into_boxed_slice(),
                val.into_boxed_slice(),
            ))),
            Ok(_) => {
                self.done = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}
