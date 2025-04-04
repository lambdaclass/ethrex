//! This module contains the logic for storage range downloads during state sync
//! It works like a queue, waiting for the state sync to advertise newly downloaded accounts with non-empty storages
//! Each storage will be queued and fetch in batches, once a storage is fully fetched it is then advertised to the storage rebuilder
//! Each downloaded storage will be written to the storage snapshot in the DB
//! If the pivot becomes stale while there are still pending storages in queue these will be sent to the storage healer
//! Even if the pivot becomes stale, the fetcher will remain active and listening until a termination signal (an empty batch) is received
//! Potential Improvements: Currenlty, we have a specific method to handle large storage tries (aka storage tries that don't fit into a single storage range request).
//! This method is called while fetching a storage batch and can stall the fetching of other smaller storages.
//! Large storage handling could be moved to its own separate queue process so that it runs parallel to regular storage fetching

use std::time::Instant;

use ethrex_common::H256;
use ethrex_storage::Store;
use ethrex_trie::Nibbles;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tracing::{debug, info, warn};

use crate::{
    peer_handler::{PeerHandler, RequestRangesMetrics},
    sync::{
        trie_rebuild::REBUILDER_INCOMPLETE_STORAGE_ROOT, utils::run_queue, BATCH_SIZE,
        MAX_CHANNEL_MESSAGES,
    },
};

use super::SyncError;
struct StorageFetcherMetrics {
    request_range_metrics: RequestRangesMetrics,
    full_time: u128,
    write_snapshot: u128,
}

impl StorageFetcherMetrics {
    fn show(&self) {
        let write_snapshot_percentage = (100 * self.write_snapshot) / self.full_time;
        let request_range_percentage =
            (100 * self.request_range_metrics.full_time) / self.full_time;
        info!(
            "Fetched storage batch of len {} in {} ms.
            Time Breakdown:
            {request_range_percentage}% Requesting Ranges ({}ms)
            {write_snapshot_percentage}% Writing Snapshot ({}ms)
            Request Range time breakdown:
            {}",
            self.request_range_metrics.ranges,
            self.full_time,
            self.request_range_metrics.full_time,
            self.write_snapshot,
            self.request_range_metrics.breakdown()
        );
    }
}
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
    // Spawn large storage fetcher
    let (large_storage_sender, large_storage_receiver) =
        channel::<Vec<(H256, H256, H256)>>(MAX_CHANNEL_MESSAGES);
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
    let l_sender = large_storage_sender.clone();
    let fetch_batch = move |batch: Vec<(H256, H256)>, peers: PeerHandler, store: Store| {
        let l_sender = l_sender.clone();
        let s_sender = storage_trie_rebuilder_sender.clone();
        async move {
            fetch_storage_batch(
                batch,
                state_root,
                peers,
                store,
                l_sender.clone(),
                s_sender.clone(),
            )
            .await
            .unwrap()
        }
    };
    run_queue(&mut receiver, &mut pending_storage, &fetch_batch, peers.clone(), store.clone(), BATCH_SIZE).await;
    info!(
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
    debug!(
        "Requesting storage ranges for addresses {}..{}",
        batch.first().unwrap().0,
        batch.last().unwrap().0
    );
    let full = Instant::now();
    let (batch_hahses, batch_roots) = batch.clone().into_iter().unzip();
    if let Some((mut keys, mut values, incomplete, request_range_metrics)) = peers
        .request_storage_ranges(state_root, batch_roots, batch_hahses, H256::zero())
        .await
    {
        debug!("Received {} storage ranges", keys.len(),);
        //debug!("Received {} storage ranges", keys.len(),);
        // Handle incomplete ranges
        if incomplete {
            // An incomplete range cannot be empty
            let (last_keys, last_values) = (keys.pop().unwrap(), values.pop().unwrap());
            // If only one incomplete range is returned then it must belong to a trie that is too big to fit into one request
            // We will handle this large trie separately
            if keys.is_empty() {
                info!("Large storage trie encountered, sending to queue");
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
        let write_to_snapshot = Instant::now();
        // Store the storage ranges & rebuild the storage trie for each account
        let complete_storages = batch[..values.len()].to_vec();
        let batch = batch[values.len()..].to_vec();
        let account_hashes: Vec<H256> = complete_storages.iter().map(|(hash, _)| *hash).collect();
        store.write_snapshot_storage_batches(account_hashes, keys, values)?;
        let write_snapshot = write_to_snapshot.elapsed().as_millis();
        // Send complete storages to the rebuilder
        storage_trie_rebuilder_sender
            .send(complete_storages)
            .await?;
        let full_time = full.elapsed().as_millis();
        let metrics = StorageFetcherMetrics {
            request_range_metrics,
            full_time,
            write_snapshot,
        };
        metrics.show();
        // Return remaining code hashes in the batch if we couldn't fetch all of them
        return Ok((batch, false));
    }
    // Pivot became stale
    warn!("STORAGE PIVOT BECAME STALE");
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
) -> Result<(), SyncError> {
    // Pending list of storages to fetch
    // (account_hash, storage_root, last_key)
    let mut pending_storage: Vec<(H256, H256, H256)> = vec![];
    // Create an async closure to pass to the generic task spawner
    let s_sender = storage_trie_rebuilder_sender.clone();
    let fetch_batch = move |batch: Vec<(H256, H256, H256)>, peers: PeerHandler, store: Store| {
        let s_sender = s_sender.clone();
        // Batch size should always be 1
        if batch.len() != 1 {
            warn!("Invalid large storage batch size, check source code");
        }
        async move {
            let (rem, stale) =
                fetch_large_storage_batch(batch[0], state_root, peers, store, s_sender.clone())
                    .await
                    .unwrap();
            let remaining = rem.map(|r| vec![r]).unwrap_or_default();
            (remaining, stale)
        }
    };
    run_queue(&mut receiver, &mut pending_storage, &fetch_batch, peers.clone(), store.clone(), BATCH_SIZE).await;
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
        store.set_storage_heal_paths(account_hashes.into_iter().map(|hash| (hash, vec![Nibbles::default()])).collect())?;
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
