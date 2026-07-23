//! Write-recording storage decorator + a file-serializable undo log.
//!
//! [`RecordingBackend`] wraps any [`StorageBackend`] (in practice a RocksDB
//! backend) and sits *below* the `Store`'s in-memory buffering. When recording
//! is enabled, every physical write that reaches the backend is captured with
//! its pre-image, so the exact set of key-value mutations can later be undone.
//!
//! The undo log is deliberately serializable to a file: the benchmark harness
//! runs each step (warmup, measure, reset) in a *separate process* (a fresh
//! process guarantees a released RocksDB lock and a genuinely cold block
//! cache), so the warmup step captures the log to disk and later reset steps
//! load and replay it. Nothing is held only in memory across runs.
//!
//! Recording is a warmup-only cost: on the measured path recording is toggled
//! off and writes are delegated straight through with zero pre-image reads, so
//! the recorder never pollutes the cold-read measurement.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use ethrex_crypto::keccak::Keccak256;
use ethrex_storage::api::tables::TABLES;
use ethrex_storage::api::{StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch};
use ethrex_storage::error::StoreError;

/// A single recorded mutation and the value that was present *before* it.
///
/// `table` is stored owned (`String`) so the log serializes cleanly across a
/// process boundary; on replay it is mapped back to the matching `&'static str`
/// column-family constant. `prev == None` means the key was absent before the
/// write, so undoing the mutation deletes the key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoEntry {
    pub table: String,
    pub key: Vec<u8>,
    pub prev: Option<Vec<u8>>,
}

/// Storage backend decorator that records pre-images of every write while
/// recording is enabled.
#[derive(Debug)]
pub struct RecordingBackend {
    inner: Arc<dyn StorageBackend>,
    recording: AtomicBool,
    undo: Arc<Mutex<Vec<UndoEntry>>>,
}

impl RecordingBackend {
    /// Wraps `inner` in a recorder. Recording starts disabled.
    pub fn new(inner: Arc<dyn StorageBackend>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            recording: AtomicBool::new(false),
            undo: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Enables or disables pre-image recording for subsequent write batches.
    ///
    /// Each write batch snapshots the flag at `begin_write()` time, so toggle
    /// this only while the Store is quiescent (e.g. after
    /// `wait_for_persistence_idle()`); flipping it while a batch is in flight is
    /// a race whose outcome depends on timing.
    pub fn set_recording(&self, on: bool) {
        self.recording.store(on, Ordering::SeqCst);
    }

    /// Drains and returns the accumulated undo log.
    pub fn take_undo_log(&self) -> Vec<UndoEntry> {
        let mut guard = self.undo.lock().expect("undo log mutex poisoned");
        std::mem::take(&mut *guard)
    }

    /// Number of entries currently in the undo log.
    pub fn undo_log_len(&self) -> usize {
        self.undo.lock().expect("undo log mutex poisoned").len()
    }
}

impl StorageBackend for RecordingBackend {
    // NOTE: `clear_table` bypasses recording — a wipe is NOT captured in the undo
    // log and could not be reconstructed by `apply_undo_log`. No benchmark code
    // path calls it while recording; do not start doing so.
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        self.inner.clear_table(table)
    }

    fn begin_read(&self) -> Result<Arc<dyn StorageReadView>, StoreError> {
        self.inner.begin_read()
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        Ok(Box::new(RecordingWriteBatch {
            inner: self.inner.begin_write()?,
            backend: self.inner.clone(),
            recording: self.recording.load(Ordering::SeqCst),
            undo: self.undo.clone(),
        }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView + 'static>, StoreError> {
        self.inner.begin_locked(table_name)
    }

    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        self.inner.create_checkpoint(path)
    }
}

/// Write batch that records pre-images before delegating to the wrapped backend.
struct RecordingWriteBatch {
    inner: Box<dyn StorageWriteBatch>,
    backend: Arc<dyn StorageBackend>,
    recording: bool,
    undo: Arc<Mutex<Vec<UndoEntry>>>,
}

impl RecordingWriteBatch {
    /// Pushes a single pre-image entry onto the shared undo log.
    fn record_entry(&self, table: &'static str, key: &[u8], prev: Option<Vec<u8>>) {
        self.undo
            .lock()
            .expect("undo log mutex poisoned")
            .push(UndoEntry {
                table: table.to_string(),
                key: key.to_vec(),
                prev,
            });
    }
}

impl StorageWriteBatch for RecordingWriteBatch {
    // `put` is intentionally left to the trait default, which forwards to
    // `put_batch`, so the recording path lives in one place.

    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        if self.recording {
            // Capture the pre-image for each key before delegating. Recording only
            // happens during the unmeasured warmup, so a per-key `get` (rather than
            // a batched read) is fine and keeps the measured path untouched.
            let view = self.backend.begin_read()?;
            let mut guard = self.undo.lock().expect("undo log mutex poisoned");
            for (key, _) in &batch {
                let prev = view.get(table, key)?;
                guard.push(UndoEntry {
                    table: table.to_string(),
                    key: key.clone(),
                    prev,
                });
            }
        }
        self.inner.put_batch(table, batch)
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        if self.recording {
            let prev = self.backend.begin_read()?.get(table, key)?;
            self.record_entry(table, key, prev);
        }
        self.inner.delete(table, key)
    }

    fn merge(&mut self, table: &'static str, key: &[u8], operand: &[u8]) -> Result<(), StoreError> {
        if self.recording {
            let prev = self.backend.begin_read()?.get(table, key)?;
            if prev.is_some() {
                // Flattening a non-empty merge chain into a Put on undo would
                // silently change the physical layout and skew read-amp stats.
                // Benchmark blocks are new, so every merged key must be fresh.
                // NOTE: this can fire on the Store's dedicated persist-worker
                // thread (flush_block_data), which has no catch_unwind — the
                // panic kills that thread and bricks the Store for the rest of
                // the process. That is the intended fail-loud behavior for an
                // invariant violation; it is unreachable for well-formed
                // benchmark workloads (tx-location keys are always fresh).
                panic!(
                    "RecordingBackend: merge onto pre-existing key in table {table} \
                     (key len {}). The undo log cannot faithfully reconstruct a \
                     merge chain; benchmark blocks must only merge fresh keys.",
                    key.len()
                );
            }
            // Pre-image is absent, so undoing this merge deletes the key.
            self.record_entry(table, key, None);
        }
        self.inner.merge(table, key, operand)
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        self.inner.commit()
    }
}

// The digest scans the full `TABLES` list (below), not a hand-picked subset:
// block import writes far more CFs than the four state CFs (BLOCK_NUMBERS,
// TRANSACTION_LOCATIONS, RECEIPTS_V2, ACCOUNT_CODES/_METADATA, CHAIN_DATA,
// MISC_VALUES, BLOCK_ACCESS_LISTS, ...). Every write funnels through the undo
// log, so all of them are undoable; scanning all of them makes the pristine ==
// post-undo assertion catch reset drift in ANY CF, not just state/block ones.
// CFs untouched by import stay empty in both snapshots, so including them is free.

/// Serializes the undo log to `path` using bincode (compact binary; the log can
/// hold millions of `Vec<u8>` key/value entries, so a text format would bloat
/// size and parse time).
pub fn save_undo_log(log: &[UndoEntry], path: &Path) -> Result<()> {
    let bytes = bincode::serialize(log).context("serialize undo log")?;
    std::fs::write(path, bytes).with_context(|| format!("write undo log to {}", path.display()))?;
    Ok(())
}

/// Loads an undo log previously written by [`save_undo_log`].
pub fn load_undo_log(path: &Path) -> Result<Vec<UndoEntry>> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read undo log from {}", path.display()))?;
    bincode::deserialize(&bytes).context("deserialize undo log")
}

/// Replays an undo log against `backend`, restoring the pre-image state.
///
/// Entries are applied in **reverse** order within a single write batch, so
/// repeated writes to the same key are unwound to the earliest recorded
/// pre-image. `Some(prev)` restores the value; `None` deletes the key.
pub fn apply_undo_log(backend: &dyn StorageBackend, log: &[UndoEntry]) -> Result<()> {
    let mut tx = backend.begin_write()?;
    for entry in log.iter().rev() {
        let table = table_name_to_static(&entry.table)?;
        match &entry.prev {
            Some(prev) => tx.put(table, &entry.key, prev)?,
            None => tx.delete(table, &entry.key)?,
        }
    }
    tx.commit()?;
    Ok(())
}

/// Maps an owned table name back to its `&'static str` column-family constant.
///
/// The full [`TABLES`] list is authoritative and covers every CF that block
/// import writes, so an unknown name is a hard error rather than a silent skip.
fn table_name_to_static(name: &str) -> Result<&'static str> {
    TABLES
        .iter()
        .copied()
        .find(|t| *t == name)
        .ok_or_else(|| anyhow!("unknown table name in undo log: {name}"))
}

/// Computes a deterministic digest over the state + block column families.
///
/// Each CF is scanned in full via an empty-prefix iterator (the RocksDB backend
/// configures no prefix extractor, so an empty prefix is a total-order scan) and
/// its `(key, value)` pairs are folded into a streaming keccak in iterator
/// order. Table names and length prefixes are absorbed too, so entries and CF
/// boundaries cannot alias. Used to assert post-undo state byte-equals pristine.
pub fn state_digest(backend: &dyn StorageBackend) -> Result<[u8; 32]> {
    let view = backend.begin_read()?;
    let mut hasher = Keccak256::new();
    for table in TABLES {
        hasher.update((table.len() as u64).to_le_bytes());
        hasher.update(table.as_bytes());
        for item in view.prefix_iterator(table, &[])? {
            let (key, value): (Box<[u8]>, Box<[u8]>) =
                item.map_err(|e| anyhow!("scan {table}: {e}"))?;
            hasher.update((key.len() as u64).to_le_bytes());
            hasher.update(&key);
            hasher.update((value.len() as u64).to_le_bytes());
            hasher.update(&value);
        }
    }
    Ok(hasher.finalize())
}
