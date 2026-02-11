use crate::peer_handler::DumpError;
use crate::snap::constants::CODE_HASH_WRITE_BUFFER_SIZE;
use crate::sync::SyncError;
use crate::utils::{dump_to_file, get_code_hashes_snapshot_file};
use ethrex_common::H256;
use ethrex_rlp::encode::RLPEncode;
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::sync::mpsc;
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
    // Optional channel to stream code hashes for concurrent bytecode downloading
    code_hash_sender: Option<mpsc::UnboundedSender<Vec<H256>>>,
}

impl CodeHashCollector {
    /// Creates a new code collector
    pub fn new(snapshots_dir: PathBuf) -> Self {
        Self {
            buffer: HashSet::new(),
            snapshots_dir,
            file_index: 0,
            disk_tasks: JoinSet::new(),
            code_hash_sender: None,
        }
    }

    /// Creates the channel and returns the receiver for concurrent bytecode downloading.
    /// Must be called before any code hashes are added.
    pub fn take_receiver(&mut self) -> mpsc::UnboundedReceiver<Vec<H256>> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.code_hash_sender = Some(tx);
        rx
    }

    /// Adds a code hash to the buffer and sends it through the channel if present
    pub fn add(&mut self, hash: H256) {
        if self.buffer.insert(hash) && let Some(sender) = &self.code_hash_sender {
            // Ignore send errors — the receiver may have been dropped if
            // the bytecode task finished early or errored out
            let _ = sender.send(vec![hash]);
        }
    }

    // The optimization for rocksdb database doesn't use this method
    #[cfg(not(feature = "rocksdb"))]
    /// Extends the buffer with a list of code hashes
    pub fn extend(&mut self, hashes: impl IntoIterator<Item = H256>) {
        let new_hashes: Vec<H256> = hashes
            .into_iter()
            .filter(|h| self.buffer.insert(*h))
            .collect();
        if !new_hashes.is_empty() && let Some(sender) = &self.code_hash_sender {
            let _ = sender.send(new_hashes);
        }
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

    /// Finishes the code collector: flushes remaining hashes and closes the channel
    pub async fn finish(mut self) -> Result<(), SyncError> {
        // Final flush if needed
        if !self.buffer.is_empty() {
            let buffer = std::mem::take(&mut self.buffer);
            self.flush_buffer(buffer);
        }

        // Drop the sender to close the channel, signaling the bytecode task to drain
        self.code_hash_sender.take();

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
        self.disk_tasks
            .spawn(async move { dump_to_file(&file_name, encoded) });
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
