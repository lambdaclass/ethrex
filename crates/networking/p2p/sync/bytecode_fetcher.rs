//! This module contains the logic for bytecode download during snap sync
//! It works like a queue, waiting for the state sync & healing processes to advertise downloaded accounts' code hashes
//! Each code hash will be queued and fetched in batches
//! Bytecodes are not tied to a block so this process will not be affected by pivot staleness
//! The fetcher will remain active and listening until a termination signal (an empty batch) is received

use ethrex_common::H256;
use ethrex_storage::Store;
use tokio::sync::mpsc::Receiver;
use tracing::debug;

use crate::peer_handler::PeerHandler;

use super::{BYTECODE_BATCH_SIZE, SyncError, fetcher_queue::run_queue};

/// Waits for incoming code hashes from the receiver channel endpoint, queues them, and fetches and stores their bytecodes in batches
pub(crate) async fn bytecode_fetcher(
    mut receiver: Receiver<Vec<H256>>,
    peers: PeerHandler,
    store: Store,
) -> Result<(), SyncError> {
    let mut pending_bytecodes: Vec<H256> = vec![];
    let fetch_batch = move |batch: Vec<H256>, peers: PeerHandler, store: Store| async {
        // Bytecode fetcher will never become stale
        fetch_bytecode_batch(batch, peers, store)
            .await
            .map(|res| (res, false))
    };
    run_queue(
        &mut receiver,
        &mut pending_bytecodes,
        &fetch_batch,
        peers,
        store,
        BYTECODE_BATCH_SIZE,
    )
    .await?;
    Ok(())
}

/// Receives a batch of code hahses, fetches their respective bytecodes via p2p and returns a list of the code hashes that couldn't be fetched in the request (if applicable)
async fn fetch_bytecode_batch(
    mut batch: Vec<H256>,
    peers: PeerHandler,
    store: Store,
) -> Result<Vec<H256>, SyncError> {
    if let Some(bytecodes) = peers.request_bytecodes(batch.clone()).await {
        debug!("Received {} bytecodes", bytecodes.len());
        // Store the bytecodes
        for code in bytecodes.into_iter() {
            store.add_account_code(batch.remove(0), code).await?;
        }
    }
    // Return remaining code hashes in the batch if we couldn't fetch all of them
    Ok(batch)
}
