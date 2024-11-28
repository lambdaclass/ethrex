use std::sync::Arc;

use ethrex_blockchain::error::ChainError;
use ethrex_core::{
    types::{Block, BlockHash, BlockHeader},
    H256,
};
use ethrex_storage::Store;
use tokio::{sync::Mutex, time::Instant};
use tracing::{debug, info, warn};

use crate::kademlia::KademliaTable;

/// Manager in charge the sync process
/// Only performs full-sync but will also be in charge of snap-sync in the future
#[derive(Debug)]
pub struct SyncManager {
    // true: syncmode = snap, false = syncmode = full
    #[allow(unused)]
    snap_mode: bool,
    peers: Arc<Mutex<KademliaTable>>,
}

impl SyncManager {
    pub fn new(peers: Arc<Mutex<KademliaTable>>, snap_mode: bool) -> Self {
        Self { snap_mode, peers }
    }

    /// Starts a sync cycle, updating the state with all blocks between the current head and the sync head
    /// TODO: only uses full sync, should also process snap sync once implemented
    pub async fn start_sync(&mut self, mut current_head: H256, sync_head: H256, store: Store) {
        info!("Syncing from current head {current_head} to sync_head {sync_head}");
        let start_time = Instant::now();
        // Request all block headers between the current head and the sync head
        // We will begin from the current head so that we download the earliest state first
        // This step is not parallelized
        let mut all_block_headers = vec![];
        let mut all_block_hashes = vec![];
        loop {
            let peer = self.peers.lock().await.get_peer_channels().await;
            debug!("Requesting Block Headers from {current_head}");
            // Request Block Headers from Peer
            if let Some(block_headers) = peer.request_block_headers(current_head).await {
                debug!("Received {} block headers", block_headers.len());
                let block_hashes = block_headers
                    .iter()
                    .map(|header| header.compute_block_hash())
                    .collect::<Vec<_>>();
                // Keep headers so we can process them later
                // Discard the first header as we already have it
                all_block_headers.extend_from_slice(&block_headers[1..]);
                all_block_hashes.extend_from_slice(&block_hashes[1..]);

                // Check if we already reached our sync head or if we need to fetch more blocks
                if !block_hashes.contains(&sync_head) {
                    // Update the request to fetch the next batch
                    current_head = *block_hashes.last().unwrap();
                } else {
                    // No more headers to request
                    break;
                }
            }
        }
        // We finished fetching all headers, now we can process them
        // TODO: snap-sync: launch tasks to fetch blocks and state in parallel
        // full-sync: Fetch all block bodies and execute them sequentially to build the state
        match tokio::spawn(download_and_run_blocks(
            all_block_hashes,
            all_block_headers,
            self.peers.clone(),
            store.clone(),
        ))
        .await
        {
            Ok(Ok(())) => info!(
                "Sync finished, time elapsed: {} secs",
                start_time.elapsed().as_secs()
            ),
            Ok(Err(error)) => warn!(
                "Sync failed due to {error}, time elapsed: {} secs ",
                start_time.elapsed().as_secs()
            ),
            _ => warn!(
                "Sync failed due to internal error, time elapsed: {} secs",
                start_time.elapsed().as_secs()
            ),
        }
    }

    /// Creates a dummy SyncManager for tests where syncing is not needed
    /// This should only be used in tests as it won't be able to connect to the p2p network
    pub fn dummy() -> Self {
        let dummy_peer_table = Arc::new(Mutex::new(KademliaTable::new(Default::default())));
        Self {
            snap_mode: false,
            peers: dummy_peer_table,
        }
    }
}

/// Requests block bodies from peers via p2p, executes and stores them
/// Returns an error if there was a problem while executing or validating the blocks
async fn download_and_run_blocks(
    mut block_hashes: Vec<BlockHash>,
    mut block_headers: Vec<BlockHeader>,
    peers: Arc<Mutex<KademliaTable>>,
    store: Store,
) -> Result<(), ChainError> {
    loop {
        let peer = peers.lock().await.get_peer_channels().await;
        debug!("Requesting Block Bodies ");
        if let Some(block_bodies) = peer.request_block_bodies(block_hashes.clone()).await {
            let block_bodies_len = block_bodies.len();
            debug!("Received {} Block Bodies", block_bodies_len);
            // Execute and store blocks
            for body in block_bodies.into_iter() {
                // We already validated that there are no more block bodies than the ones requested
                let header = block_headers.remove(0);
                let hash = block_hashes.remove(0);
                let number = header.number;
                let block = Block::new(header, body);
                if let Err(error) = ethrex_blockchain::add_block(&block, &store) {
                    warn!("Failed to add block during FullSync: {error}");
                    return Err(error);
                }
                store.set_canonical_block(number, hash)?;
                store.update_latest_block_number(number)?;
            }
            debug!("Executed & stored {} blocks", block_bodies_len);
            // Check if we need to ask for another batch
            if block_hashes.is_empty() {
                break;
            }
        }
    }
    Ok(())
}
