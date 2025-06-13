//! This module contains the logic for storage range downloads during state sync
//! It works like a queue, waiting for the state sync to advertise newly downloaded accounts with non-empty storages
//! Each storage will be queued and fetch in batches, once a storage is fully fetched it is then advertised to the storage rebuilder
//! Each downloaded storage will be written to the storage snapshot in the DB
//! If a large storage is detected while fetching it will be delegated to a separate large storage fetcher process in order to not stall the rest of the storages
//! A large storage fetcher will exist and be supervised by each storage fetcher
//! If the pivot becomes stale while there are still pending storages in queue these will be sent to the storage healer
//! Even if the pivot becomes stale, the fetcher will remain active and listening until a termination signal (an empty batch) is received

use ethrex_common::H256;
use ethrex_storage::Store;
use ethrex_trie::Nibbles;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tracing::{debug, error};

use crate::{
    peer_handler::PeerHandler,
    sync::{
        MAX_CHANNEL_MESSAGES, STORAGE_BATCH_SIZE, fetcher_queue::run_queue,
        trie_rebuild::REBUILDER_INCOMPLETE_STORAGE_ROOT,
    },
};

use super::SyncError;

/// An in-progress large storage trie fetch request
struct LargeStorageRequest {
    account_hash: H256,
    storage_root: H256,
    last_key: H256,
}

/// Waits for incoming account hashes & storage roots from the receiver channel endpoint, queues them, and fetches and stores their storages in batches
/// This function will remain active until either an empty vec is sent to the receiver or the pivot becomes stale
/// Upon finish, remaining storages will be sent to the storage healer
pub(crate) async fn storage_fetcher(
    mut receiver: Receiver<Vec<(H256, H256)>>,
    peers: PeerHandler,
    store: Store,
    state_root: H256,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
) -> Result<(), SyncError> {
    // Spawn large storage fetcher
    let (large_storage_sender, large_storage_receiver) =
        channel::<Vec<LargeStorageRequest>>(MAX_CHANNEL_MESSAGES);
    let large_storage_fetcher_handler = tokio::spawn(large_storage_fetcher(
        large_storage_receiver,
        peers.clone(),
        store.clone(),
        state_root,
        storage_trie_rebuilder_sender.clone(),
    ));
    // Pending list of storages to fetch
    let mut pending_storage: Vec<(H256, H256)> = vec![];
    // Create an async closure to pass to the generic task spawner
    let fetch_batch = |batch: Vec<(H256, H256)>, peers: PeerHandler, store: Store| {
        let l_sender = large_storage_sender.clone();
        let s_sender = storage_trie_rebuilder_sender.clone();
        async move { fetch_storage_batch(batch, state_root, peers, store, l_sender, s_sender).await }
    };
    run_queue(
        &mut receiver,
        &mut pending_storage,
        &fetch_batch,
        peers,
        store.clone(),
        STORAGE_BATCH_SIZE,
    )
    .await?;
    debug!(
        "Concluding storage fetcher, {} storages left in queue to be healed later",
        pending_storage.len()
    );
    if !pending_storage.is_empty() {
        store
            .set_storage_heal_paths(
                pending_storage
                    .into_iter()
                    .map(|(hash, _)| (hash, vec![Nibbles::default()]))
                    .collect(),
            )
            .await?;
    }
    // Signal large storage fetcher
    large_storage_sender.send(vec![]).await?;
    large_storage_fetcher_handler.await?
}

/// Receives a batch of account hashes with their storage roots, fetches their respective storage ranges via p2p and returns a list of the code hashes that couldn't be fetched in the request (if applicable)
/// Also returns a boolean indicating if the pivot became stale during the request
async fn fetch_storage_batch(
    mut batch: Vec<(H256, H256)>,
    state_root: H256,
    peers: PeerHandler,
    store: Store,
    large_storage_sender: Sender<Vec<LargeStorageRequest>>,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
) -> Result<(Vec<(H256, H256)>, bool), SyncError> {
    let batch_first = batch.first().ok_or(SyncError::InvalidRangeReceived)?;
    let batch_last = batch.last().ok_or(SyncError::InvalidRangeReceived)?;
    debug!(
        "Requesting storage ranges for addresses {}..{}",
        batch_first.0, batch_last.0
    );

    let (batch_hahses, batch_roots) = batch.clone().into_iter().unzip();
    if let Some((mut keys, mut values, incomplete)) = peers
        .request_storage_ranges(state_root, batch_roots, batch_hahses, H256::zero())
        .await
    {
        debug!("Received {} storage ranges", keys.len(),);
        // Handle incomplete ranges
        if incomplete {
            // An incomplete range cannot be empty
            let last_keys = keys.pop().ok_or(SyncError::InvalidRangeReceived)?;
            let last_values = values.pop().ok_or(SyncError::InvalidRangeReceived)?;
            // If only one incomplete range is returned then it must belong to a trie that is too big to fit into one request
            // We will handle this large trie separately
            if keys.is_empty() {
                debug!("Large storage trie encountered, handling separately");
                let (account_hash, storage_root) = batch.remove(0);
                let lk = last_keys.last().ok_or(SyncError::InvalidRangeReceived)?;
                let last_key = *lk;
                // Store downloaded range
                store
                    .write_snapshot_storage_batch(account_hash, last_keys, last_values)
                    .await?;
                // Delegate the rest of the trie to the large trie fetcher
                large_storage_sender
                    .send(vec![LargeStorageRequest {
                        account_hash,
                        storage_root,
                        last_key,
                    }])
                    .await?;
                return Ok((batch, false));
            }
            // The incomplete range is not the first, we cannot asume it is a large trie, so lets add it back to the queue
        }
        // Store the storage ranges & rebuild the storage trie for each account
        let filled_storages: Vec<(H256, H256)> = batch.drain(..values.len()).collect();
        let account_hashes: Vec<H256> = filled_storages.iter().map(|(hash, _)| *hash).collect();
        store
            .write_snapshot_storage_batches(account_hashes, keys, values)
            .await?;
        // Send complete storages to the rebuilder
        storage_trie_rebuilder_sender.send(filled_storages).await?;
        // Return remaining code hashes in the batch if we couldn't fetch all of them
        return Ok((batch, false));
    }
    // Pivot became stale
    Ok((batch, true))
}

/// Waits for incoming large storage requests from the receiver channel endpoint, queues them, and fullfils them in parallel
/// This function will remain active until either an empty vec is sent to the receiver or the pivot becomes stale
/// Upon finish, remaining storages will be sent to the storage healer and storage rebuilder (so we can rebuild the partial tries)
async fn large_storage_fetcher(
    mut receiver: Receiver<Vec<LargeStorageRequest>>,
    peers: PeerHandler,
    store: Store,
    state_root: H256,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
) -> Result<(), SyncError> {
    // Pending list of storages to fetch
    // (account_hash, storage_root, last_key)
    let mut pending_storage: Vec<LargeStorageRequest> = vec![];
    // Create an async closure to pass to the generic task spawner
    let fetch_batch = |mut batch: Vec<LargeStorageRequest>, peers: PeerHandler, store: Store| {
        let s_sender = storage_trie_rebuilder_sender.clone();
        // Batch size should always be 1
        if batch.len() != 1 {
            error!("Invalid large storage batch size, check source code");
        }
        async move {
            fetch_large_storage(batch.remove(0), state_root, peers, store, s_sender)
                .await
                .map(|(rem, stale)| (rem.map(|r| vec![r]).unwrap_or_default(), stale))
        }
    };
    run_queue(
        &mut receiver,
        &mut pending_storage,
        &fetch_batch,
        peers,
        store.clone(),
        1,
    )
    .await?;
    debug!(
        "Concluding large storage fetcher, {} large storages left in queue to be healed later",
        pending_storage.len()
    );
    if !pending_storage.is_empty() {
        // Send incomplete storages to the rebuilder and healer
        // As these are large storages we should rebuild the partial tries instead of delegating them fully to the healer
        let heal_paths = pending_storage
            .iter()
            .map(|req| (req.account_hash, vec![Nibbles::default()]))
            .collect();
        let account_hashes_and_roots: Vec<(H256, H256)> = pending_storage
            .iter()
            .map(|req| (req.account_hash, REBUILDER_INCOMPLETE_STORAGE_ROOT))
            .collect();
        store.set_storage_heal_paths(heal_paths).await?;
        storage_trie_rebuilder_sender
            .send(account_hashes_and_roots)
            .await?;
    }
    Ok(())
}

// Receives a large storage request and attempts to fulfill it (by fetching the next storage range)
// Returns the updated request status or None if the request was fulfilled
/// Also returns a boolean indicating if the pivot became stale during the request
async fn fetch_large_storage(
    mut request: LargeStorageRequest,
    state_root: H256,
    peers: PeerHandler,
    store: Store,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
) -> Result<(Option<LargeStorageRequest>, bool), SyncError> {
    debug!(
        "Requesting large storage range for trie: {} from key: {}",
        request.storage_root, request.last_key,
    );
    if let Some((keys, values, incomplete)) = peers
        .request_storage_range(
            state_root,
            request.storage_root,
            request.account_hash,
            request.last_key,
        )
        .await
    {
        // Update next batch's start
        let last_key = keys
            .last()
            .ok_or(SyncError::Trie(ethrex_trie::TrieError::InconsistentTree))?;
        request.last_key = *last_key;
        // Write storage range to snapshot
        store
            .write_snapshot_storage_batch(request.account_hash, keys, values)
            .await?;
        if incomplete {
            Ok((Some(request), false))
        } else {
            // Send complete trie to rebuilder
            storage_trie_rebuilder_sender
                .send(vec![(request.account_hash, request.storage_root)])
                .await?;
            Ok((None, false))
        }
    } else {
        // Pivot became stale
        Ok((Some(request), true))
    }
}
