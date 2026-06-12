//! Streaming bytecode downloader.
//!
//! Bytecodes are content-addressed (each response is keccak-verified against
//! the requested hash) and carry no dependency on the pivot or on healing, so
//! they can download concurrently with every other sync phase. This module
//! consumes the code-hash snapshot files as the collector finishes writing
//! them — during account insertion and healing — instead of waiting for a
//! dedicated phase after healing completes. The files remain the hand-off
//! medium so the on-disk format and restart behavior are unchanged.

use crate::metrics::METRICS;
use crate::peer_handler::PeerHandler;
use crate::snap::{async_fs, constants::BYTECODE_CHUNK_SIZE, request_bytecodes};
use crate::sync::SyncError;
use ethrex_common::{H256, types::Code};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::Store;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use tracing::debug;

/// Spawns the background fetcher. Send each finished code-hash snapshot file
/// over the channel; drop the sender (the collector's `finish`) to signal
/// that no more files are coming, then await the handle to drain the final
/// partial batch and surface any download/store error.
pub(crate) fn spawn_bytecode_fetcher(
    peers: PeerHandler,
    store: Store,
    file_rx: UnboundedReceiver<PathBuf>,
) -> JoinHandle<Result<(), SyncError>> {
    tokio::spawn(run(peers, store, file_rx))
}

async fn run(
    mut peers: PeerHandler,
    store: Store,
    mut file_rx: UnboundedReceiver<PathBuf>,
) -> Result<(), SyncError> {
    let mut seen_code_hashes: HashSet<H256> = HashSet::new();
    let mut to_download: Vec<H256> = Vec::new();
    let mut started = false;

    while let Some(file_path) = file_rx.recv().await {
        if !started {
            started = true;
            *METRICS.bytecode_download_start_time.lock().await = Some(SystemTime::now());
        }
        let snapshot_contents = async_fs::read_file(&file_path).await?;
        let code_hashes: Vec<H256> = RLPDecode::decode(&snapshot_contents)
            .map_err(|_| SyncError::CodeHashesSnapshotDecodeError(file_path))?;

        for hash in code_hashes {
            if seen_code_hashes.insert(hash) {
                to_download.push(hash);
                if to_download.len() >= BYTECODE_CHUNK_SIZE {
                    download_batch(&mut peers, &store, &mut to_download).await?;
                }
            }
        }
    }

    // Channel closed: the collector finished and every file was processed.
    if !to_download.is_empty() {
        download_batch(&mut peers, &store, &mut to_download).await?;
    }
    Ok(())
}

async fn download_batch(
    peers: &mut PeerHandler,
    store: &Store,
    to_download: &mut Vec<H256>,
) -> Result<(), SyncError> {
    debug!("Starting bytecode download of {} hashes", to_download.len());
    let bytecodes = request_bytecodes(peers, to_download)
        .await?
        .ok_or(SyncError::BytecodesNotFound)?;
    store
        .write_account_code_batch(
            to_download
                .drain(..)
                .zip(bytecodes)
                // SAFETY: hash already checked by the download worker
                .map(|(hash, code)| (hash, Code::from_bytecode_unchecked(code, hash)))
                .collect(),
        )
        .await?;
    Ok(())
}
