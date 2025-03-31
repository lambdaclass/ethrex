//! This module contains the logic for storage range downloads during state sync
//! It works like a queue, waiting for the state sync to advertise newly downloaded accounts with non-empty storages
//! Each storage will be queued and fetch in batches, once a storage is fully fetched it is then advertised to the storage rebuilder
//! Each downloaded storage will be written to the storage snapshot in the DB
//! If the pivot becomes stale while there are still pending storages in queue these will be sent to the storage healer
//! Even if the pivot becomes stale, the fetcher will remain active and listening until a termination signal (an empty batch) is received
//! Potential Improvements: Currenlty, we have a specific method to handle large storage tries (aka storage tries that don't fit into a single storage range request).
//! This method is called while fetching a storage batch and can stall the fetching of other smaller storages.
//! Large storage handling could be moved to its own separate queue process so that it runs parallel to regular storage fetching

use ethrex_common::{H256, U256};
use ethrex_storage::Store;
use ethrex_trie::Nibbles;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::debug;

use crate::{
    peer_handler::PeerHandler,
    sync::{BATCH_SIZE, MAX_CHANNEL_READS, MAX_PARALLEL_FETCHES},
};

use super::SyncError;

/// Waits for incoming account hashes & storage roots from the receiver channel endpoint, queues them, and fetches and stores their bytecodes in batches
/// This function will remain active until either an empty vec is sent to the receiver or the pivot becomes stale
/// Upon finsih, remaining storages will be sent to the storage healer
pub(crate) async fn storage_fetcher(
    mut receiver: Receiver<Vec<(H256, H256)>>,
    peers: PeerHandler,
    store: Store,
    state_root: H256,
    storage_trie_rebuilder_sender: Sender<Vec<(H256, H256)>>,
) -> Result<(), SyncError> {
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
                    // Empty message signaling no more bytecodes to sync
                    incoming = false
                }
            }
        } else {
            // Disconnect
            incoming = false
        }
        // If we have enough pending bytecodes to fill a batch
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
        store.set_storage_heal_paths(
            pending_storage
                .into_iter()
                .map(|(hash, _)| (hash, vec![Nibbles::default()]))
                .collect(),
        )?;
    }
    Ok(())
}

/// Receives a batch of account hashes with their storage roots, fetches their respective storage ranges via p2p and returns a list of the code hashes that couldn't be fetched in the request (if applicable)
/// Also returns a boolean indicating if the pivot became stale during the request
async fn fetch_storage_batch(
    mut batch: Vec<(H256, H256)>,
    state_root: H256,
    peers: PeerHandler,
    store: Store,
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
                if handle_large_storage_range(
                    state_root,
                    account_hash,
                    storage_root,
                    last_keys,
                    last_values,
                    peers.clone(),
                    store.clone(),
                )
                .await?
                {
                    // Pivot became stale
                    // Add trie back to the queue and return stale pivot status
                    batch.push((account_hash, storage_root));
                    return Ok((batch, true));
                }
                // Add large storage to completed storages
                complete_storages.push((account_hash, storage_root));
            }
            // The incomplete range is not the first, we cannot asume it is a large trie, so lets add it back to the queue
        }
        // Store the storage ranges & rebuild the storage trie for each account
        let filled_storages: Vec<(H256, H256)> = batch.drain(..values.len()).collect();
        let account_hashes: Vec<H256> = filled_storages.iter().map(|(hash, _)| *hash).collect();
        complete_storages.extend(filled_storages);
        store.write_snapshot_storage_batches(account_hashes, keys, values)?;
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

/// Handles the returned incomplete storage range of a large storage trie and
/// fetches the rest of the trie using single requests
/// Returns a boolean indicating is the pivot became stale during fetching
// TODO: Later on this method can be refactored to use a separate queue process
// instead of blocking the current thread for the remainder of the retrieval
async fn handle_large_storage_range(
    state_root: H256,
    account_hash: H256,
    storage_root: H256,
    keys: Vec<H256>,
    values: Vec<U256>,
    peers: PeerHandler,
    store: Store,
) -> Result<bool, SyncError> {
    // First process the initial range
    // Keep hold of the last key as this will be the first key of the next range
    let mut next_key = *keys.last().unwrap();
    store.write_snapshot_storage_batch(account_hash, keys, values)?;
    let mut should_continue = true;
    // Fetch the remaining range
    while should_continue {
        debug!("Fetching large storage trie, current key: {}", next_key);

        if let Some((keys, values, incomplete)) = peers
            .request_storage_range(state_root, storage_root, account_hash, next_key)
            .await
        {
            next_key = *keys.last().unwrap();
            should_continue = incomplete;
            store.write_snapshot_storage_batch(account_hash, keys, values)?;
        } else {
            return Ok(true);
        }
    }
    Ok(false)
}
