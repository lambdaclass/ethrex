use crate::peer_handler::DumpError;
use crate::sync::SyncError;
use crate::utils::{dump_to_file, get_code_hashes_snapshot_file};
use ethrex_common::H256;
use ethrex_rlp::encode::RLPEncode;
use std::collections::HashSet;
use tokio::task::JoinSet;
use tracing::error;

/// Size of the buffer to store code hashes before flushing to a file
const CODE_HASH_WRITE_BUFFER_SIZE: usize = 100_000;

/// Manages code hash collection and async file writing
pub struct CodeHashCollector {
    // Buffer to store code hashes
    buffer: HashSet<H256>,
    // Directory to store code hashes
    snapshots_dir: String,
    // Index of the current code hash file
    file_index: u64,
    // JoinSet to manage async disk writes
    disk_tasks: JoinSet<Result<(), DumpError>>,
}

impl CodeHashCollector {
    /// Creates a new code collector
    pub fn new(snapshots_dir: String) -> Self {
        Self {
            buffer: HashSet::new(),
            snapshots_dir,
            file_index: 0,
            disk_tasks: JoinSet::new(),
        }
    }

    /// Adds a code hash to the buffer
    pub fn add(&mut self, hash: H256) {
        self.buffer.insert(hash);
    }

    /// Extends the buffer with a list of code hashes
    pub fn extend(&mut self, hashes: impl IntoIterator<Item = H256>) {
        self.buffer.extend(hashes);
    }

    /// Flushes the buffer to a file if the buffer is larger than [`CODE_HASH_WRITE_BUFFER_SIZE`]
    pub async fn flush_if_needed(&mut self) -> Result<(), SyncError> {
        if self.buffer.len() >= CODE_HASH_WRITE_BUFFER_SIZE {
            let buffer = std::mem::take(&mut self.buffer);
            self.flush_buffer(buffer);
        }
        Ok(())
    }

    /// Handles completed disk write tasks, terminating on any error
    pub async fn handle_errors(&mut self) -> Result<(), SyncError> {
        while let Some(result) = self.disk_tasks.try_join_next() {
            result
                .expect("Shouldn't have a join error")
                .inspect_err(|err| error!("We found this error while dumping to file {err:?}"))
                .map_err(|_| SyncError::BytecodeFileError)?;
        }
        Ok(())
    }

    /// Finishes the code collector and returns the final index of file
    pub async fn finish(mut self) -> Result<u64, SyncError> {
        // Final flush if needed
        if !self.buffer.is_empty() {
            let buffer = std::mem::take(&mut self.buffer);
            self.flush_buffer(buffer);
        }

        // Wait for all pending writes using join_all pattern from peer_handler
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

        Ok(self.file_index)
    }

    /// Flushes the given buffer to a file
    fn flush_buffer(&mut self, buffer: HashSet<H256>) {
        let (encoded_buffer, file_name) =
            prepare_bytecode_buffer_for_dump(buffer, self.file_index, self.snapshots_dir.clone());

        self.disk_tasks
            .spawn(async move { dump_to_file(file_name, encoded_buffer) });
        self.file_index += 1;
    }
}

/// Encode code hashes to a vector
fn prepare_bytecode_buffer_for_dump(
    buffer: HashSet<H256>,
    file_index: u64,
    dir: String,
) -> (Vec<u8>, String) {
    let mut sorted_buffer: Vec<H256> = buffer.into_iter().collect();
    sorted_buffer.sort();
    let encoded = sorted_buffer.encode_to_vec();
    let filename = get_code_hashes_snapshot_file(dir, file_index);
    (encoded, filename)
}
