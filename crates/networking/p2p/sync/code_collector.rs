use crate::peer_handler::DumpError;
use crate::sync::SyncError;
use crate::utils::{dump_to_file, get_code_hashes_snapshot_file};
use ethrex_common::H256;
use ethrex_rlp::encode::RLPEncode;
use std::collections::HashSet;

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
    // Sender to send code hash dump results
    sender: tokio::sync::mpsc::Sender<Result<(), DumpError>>,
    // Receiver to receive code hash dump results
    receiver: tokio::sync::mpsc::Receiver<Result<(), DumpError>>,
}

impl CodeHashCollector {
    /// Creates a new code collector
    pub fn new(initial_index: u64, snapshots_dir: String) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(100);
        Self {
            buffer: HashSet::new(),
            snapshots_dir,
            file_index: initial_index,
            sender,
            receiver,
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

    /// Handles errors from the receiver when flushing the buffer to a file
    /// and retries the dump to the file
    pub async fn handle_errors(&mut self) -> Result<(), SyncError> {
        if let Ok(Err(dump_error)) = self.receiver.try_recv() {
            if dump_error.error == std::io::ErrorKind::StorageFull {
                return Err(SyncError::BytecodeFileError);
            }
            let sender_clone = self.sender.clone();
            tokio::task::spawn(async move {
                let result = dump_to_file(dump_error.path, dump_error.contents);
                sender_clone.send(result).await.ok();
            });
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

        // Wait for all pending writes
        drop(self.sender);
        while let Some(result) = self.receiver.recv().await {
            if let Err(dump_error) = result {
                if dump_error.error == std::io::ErrorKind::StorageFull {
                    return Err(SyncError::BytecodeFileError);
                }
                dump_to_file(dump_error.path, dump_error.contents)
                    .inspect_err(|err| {
                        tracing::error!("Failed final retry for bytecode dump: {:?}", err);
                    })
                    .map_err(|_| SyncError::BytecodeFileError)?;
            }
        }

        Ok(self.file_index)
    }

    /// Flushes the given buffer to a file
    fn flush_buffer(&mut self, buffer: HashSet<H256>) {
        let (encoded_buffer, file_name) =
            prepare_bytecode_buffer_for_dump(buffer, self.file_index, self.snapshots_dir.clone());

        let sender_clone = self.sender.clone();
        tokio::task::spawn(async move {
            let result = dump_to_file(file_name, encoded_buffer);
            sender_clone.send(result).await.ok();
        });
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
