//! This module contains the logic for bytecode download during snap sync
//! It works like a queue, waiting for the state sync & healing processes to advertise downloaded accounts' code hashes
//! Each code hash will be queued and fetched in batches
//! Bytecodes are not tied to a block so this process will not be affected by pivot staleness
//! The fetcher will remain active and listening until a termination signal (an empty batch) is received

use ethrex_common::H256;
use ethrex_storage::{error::StoreError, Store};
use tokio::sync::mpsc::Receiver;
use tracing::{debug, info};

use crate::{peer_handler::PeerHandler, sync::MAX_PARALLEL_FETCHES};

use super::{SyncError, BATCH_SIZE, BYTECODE_BATCH_SIZE};

/// Waits for incoming code hashes from the receiver channel endpoint, queues them, and fetches and stores their bytecodes in batches
pub(crate) async fn bytecode_fetcher(
    mut receiver: Receiver<Vec<H256>>,
    peers: PeerHandler,
    store: Store,
) -> Result<(), SyncError> {
    let mut pending_bytecodes: Vec<H256> = vec![];
    let mut incoming = true;
    while incoming {
        // Fetch incoming requests
        match receiver.recv().await {
            Some(code_hashes) if !code_hashes.is_empty() => {
                pending_bytecodes.extend(code_hashes);
            }
            // Disconnect / Empty message signaling no more bytecodes to sync
            _ => incoming = false,
        }
        // If we have enough pending bytecodes to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        while pending_bytecodes.len() >= BYTECODE_BATCH_SIZE || !incoming && !pending_bytecodes.is_empty() {
            // We will be spawning multiple tasks and then collecting their results
            // This uses a loop inside the main loop as the result from these tasks may lead to more values in queue
            let mut bytecode_tasks = tokio::task::JoinSet::new();
            info!("Spawning bytecode tasks");
            let instant = tokio::time::Instant::now();
            for n in 0..MAX_PARALLEL_FETCHES {
                info!("Spawning bytecode task n");
                let next_batch = pending_bytecodes
                    .drain(..BYTECODE_BATCH_SIZE.min(pending_bytecodes.len()))
                    .collect::<Vec<_>>();
                bytecode_tasks.spawn(fetch_bytecode_batch(
                    next_batch,
                    peers.clone(),
                    store.clone(),
                ));
                // End loop if we don't have enough elements to fill up a batch
                if pending_bytecodes.is_empty()
                    || (incoming && pending_bytecodes.len() < BYTECODE_BATCH_SIZE)
                {
                    break;
                }
            }
            info!(
                "Completed bytecode tasks in {} miliseconds",
                instant.elapsed().as_millis()
            );
            // Add unfetched bytecodes back to the queue
            for remaining in bytecode_tasks.join_all().await {
                pending_bytecodes.extend(remaining?);
            }
            info!(
                "{} pending bytecodes after fetch cycle",
                pending_bytecodes.len()
            )
        }
    }
    Ok(())
}

/// Receives a batch of code hahses, fetches their respective bytecodes via p2p and returns a list of the code hashes that couldn't be fetched in the request (if applicable)
async fn fetch_bytecode_batch(
    mut batch: Vec<H256>,
    peers: PeerHandler,
    store: Store,
) -> Result<Vec<H256>, StoreError> {
    if let Some(bytecodes) = peers.request_bytecodes(batch.clone()).await {
        debug!("Received {} bytecodes", bytecodes.len());
        // Store the bytecodes
        for code in bytecodes.into_iter() {
            store.add_account_code(batch.remove(0), code)?;
        }
    }
    // Return remaining code hashes in the batch if we couldn't fetch all of them
    Ok(batch)
}
