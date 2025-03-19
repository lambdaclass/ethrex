//! This module contains the logic for storage range downloads during state sync
//! It works like a queue, waiting for the state sync to advertise newly downloaded accounts with non-empty storages
//! Each storage will be queued and fetch in batches, once a storage is fully fetched it is then advertised to the storage rebuilder
//! Each downloaded storage will be written to the storage snapshot in the DB
//! If the pivot becomes stale while there are still pending storages in queue these will be sent to the storage healer
//! Even if the pivot becomes stale, the fetcher will remain active and listening until a termination signal (an empty batch) is received
//! Potential Improvements: Currenlty, we have a specific method to handle large storage tries (aka storage tries that don't fit into a single storage range request).
//! This method is called while fetching a storage batch and can stall the fetching of other smaller storages.
//! Large storage handling could be moved to its own separate queue process so that it runs parallel to regular storage fetching

use ethrex_common::H256;
use ethrex_storage::Store;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tracing::{debug, info};

use crate::{
    peer_handler::PeerHandler,
    sync::{
        trie_rebuild::REBUILDER_INCOMPLETE_STORAGE_ROOT, BATCH_SIZE, MAX_CHANNEL_MESSAGES,
        MAX_CHANNEL_READS, MAX_PARALLEL_FETCHES,
    },
};

use super::SyncError;

/// Waits for incoming account hashes & storage roots from the receiver channel endpoint, queues them, and fetches and stores their storages in batches
/// This function will remain active until either an empty vec is sent to the receiver or the pivot becomes stale
/// Upon finsih, remaining storages will be sent to the storage healer
pub(crate) async fn storage_fetcher(
    mut receiver: Receiver<Vec<(H256, H256)>>,
    peers: PeerHandler,
    store: Store,
    state_root: H256,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
    storage_healer_sender: Sender<Vec<H256>>,
) -> Result<(), SyncError> {
    // Spawn large storage fetcher
    let (large_storage_sender, large_storage_receiver) =
        channel::<Vec<(H256, H256, H256)>>(MAX_CHANNEL_MESSAGES);
    let large_storage_fetcher_handler = tokio::spawn(large_storage_fetcher(
        large_storage_receiver,
        peers.clone(),
        store.clone(),
        state_root,
        storage_trie_rebuilder_sender.clone(),
        storage_healer_sender.clone(),
    ));
    // Pending list of storages to fetch
    let mut pending_storage: Vec<(H256, H256)> = vec![];
    // The pivot may become stale while the fetcher is active, we will still keep the process
    // alive until the end signal so we don't lose queued messages
    let mut stale = false;
    let mut incoming = true;
    while incoming {
        // Fetch incoming requests
        let mut msg_buffer = vec![];
        if receiver.recv_many(&mut msg_buffer, MAX_CHANNEL_READS).await != 0 {
            for account_hashes_and_roots in msg_buffer {
                if !account_hashes_and_roots.is_empty() {
                    pending_storage.extend(account_hashes_and_roots);
                } else {
                    // Empty message signaling no more storages to sync
                    incoming = false
                }
            }
        } else {
            // Disconnect
            incoming = false
        }
        // If we have enough pending storages to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        // If the pivot became stale don't process anything and just save incoming requests
        while !stale
            && (pending_storage.len() >= BATCH_SIZE || (!incoming && !pending_storage.is_empty()))
        {
            // We will be spawning multiple tasks and then collecting their results
            // This uses a loop inside the main loop as the result from these tasks may lead to more values in queue
            let mut storage_tasks = tokio::task::JoinSet::new();
            for _ in 0..MAX_PARALLEL_FETCHES {
                let next_batch = pending_storage
                    .drain(..BATCH_SIZE.min(pending_storage.len()))
                    .collect::<Vec<_>>();
                storage_tasks.spawn(fetch_storage_batch(
                    next_batch,
                    state_root,
                    peers.clone(),
                    store.clone(),
                    large_storage_sender.clone(),
                    storage_trie_rebuilder_sender.clone(),
                ));
                // End loop if we don't have enough elements to fill up a batch
                if pending_storage.is_empty() || (incoming && pending_storage.len() < BATCH_SIZE) {
                    break;
                }
            }
            // Add unfetched accounts to queue and handle stale signal
            for res in storage_tasks.join_all().await {
                let (remaining, is_stale) = res?;
                pending_storage.extend(remaining);
                stale |= is_stale;
            }
        }
    }
    debug!(
        "Concluding storage fetcher, {} storages left in queue to be healed later",
        pending_storage.len()
    );
    if !pending_storage.is_empty() {
        storage_healer_sender
            .send(pending_storage.into_iter().map(|(hash, _)| hash).collect())
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
    large_storage_sender: Sender<Vec<(H256, H256, H256)>>,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
) -> Result<(Vec<(H256, H256)>, bool), SyncError> {
    // A list of all completely fetched storages to send to the rebuilder
    let mut complete_storages = vec![];
    debug!(
        "Requesting storage ranges for addresses {}..{}",
        batch.first().unwrap().0,
        batch.last().unwrap().0
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
            let (last_keys, last_values) = (keys.pop().unwrap(), values.pop().unwrap());
            // If only one incomplete range is returned then it must belong to a trie that is too big to fit into one request
            // We will handle this large trie separately
            if keys.is_empty() {
                debug!("Large storage trie encountered, handling separately");
                let (account_hash, storage_root) = batch.remove(0);
                let last_key = *last_keys.last().unwrap();
                // Store downloaded range
                store.write_snapshot_storage_batch(account_hash, last_keys, last_values)?;
                // Delegate the rest of the trie to the large trie fetcher
                large_storage_sender
                    .send(vec![(account_hash, storage_root, last_key)])
                    .await?;
                return Ok((batch, false));
            }
            // The incomplete range is not the first, we cannot asume it is a large trie, so lets add it back to the queue
        }
        // Store the storage ranges & rebuild the storage trie for each account
        for (keys, values) in keys.into_iter().zip(values.into_iter()) {
            let (account_hash, expected_root) = batch.remove(0);
            // Write storage to snapshot
            store.write_snapshot_storage_batch(account_hash, keys, values)?;
            complete_storages.push((account_hash, expected_root));
        }
        // Send complete storages to the rebuilder
        storage_trie_rebuilder_sender
            .send(complete_storages)
            .await?;
        // Return remaining code hashes in the batch if we couldn't fetch all of them
        return Ok((batch, false));
    }
    // Pivot became stale
    Ok((batch, true))
}

/// Waits for incoming account hashes, storage roots & paths from the receiver channel endpoint, queues them, and fetches and stores their storage ranges in batches
/// This function will remain active until either an empty vec is sent to the receiver or the pivot becomes stale
/// Upon finsih, remaining storages will be sent to the storage healer
pub(crate) async fn large_storage_fetcher(
    mut receiver: Receiver<Vec<(H256, H256, H256)>>,
    peers: PeerHandler,
    store: Store,
    state_root: H256,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
    storage_healer_sender: Sender<Vec<H256>>,
) -> Result<(), SyncError> {
    // Pending list of storages to fetch
    // (account_hash, storage_root, last_key)
    let mut pending_storage: Vec<(H256, H256, H256)> = vec![];
    // The pivot may become stale while the fetcher is active, we will still keep the process
    // alive until the end signal so we don't lose queued messages
    let mut stale = false;
    let mut incoming = true;
    while incoming || !pending_storage.is_empty() {
        // Fetch incoming requests
        if !receiver.is_empty() || pending_storage.is_empty() {
            let mut msg_buffer = vec![];
            if receiver.recv_many(&mut msg_buffer, MAX_CHANNEL_READS).await != 0 {
                for hashes_roots_keys in msg_buffer {
                    if !hashes_roots_keys.is_empty() {
                        pending_storage.extend(hashes_roots_keys);
                    } else {
                        // Empty message signaling no more storages to sync
                        incoming = false
                    }
                }
            } else {
                // Disconnect
                incoming = false
            }
        }
        // If we have enough pending bytecodes to fill a batch
        // or if we have no more incoming batches, spawn a fetch process
        // If the pivot became stale don't process anything and just save incoming requests
        while !stale && !pending_storage.is_empty() {
            // We will be spawning multiple tasks and then collecting their results
            // This uses a loop inside the main loop as the result from these tasks may lead to more values in queue
            let mut storage_tasks = tokio::task::JoinSet::new();
            for batch in pending_storage.iter().take(MAX_PARALLEL_FETCHES) {
                storage_tasks.spawn(fetch_large_storage_batch(
                    *batch,
                    state_root,
                    peers.clone(),
                    store.clone(),
                    storage_trie_rebuilder_sender.clone(),
                ));
            }
            // Add unfetched storages to the queue and handle stale signal
            for res in storage_tasks.join_all().await {
                let (next_batch, is_stale) = res?;
                if let Some(next_batch) = next_batch {
                    pending_storage.push(next_batch)
                }
                stale |= is_stale;
            }
        }
    }
    info!(
        "Concluding large storage fetcher, {} large storages left in queue to be healed later",
        pending_storage.len()
    );
    if !pending_storage.is_empty() {
        // Send incomplete storages to the rebuilder and healer
        // As these are large storages we should rebuild the partial tries instead of delegating them fully to the healer
        let account_hashes: Vec<H256> = pending_storage
            .into_iter()
            .map(|(hash, _, _)| hash)
            .collect();
        let account_hashes_and_roots: Vec<(H256, H256)> = account_hashes
            .iter()
            .map(|hash| (*hash, REBUILDER_INCOMPLETE_STORAGE_ROOT))
            .collect();
        storage_healer_sender.send(account_hashes).await?;
        storage_trie_rebuilder_sender
            .send(account_hashes_and_roots)
            .await?;
    }
    Ok(())
}

// Receives a batch of account hashes with their storage roots, fetches their respective storage ranges via p2p and returns a list of the code hashes that couldn't be fetched in the request (if applicable)
/// Also returns a boolean indicating if the pivot became stale during the request
async fn fetch_large_storage_batch(
    // (acc_hash, storage_root, hash)
    mut batch: (H256, H256, H256),
    state_root: H256,
    peers: PeerHandler,
    store: Store,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
) -> Result<(Option<(H256, H256, H256)>, bool), SyncError> {
    info!(
        "Requesting large storage range for trie: {} from key: {}",
        batch.1, batch.2,
    );
    if let Some((keys, values, incomplete)) = peers
        .request_storage_range(state_root, batch.1, batch.0, batch.2)
        .await
    {
        // Update next batch's start
        batch.2 = *keys.last().unwrap();
        // Write storage range to snapshot
        store.write_snapshot_storage_batch(batch.0, keys, values)?;
        if incomplete {
            Ok((Some(batch), false))
        } else {
            // Send complete trie to rebuilder
            storage_trie_rebuilder_sender
                .send(vec![(batch.0, batch.1)])
                .await?;
            Ok((None, false))
        }
    } else {
        // Pivot became stale
        Ok((Some(batch), true))
    }
}
