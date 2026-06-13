use crate::peer_handler::DumpError;
use crate::snap::constants::CODE_HASH_WRITE_BUFFER_SIZE;
use crate::sync::SyncError;
use crate::utils::{dump_to_file, get_code_hashes_snapshot_file};
use ethrex_common::H256;
use ethrex_rlp::encode::RLPEncode;
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinSet;
use tracing::error;

/// Manages code hash collection and async file writing
pub struct CodeHashCollector {
    // Buffer to store code hashes
    buffer: HashSet<H256>,
    // Directory to store code hashes
    snapshots_dir: PathBuf,
    // Index of the current code hash file
    file_index: u64,
    // JoinSet to manage async disk writes
    disk_tasks: JoinSet<Result<(), DumpError>>,
    // Notified with each snapshot file path once its write completes; the
    // receiving end is the streaming bytecode fetcher. Dropped by `finish`,
    // which closes the channel and lets the fetcher drain.
    notify_tx: UnboundedSender<PathBuf>,
}

impl CodeHashCollector {
    /// Creates a new code collector
    pub fn new(snapshots_dir: PathBuf, notify_tx: UnboundedSender<PathBuf>) -> Self {
        Self {
            buffer: HashSet::new(),
            snapshots_dir,
            file_index: 0,
            disk_tasks: JoinSet::new(),
            notify_tx,
        }
    }

    /// Adds a code hash to the buffer
    pub fn add(&mut self, hash: H256) {
        self.buffer.insert(hash);
    }

    // The optimization for rocksdb database doesn't use this method
    #[cfg(not(feature = "rocksdb"))]
    /// Extends the buffer with a list of code hashes
    pub fn extend(&mut self, hashes: impl IntoIterator<Item = H256>) {
        self.buffer.extend(hashes);
    }

    /// Flushes the buffer to a file if the buffer is larger than [`CODE_HASH_WRITE_BUFFER_SIZE`]
    pub async fn flush_if_needed(&mut self) -> Result<(), SyncError> {
        if self.buffer.len() >= CODE_HASH_WRITE_BUFFER_SIZE {
            self.check_previous_task().await?;

            let buffer = std::mem::take(&mut self.buffer);
            self.flush_buffer(buffer);
        }
        Ok(())
    }

    /// Sync variant of [`Self::flush_if_needed`], callable from iterator
    /// closures (the sorted trie build). Skips awaiting the previous disk
    /// write: the JoinSet accumulates the handful of in-flight dump tasks
    /// instead, and their results — including write errors — are collected
    /// by [`Self::finish`].
    #[cfg(feature = "rocksdb")]
    pub fn flush_if_needed_sync(&mut self) {
        if self.buffer.len() >= CODE_HASH_WRITE_BUFFER_SIZE {
            let buffer = std::mem::take(&mut self.buffer);
            self.flush_buffer(buffer);
        }
    }

    /// Finishes the code collector and returns the final index of file
    pub async fn finish(mut self) -> Result<(), SyncError> {
        // Final flush if needed
        if !self.buffer.is_empty() {
            let buffer = std::mem::take(&mut self.buffer);
            self.flush_buffer(buffer);
        }

        // Wait for all pending writes
        self.disk_tasks
            .join_all()
            .await
            .into_iter()
            .map(|result| {
                result.inspect_err(|err| {
                    error!("Failed final write for code hashes: {err:?}");
                })
            })
            .collect::<Result<Vec<()>, DumpError>>()
            .map_err(|_| SyncError::BytecodeFileError)?;

        Ok(())
    }

    /// Flushes the given buffer to a file
    fn flush_buffer(&mut self, buffer: HashSet<H256>) {
        let file_name = get_code_hashes_snapshot_file(&self.snapshots_dir, self.file_index);
        let encoded = buffer.into_iter().collect::<Vec<_>>().encode_to_vec();
        let notify_tx = self.notify_tx.clone();
        self.disk_tasks.spawn(async move {
            dump_to_file(&file_name, encoded)?;
            // A failed send means the fetcher already exited; its error
            // surfaces at the join point, the unread file is harmless.
            let _ = notify_tx.send(file_name);
            Ok(())
        });
        self.file_index += 1;
    }

    /// Check possible errors from the previous task
    async fn check_previous_task(&mut self) -> Result<(), SyncError> {
        if let Some(task) = self.disk_tasks.join_next().await {
            task?
                .inspect_err(|err| error!("Error when dumping code hashes to file: {err:?}"))
                .map_err(|_| SyncError::BytecodeFileError)?;
        }
        Ok(())
    }
}
