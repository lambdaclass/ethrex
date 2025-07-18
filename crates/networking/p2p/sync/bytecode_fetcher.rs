//! This module contains the logic for bytecode download during snap sync
//! It works like a queue, waiting for the state sync & healing processes to advertise downloaded accounts' code hashes
//! Each code hash will be queued and fetched in batches
//! Bytecodes are not tied to a block so this process will not be affected by pivot staleness
//! The fetcher will remain active and listening until a termination signal (an empty batch) is received

use std::time::Instant;

use ethrex_common::H256;
use ethrex_storage::Store;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::peer_handler::PeerHandler;

use super::{
    BYTECODE_BATCH_SIZE, MAX_CHANNEL_MESSAGES, SHOW_PROGRESS_INTERVAL_DURATION, SyncError,
    fetcher_queue::{read_incoming_requests, spawn_fetch_tasks},
};

/// Represents the permanently ongoing background trie rebuild process
/// This process will be started whenever a state sync is initiated and will be
/// kept alive throughout sync cycles, only stopping once the tries are fully rebuilt or the node is stopped
#[derive(Debug)]
pub(crate) struct BytecodeFetcher {
    task: tokio::task::JoinHandle<Result<(), SyncError>>,
    pub(crate) sender: Sender<Vec<H256>>,
}

impl BytecodeFetcher {
    /// Returns true is the trie rebuild porcess is alive and well
    pub fn alive(&self) -> bool {
        !(self.task.is_finished() || self.sender.is_closed())
    }
    /// Waits for the rebuild process to complete and returns the resulting mismatched accounts
    pub async fn complete(self) -> Result<(), SyncError> {
        // Signal storage rebuilder to finish
        self.sender.send(vec![]).await?;
        self.task.await?
    }

    /// starts the background trie rebuild process
    pub fn startup(cancel_token: CancellationToken, store: Store, peers: PeerHandler) -> Self {
        let (sender, receiver) = channel::<Vec<H256>>(MAX_CHANNEL_MESSAGES);
        let task = tokio::task::spawn(bytecode_fetcher(
            receiver,
            peers,
            store.clone(),
            cancel_token.clone(),
        ));
        Self { task, sender }
    }
}

/// Waits for incoming code hashes from the receiver channel endpoint, queues them, and fetches and stores their bytecodes in batches
async fn bytecode_fetcher(
    mut receiver: Receiver<Vec<H256>>,
    peers: PeerHandler,
    store: Store,
    cancel_token: CancellationToken,
) -> Result<(), SyncError> {
    let mut pending_bytecodes: Vec<H256> = vec![];
    let fetch_batch = move |batch: Vec<H256>, peers: PeerHandler, store: Store| async {
        // Bytecode fetcher will never become stale
        fetch_bytecode_batch(batch, peers, store)
            .await
            .map(|res| (res, false))
    };
    let mut last_update = Instant::now();
    // The pivot may become stale while the fetcher is active, we will still keep the process
    // alive until the end signal so we don't lose incoming messages
    let mut incoming = true;
    while incoming || !pending_bytecodes.is_empty() {
        if last_update.elapsed() >= SHOW_PROGRESS_INTERVAL_DURATION {
            last_update = Instant::now();
            info!(
                "Bytecode Fetching in Progress, queuded: {}",
                pending_bytecodes.len(),
            );
        }
        if cancel_token.is_cancelled() {
            // TODO: store them in DB
            tracing::warn!(
                "Bytecode fetcher cancelled with {} in queue",
                pending_bytecodes.len()
            );
        }
        // Read incoming messages and add them to the queue
        incoming = read_incoming_requests(&mut receiver, &mut pending_bytecodes).await;
        spawn_fetch_tasks(
            &mut pending_bytecodes,
            incoming,
            &fetch_batch,
            peers.clone(),
            store.clone(),
            BYTECODE_BATCH_SIZE,
        )
        .await?;
    }
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
