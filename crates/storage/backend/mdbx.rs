use ethrex_mdbx::env::{EnvConfig, Environment};
use ethrex_mdbx::error::MdbxError;
use ethrex_mdbx::txn::{RO, RW, Transaction};

use crate::api::tables::TABLES;
use crate::api::{
    PrefixResult, StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch,
};
use crate::error::StoreError;

use std::fmt;
use std::path::Path;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// MdbxBackend
// ---------------------------------------------------------------------------

/// MDBX-backed storage engine.
pub struct MdbxBackend {
    env: Arc<Environment>,
}

impl fmt::Debug for MdbxBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MdbxBackend").finish()
    }
}

impl MdbxBackend {
    /// Open (or create) an MDBX database at the given path with default config.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with_config(path, EnvConfig::default())
    }

    /// Open (or create) an MDBX database with custom configuration.
    pub fn open_with_config(
        path: impl AsRef<Path>,
        config: EnvConfig,
    ) -> Result<Self, StoreError> {
        // Ensure the directory exists
        std::fs::create_dir_all(path.as_ref()).map_err(|e| {
            StoreError::Custom(format!(
                "Failed to create MDBX data directory {:?}: {e}",
                path.as_ref()
            ))
        })?;

        let env = Environment::open(path.as_ref(), config, &TABLES).map_err(mdbx_to_store)?;
        Ok(MdbxBackend {
            env: Arc::new(env),
        })
    }
}

impl StorageBackend for MdbxBackend {
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        let txn = self.env.begin_rw_txn().map_err(mdbx_to_store)?;
        txn.clear_table(table).map_err(mdbx_to_store)?;
        txn.commit().map_err(mdbx_to_store)?;
        Ok(())
    }

    fn begin_read(&self) -> Result<Arc<dyn StorageReadView>, StoreError> {
        let txn = self.env.begin_ro_txn().map_err(mdbx_to_store)?;
        Ok(Arc::new(MdbxReadView {
            txn,
            _env: self.env.clone(),
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        Ok(Box::new(MdbxWriteBatch {
            env: self.env.clone(),
            ops: Vec::new(),
            flushed_txn: None,
        }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView + 'static>, StoreError> {
        let txn = self.env.begin_ro_txn().map_err(mdbx_to_store)?;
        Ok(Box::new(MdbxLockedView {
            txn,
            table: table_name,
            _env: self.env.clone(),
        }))
    }

    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        self.env.copy_to(path).map_err(mdbx_to_store)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MdbxReadView
// ---------------------------------------------------------------------------

struct MdbxReadView {
    txn: Transaction<RO>,
    /// Prevent the environment from being dropped while this view is alive.
    _env: Arc<Environment>,
}

impl StorageReadView for MdbxReadView {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.txn.get(table, key).map_err(mdbx_to_store)
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        let cursor = self.txn.cursor(table).map_err(mdbx_to_store)?;
        let iter = cursor.prefix_iter(prefix).map(|r| r.map_err(mdbx_to_store));
        Ok(Box::new(iter))
    }
}

// ---------------------------------------------------------------------------
// MdbxWriteBatch
// ---------------------------------------------------------------------------

enum WriteOp {
    Put {
        table: &'static str,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Delete {
        table: &'static str,
        key: Vec<u8>,
    },
}

struct MdbxWriteBatch {
    env: Arc<Environment>,
    ops: Vec<WriteOp>,
    /// Holds the open RW transaction after `flush()` has been called.
    flushed_txn: Option<Transaction<RW>>,
}

impl MdbxWriteBatch {
    /// Apply buffered ops to the given transaction, draining the ops buffer.
    fn apply_ops(txn: &Transaction<RW>, ops: Vec<WriteOp>) -> Result<(), StoreError> {
        for op in ops {
            match op {
                WriteOp::Put { table, key, value } => {
                    txn.put(table, &key, &value).map_err(mdbx_to_store)?;
                }
                WriteOp::Delete { table, key } => {
                    txn.del(table, &key).map_err(mdbx_to_store)?;
                }
            }
        }
        Ok(())
    }
}

impl StorageWriteBatch for MdbxWriteBatch {
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        self.ops.push(WriteOp::Put {
            table,
            key: key.to_vec(),
            value: value.to_vec(),
        });
        Ok(())
    }

    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        for (key, value) in batch {
            self.ops.push(WriteOp::Put { table, key, value });
        }
        Ok(())
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        self.ops.push(WriteOp::Delete {
            table,
            key: key.to_vec(),
        });
        Ok(())
    }

    fn flush(&mut self) -> Result<(), StoreError> {
        let ops = std::mem::take(&mut self.ops);
        if ops.is_empty() {
            return Ok(());
        }
        let txn = self.env.begin_rw_txn().map_err(mdbx_to_store)?;
        Self::apply_ops(&txn, ops)?;
        self.flushed_txn = Some(txn);
        Ok(())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        if let Some(txn) = self.flushed_txn.take() {
            // flush() was called — just commit the already-open transaction.
            txn.commit().map_err(mdbx_to_store)?;
        } else {
            // No flush — apply + commit in one step (original behavior).
            let ops = std::mem::take(&mut self.ops);
            if ops.is_empty() {
                return Ok(());
            }
            let txn = self.env.begin_rw_txn().map_err(mdbx_to_store)?;
            Self::apply_ops(&txn, ops)?;
            txn.commit().map_err(mdbx_to_store)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MdbxLockedView
// ---------------------------------------------------------------------------

struct MdbxLockedView {
    txn: Transaction<RO>,
    table: &'static str,
    /// Prevent the environment from being dropped while this view is alive.
    _env: Arc<Environment>,
}

impl StorageLockedView for MdbxLockedView {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.txn.get(self.table, key).map_err(mdbx_to_store)
    }
}

// ---------------------------------------------------------------------------
// Error conversion
// ---------------------------------------------------------------------------

fn mdbx_to_store(e: MdbxError) -> StoreError {
    StoreError::Custom(format!("MDBX error: {e}"))
}
