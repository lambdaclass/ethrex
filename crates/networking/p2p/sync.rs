mod state_healing;
mod storage_healing;

use crate::peer_handler::{PeerHandlerError, SNAP_LIMIT};
use crate::rlpx::p2p::SUPPORTED_ETH_CAPABILITIES;
use crate::sync::state_healing::heal_state_trie_wrap;
use crate::sync::storage_healing::heal_storage_trie;
use crate::utils::{
    current_unix_time, get_account_state_snapshots_dir, get_account_storages_snapshots_dir,
};
use crate::{
    metrics::METRICS,
    peer_handler::{HASH_MAX, MAX_BLOCK_BODIES_TO_REQUEST, PeerHandler},
};
use ethrex_blockchain::{BatchBlockProcessingFailure, Blockchain, error::ChainError};
use ethrex_common::{
    BigEndianHash, H256, U256,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountState, Block, BlockHash, BlockHeader},
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{EngineType, STATE_TRIE_SEGMENTS, Store, error::StoreError};
use ethrex_trie::Nibbles;
use ethrex_trie::{NodeHash, Trie, TrieError};
use rayon::iter::{IntoParallelIterator, ParallelBridge, ParallelIterator};
use std::cell::OnceCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::{
    array,
    cmp::min,
    collections::{HashMap, hash_map::Entry},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};
use tokio::{sync::mpsc::error::SendError, time::Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// The minimum amount of blocks from the head that we want to full sync during a snap sync
const MIN_FULL_BLOCKS: usize = 64;
/// Amount of blocks to execute in a single batch during FullSync
const EXECUTE_BATCH_SIZE_DEFAULT: usize = 1024;

#[cfg(feature = "sync-test")]
lazy_static::lazy_static! {
    static ref EXECUTE_BATCH_SIZE: usize = std::env::var("EXECUTE_BATCH_SIZE").map(|var| var.parse().expect("Execute batch size environmental variable is not a number")).unwrap_or(EXECUTE_BATCH_SIZE_DEFAULT);
}
#[cfg(not(feature = "sync-test"))]
lazy_static::lazy_static! {
    static ref EXECUTE_BATCH_SIZE: usize = EXECUTE_BATCH_SIZE_DEFAULT;
}

lazy_static::lazy_static! {
    // Size of each state trie segment
    static ref STATE_TRIE_SEGMENT_SIZE: U256 = HASH_MAX.into_uint()/STATE_TRIE_SEGMENTS;
    // Starting hash of each state trie segment
    static ref STATE_TRIE_SEGMENTS_START: [H256; STATE_TRIE_SEGMENTS] = {
        array::from_fn(|i| H256::from_uint(&(*STATE_TRIE_SEGMENT_SIZE * i)))
    };
    // Ending hash of each state trie segment
    static ref STATE_TRIE_SEGMENTS_END: [H256; STATE_TRIE_SEGMENTS] = {
        array::from_fn(|i| H256::from_uint(&(*STATE_TRIE_SEGMENT_SIZE * (i+1))))
    };
}

#[derive(Debug, PartialEq, Clone, Default)]
pub enum SyncMode {
    #[default]
    Full,
    Snap,
}

/// Manager in charge the sync process
/// Only performs full-sync but will also be in charge of snap-sync in the future
#[derive(Debug)]
pub struct Syncer {
    /// This is also held by the SyncManager allowing it to track the latest syncmode, without modifying it
    /// No outside process should modify this value, only being modified by the sync cycle
    snap_enabled: Arc<AtomicBool>,
    peers: PeerHandler,
    // Used for cancelling long-living tasks upon shutdown
    cancel_token: CancellationToken,
    blockchain: Arc<Blockchain>,
}

impl Syncer {
    pub fn new(
        peers: PeerHandler,
        snap_enabled: Arc<AtomicBool>,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
    ) -> Self {
        Self {
            snap_enabled,
            peers,
            cancel_token,
            blockchain,
        }
    }

    /// Creates a dummy Syncer for tests where syncing is not needed
    /// This should only be used in tests as it won't be able to connect to the p2p network
    pub fn dummy() -> Self {
        Self {
            snap_enabled: Arc::new(AtomicBool::new(false)),
            peers: PeerHandler::dummy(),
            // This won't be used
            cancel_token: CancellationToken::new(),
            blockchain: Arc::new(Blockchain::default_with_store(
                Store::new("", EngineType::InMemory).expect("Failed to start Sotre Engine"),
            )),
        }
    }

    /// Starts a sync cycle, updating the state with all blocks between the current head and the sync head
    /// Will perforn either full or snap sync depending on the manager's `snap_mode`
    /// In full mode, all blocks will be fetched via p2p eth requests and executed to rebuild the state
    /// In snap mode, blocks and receipts will be fetched and stored in parallel while the state is fetched via p2p snap requests
    /// After the sync cycle is complete, the sync mode will be set to full
    /// If the sync fails, no error will be returned but a warning will be emitted
    /// [WARNING] Sync is done optimistically, so headers and bodies may be stored even if their data has not been fully synced if the sync is aborted halfway
    /// [WARNING] Sync is currenlty simplified and will not download bodies + receipts previous to the pivot during snap sync
    pub async fn start_sync(&mut self, sync_head: H256, store: Store) {
        let start_time = Instant::now();
        match self.sync_cycle(sync_head, store).await {
            Ok(()) => {
                info!(
                    "Sync cycle finished, time elapsed: {} secs",
                    start_time.elapsed().as_secs()
                );
            }
            Err(error) => warn!(
                "Sync cycle failed due to {error}, time elapsed: {} secs ",
                start_time.elapsed().as_secs()
            ),
        }
    }

    /// Performs the sync cycle described in `start_sync`, returns an error if the sync fails at any given step and aborts all active processes
    async fn sync_cycle(&mut self, sync_head: H256, store: Store) -> Result<(), SyncError> {
        // Take picture of the current sync mode, we will update the original value when we need to
        if self.snap_enabled.load(Ordering::Relaxed) {
            METRICS.enable().await;
            let sync_cycle_result = self.sync_cycle_snap(sync_head, store).await;
            METRICS.disable().await;
            sync_cycle_result
        } else {
            self.sync_cycle_full(sync_head, store).await
        }
    }

    /// Performs the sync cycle described in `start_sync`, returns an error if the sync fails at any given step and aborts all active processes
    async fn sync_cycle_snap(&mut self, sync_head: H256, store: Store) -> Result<(), SyncError> {
        // Take picture of the current sync mode, we will update the original value when we need to
        let mut sync_mode = SyncMode::Snap;
        // Request all block headers between the current head and the sync head
        // We will begin from the current head so that we download the earliest state first
        // This step is not parallelized
        let mut block_sync_state = BlockSyncState::new(&sync_mode, store.clone());
        // Check if we have some blocks downloaded from a previous sync attempt
        // This applies only to snap sync—full sync always starts fetching headers
        // from the canonical block, which updates as new block headers are fetched.
        let mut current_head = block_sync_state.get_current_head().await?;
        let current_head_number = store
            .get_block_number(current_head)
            .await?
            .ok_or(SyncError::BlockNumber(current_head))?;
        info!(
            "Syncing from current head {:?} to sync_head {:?}",
            current_head, sync_head
        );
        let pending_block = match store.get_pending_block(sync_head).await {
            Ok(res) => res,
            Err(e) => return Err(e.into()),
        };

        loop {
            debug!("Sync Log 1: In snap sync");
            debug!(
                "Sync Log 2: State block hashes len {}",
                block_sync_state.to_snap_block_hashes().len()
            );
            debug!("Requesting Block Headers from {current_head}");

            let Some(mut block_headers) = self
                .peers
                .request_block_headers(current_head_number, sync_head)
                .await
            else {
                warn!("Sync failed to find target block header, aborting");
                return Ok(());
            };

            let (first_block_hash, first_block_number, first_block_parent_hash) =
                match block_headers.first() {
                    Some(header) => (header.hash(), header.number, header.parent_hash),
                    None => continue,
                };
            let (last_block_hash, last_block_number) = match block_headers.last() {
                Some(header) => (header.hash(), header.number),
                None => continue,
            };
            // TODO(#2126): This is just a temporary solution to avoid a bug where the sync would get stuck
            // on a loop when the target head is not found, i.e. on a reorg with a side-chain.
            if first_block_hash == last_block_hash
                && first_block_hash == current_head
                && current_head != sync_head
            {
                // There is no path to the sync head this goes back until it find a common ancerstor
                warn!("Sync failed to find target block header, going back to the previous parent");
                current_head = first_block_parent_hash;
                continue;
            }

            debug!(
                "Received {} block headers| First Number: {} Last Number: {}",
                block_headers.len(),
                first_block_number,
                last_block_number
            );

            // If we have a pending block from new_payload request
            // attach it to the end if it matches the parent_hash of the latest received header
            if let Some(ref block) = pending_block {
                if block.header.parent_hash == last_block_hash {
                    block_headers.push(block.header.clone());
                }
            }

            // Filter out everything after the sync_head
            let mut sync_head_found = false;
            if let Some(index) = block_headers
                .iter()
                .position(|header| header.hash() == sync_head)
            {
                sync_head_found = true;
                block_headers.drain(index + 1..);
            }

            // Update current fetch head
            current_head = last_block_hash;

            // If the sync head is less than 64 blocks away from our current head switch to full-sync
            if sync_mode == SyncMode::Snap && sync_head_found {
                let latest_block_number = store.get_latest_block_number().await?;
                if last_block_number.saturating_sub(latest_block_number) < MIN_FULL_BLOCKS as u64 {
                    // Too few blocks for a snap sync, switching to full sync
                    debug!(
                        "Sync head is less than {MIN_FULL_BLOCKS} blocks away, switching to FullSync"
                    );
                    sync_mode = SyncMode::Full;
                    self.snap_enabled.store(false, Ordering::Relaxed);
                    block_sync_state = block_sync_state.into_fullsync().await?;
                }
            }

            // Discard the first header as we already have it
            block_headers.remove(0);
            if !block_headers.is_empty() {
                match block_sync_state {
                    BlockSyncState::Full(ref mut state) => {
                        state
                            .process_incoming_headers(
                                block_headers,
                                sync_head_found,
                                self.blockchain.clone(),
                                self.peers.clone(),
                                self.cancel_token.clone(),
                            )
                            .await?;
                    }
                    BlockSyncState::Snap(ref mut state) => {
                        state.process_incoming_headers(block_headers).await?
                    }
                }
            }

            if sync_head_found {
                break;
            };
        }

        if let SyncMode::Snap = sync_mode {
            self.snap_sync(store, &mut block_sync_state).await?;

            // Next sync will be full-sync
            block_sync_state.into_fullsync().await?;
            self.snap_enabled.store(false, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Performs the sync cycle described in `start_sync`.
    ///
    /// # Returns
    ///
    /// Returns an error if the sync fails at any given step and aborts all active processes
    async fn sync_cycle_full(&mut self, sync_head: H256, store: Store) -> Result<(), SyncError> {
        // Request all block headers between the current head and the sync head
        // We will begin from the current head so that we download the earliest state first
        // This step is not parallelized
        let mut block_sync_state = FullBlockSyncState::new(store.clone());
        // Check if we have some blocks downloaded from a previous sync attempt
        // This applies only to snap sync—full sync always starts fetching headers
        // from the canonical block, which updates as new block headers are fetched.
        let mut current_head = block_sync_state.get_current_head().await?;
        let current_head_number = store
            .get_block_number(current_head)
            .await?
            .ok_or(SyncError::BlockNumber(current_head))?;
        info!(
            "Syncing from current head {:?} to sync_head {:?}",
            current_head, sync_head
        );
        let pending_block = match store.get_pending_block(sync_head).await {
            Ok(res) => res,
            Err(e) => return Err(e.into()),
        };

        loop {
            debug!("Sync Log 1: In Full Sync");
            debug!(
                "Sync Log 3: State current headears len {}",
                block_sync_state.current_headers.len()
            );
            debug!(
                "Sync Log 4: State current blocks len {}",
                block_sync_state.current_blocks.len()
            );

            debug!("Requesting Block Headers from {current_head}");

            let Some(mut block_headers) = self
                .peers
                .request_block_headers(current_head_number, sync_head)
                .await
            else {
                warn!("Sync failed to find target block header, aborting");
                debug!("Sync Log 8: Sync failed to find target block header, aborting");
                return Ok(());
            };

            debug!("Sync Log 9: Received {} block headers", block_headers.len());

            let (first_block_hash, first_block_number, first_block_parent_hash) =
                match block_headers.first() {
                    Some(header) => (header.hash(), header.number, header.parent_hash),
                    None => continue,
                };
            let (last_block_hash, last_block_number) = match block_headers.last() {
                Some(header) => (header.hash(), header.number),
                None => continue,
            };
            // TODO(#2126): This is just a temporary solution to avoid a bug where the sync would get stuck
            // on a loop when the target head is not found, i.e. on a reorg with a side-chain.
            if first_block_hash == last_block_hash
                && first_block_hash == current_head
                && current_head != sync_head
            {
                // There is no path to the sync head this goes back until it find a common ancerstor
                warn!("Sync failed to find target block header, going back to the previous parent");
                current_head = first_block_parent_hash;
                continue;
            }

            debug!(
                "Received {} block headers| First Number: {} Last Number: {}",
                block_headers.len(),
                first_block_number,
                last_block_number
            );

            // If we have a pending block from new_payload request
            // attach it to the end if it matches the parent_hash of the latest received header
            if let Some(ref block) = pending_block {
                if block.header.parent_hash == last_block_hash {
                    block_headers.push(block.header.clone());
                }
            }

            // Filter out everything after the sync_head
            let mut sync_head_found = false;
            if let Some(index) = block_headers
                .iter()
                .position(|header| header.hash() == sync_head)
            {
                sync_head_found = true;
                block_headers.drain(index + 1..);
            }

            // Update current fetch head
            current_head = last_block_hash;

            // Discard the first header as we already have it
            block_headers.remove(0);
            if !block_headers.is_empty() {
                let mut finished = false;
                while !finished {
                    finished = block_sync_state
                        .process_incoming_headers(
                            block_headers.clone(),
                            sync_head_found,
                            self.blockchain.clone(),
                            self.peers.clone(),
                            self.cancel_token.clone(),
                        )
                        .await?;
                    block_headers.clear();
                }
            }

            if sync_head_found {
                break;
            };
        }
        Ok(())
    }

    /// Executes the given blocks and stores them
    /// If sync_head_found is true, they will be executed one by one
    /// If sync_head_found is false, they will be executed in a single batch
    async fn add_blocks(
        blockchain: Arc<Blockchain>,
        blocks: Vec<Block>,
        sync_head_found: bool,
        cancel_token: CancellationToken,
    ) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
        // If we found the sync head, run the blocks sequentially to store all the blocks's state
        if sync_head_found {
            let mut last_valid_hash = H256::default();
            for block in blocks {
                blockchain.add_block(&block).await.map_err(|e| {
                    (
                        e,
                        Some(BatchBlockProcessingFailure {
                            last_valid_hash,
                            failed_block_hash: block.hash(),
                        }),
                    )
                })?;
                last_valid_hash = block.hash();
            }
            Ok(())
        } else {
            blockchain.add_blocks_in_batch(blocks, cancel_token).await
        }
    }
}

/// Fetches all block bodies for the given block hashes via p2p and stores them
async fn store_block_bodies(
    mut block_hashes: Vec<BlockHash>,
    peers: PeerHandler,
    store: Store,
) -> Result<(), SyncError> {
    loop {
        debug!("Requesting Block Bodies ");
        if let Some(block_bodies) = peers.request_block_bodies(block_hashes.clone()).await {
            debug!(" Received {} Block Bodies", block_bodies.len());
            // Track which bodies we have already fetched
            let current_block_hashes = block_hashes.drain(..block_bodies.len());
            // Add bodies to storage
            for (hash, body) in current_block_hashes.zip(block_bodies.into_iter()) {
                store.add_block_body(hash, body).await?;
            }

            // Check if we need to ask for another batch
            if block_hashes.is_empty() {
                break;
            }
        }
    }
    Ok(())
}

/// Fetches all receipts for the given block hashes via p2p and stores them
// TODO: remove allow when used again
#[allow(unused)]
async fn store_receipts(
    mut block_hashes: Vec<BlockHash>,
    peers: PeerHandler,
    store: Store,
) -> Result<(), SyncError> {
    loop {
        debug!("Requesting Receipts ");
        if let Some(receipts) = peers.request_receipts(block_hashes.clone()).await {
            debug!(" Received {} Receipts", receipts.len());
            // Track which blocks we have already fetched receipts for
            for (block_hash, receipts) in block_hashes.drain(0..receipts.len()).zip(receipts) {
                store.add_receipts(block_hash, receipts).await?;
            }
            // Check if we need to ask for another batch
            if block_hashes.is_empty() {
                break;
            }
        }
    }
    Ok(())
}

/// Persisted State during the Block Sync phase
#[derive(Clone)]
enum BlockSyncState {
    Full(FullBlockSyncState),
    Snap(SnapBlockSyncState),
}

/// Persisted State during the Block Sync phase for SnapSync
#[derive(Clone)]
struct SnapBlockSyncState {
    block_hashes: Vec<H256>,
    store: Store,
}

/// Persisted State during the Block Sync phase for FullSync
#[derive(Clone)]
struct FullBlockSyncState {
    current_headers: Vec<BlockHeader>,
    current_blocks: Vec<Block>,
    store: Store,
}

impl BlockSyncState {
    fn new(sync_mode: &SyncMode, store: Store) -> Self {
        match sync_mode {
            SyncMode::Full => BlockSyncState::Full(FullBlockSyncState::new(store)),
            SyncMode::Snap => BlockSyncState::Snap(SnapBlockSyncState::new(store)),
        }
    }

    /// Obtain the current head from where to start or resume block sync
    async fn get_current_head(&self) -> Result<H256, SyncError> {
        match self {
            BlockSyncState::Full(state) => state.get_current_head().await,
            BlockSyncState::Snap(state) => state.get_current_head().await,
        }
    }

    /// Consumes the current state and returns the contained block hashes if the state is a SnapSynd state
    /// If it is a FullSync state, returns an empty vector
    pub fn to_snap_block_hashes(&self) -> Vec<BlockHash> {
        match self {
            BlockSyncState::Full(_) => vec![],
            BlockSyncState::Snap(state) => state.block_hashes.clone(),
        }
    }

    /// Converts self into a FullSync state, does nothing if self is already a FullSync state
    pub async fn into_fullsync(self) -> Result<Self, SyncError> {
        // Switch from Snap to Full sync and vice versa
        let state = match self {
            BlockSyncState::Full(state) => state,
            BlockSyncState::Snap(state) => state.into_fullsync().await?,
        };
        Ok(Self::Full(state))
    }
}

impl FullBlockSyncState {
    fn new(store: Store) -> Self {
        Self {
            store,
            current_headers: Vec::new(),
            current_blocks: Vec::new(),
        }
    }

    /// Obtain the current head from where to start or resume block sync
    async fn get_current_head(&self) -> Result<H256, SyncError> {
        self.store
            .get_latest_canonical_block_hash()
            .await?
            .ok_or(SyncError::NoLatestCanonical)
    }

    /// Saves incoming headers, requests as many block bodies as needed to complete
    /// an execution batch and executes it.
    /// An incomplete batch may be executed if the sync_head was already found
    async fn process_incoming_headers(
        &mut self,
        block_headers: Vec<BlockHeader>,
        sync_head_found: bool,
        blockchain: Arc<Blockchain>,
        peers: PeerHandler,
        cancel_token: CancellationToken,
    ) -> Result<bool, SyncError> {
        info!("Processing incoming headers full sync");
        self.current_headers.extend(block_headers);
        let finished = self.current_headers.len() <= MAX_BLOCK_BODIES_TO_REQUEST;
        // if self.current_headers.len() < *EXECUTE_BATCH_SIZE && !sync_head_found {
        //     // We don't have enough headers to fill up a batch, lets request more
        //     return Ok(());
        // }
        // If we have enough headers to fill execution batches, request the matching bodies
        // while self.current_headers.len() >= *EXECUTE_BATCH_SIZE
        //     || !self.current_headers.is_empty() && sync_head_found
        // {
        // Download block bodies
        let headers =
            &self.current_headers[..min(MAX_BLOCK_BODIES_TO_REQUEST, self.current_headers.len())];
        let bodies = peers
            .request_and_validate_block_bodies(headers)
            .await
            .ok_or(SyncError::BodiesNotFound)?;
        debug!("Obtained: {} block bodies", bodies.len());
        let blocks = self
            .current_headers
            .drain(..bodies.len())
            .zip(bodies)
            .map(|(header, body)| Block { header, body });
        self.current_blocks.extend(blocks);
        // }
        // Execute full blocks
        // while self.current_blocks.len() >= *EXECUTE_BATCH_SIZE
        //     || (!self.current_blocks.is_empty() && sync_head_found)
        // {
        // Now that we have a full batch, we can execute and store the blocks in batch

        info!(
            "Executing {} blocks for full sync. First block hash: {:#?} Last block hash: {:#?}",
            self.current_blocks.len(),
            self.current_blocks
                .first()
                .ok_or(SyncError::NoBlocks)?
                .hash(),
            self.current_blocks
                .last()
                .ok_or(SyncError::NoBlocks)?
                .hash()
        );
        let execution_start = Instant::now();
        let block_batch: Vec<Block> = self
            .current_blocks
            .drain(..min(*EXECUTE_BATCH_SIZE, self.current_blocks.len()))
            .collect();
        // Copy some values for later
        let blocks_len = block_batch.len();
        let numbers_and_hashes = block_batch
            .iter()
            .map(|b| (b.header.number, b.hash()))
            .collect::<Vec<_>>();
        let (last_block_number, last_block_hash) = numbers_and_hashes
            .last()
            .cloned()
            .ok_or(SyncError::InvalidRangeReceived)?;
        let (first_block_number, first_block_hash) = numbers_and_hashes
            .first()
            .cloned()
            .ok_or(SyncError::InvalidRangeReceived)?;
        // Run the batch
        if let Err((err, batch_failure)) = Syncer::add_blocks(
            blockchain.clone(),
            block_batch,
            sync_head_found,
            cancel_token.clone(),
        )
        .await
        {
            if let Some(batch_failure) = batch_failure {
                warn!("Failed to add block during FullSync: {err}");
                self.store
                    .set_latest_valid_ancestor(
                        batch_failure.failed_block_hash,
                        batch_failure.last_valid_hash,
                    )
                    .await?;
            }
            return Err(err.into());
        }

        self.store
            .forkchoice_update(
                Some(numbers_and_hashes),
                last_block_number,
                last_block_hash,
                None,
                None,
            )
            .await?;

        let execution_time: f64 = execution_start.elapsed().as_millis() as f64 / 1000.0;
        let blocks_per_second = blocks_len as f64 / execution_time;

        info!(
            "[SYNCING] Executed & stored {} blocks in {:.3} seconds.\n\
            Started at block with hash {} (number {}).\n\
            Finished at block with hash {} (number {}).\n\
            Blocks per second: {:.3}",
            blocks_len,
            execution_time,
            first_block_hash,
            first_block_number,
            last_block_hash,
            last_block_number,
            blocks_per_second
        );
        // }
        Ok(finished)
    }
}

impl SnapBlockSyncState {
    fn new(store: Store) -> Self {
        Self {
            block_hashes: Vec::new(),
            store,
        }
    }

    /// Obtain the current head from where to start or resume block sync
    async fn get_current_head(&self) -> Result<H256, SyncError> {
        if let Some(head) = self.store.get_header_download_checkpoint().await? {
            Ok(head)
        } else {
            self.store
                .get_latest_canonical_block_hash()
                .await?
                .ok_or(SyncError::NoLatestCanonical)
        }
    }

    /// Stores incoming headers to the Store and saves their hashes
    async fn process_incoming_headers(
        &mut self,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), SyncError> {
        let block_hashes = block_headers.iter().map(|h| h.hash()).collect::<Vec<_>>();
        self.store
            .set_header_download_checkpoint(
                *block_hashes.last().ok_or(SyncError::InvalidRangeReceived)?,
            )
            .await?;
        self.block_hashes.extend_from_slice(&block_hashes);
        self.store.add_block_headers(block_headers).await?;
        Ok(())
    }

    /// Converts self into a FullSync state.
    /// Clears SnapSync checkpoints from the Store
    /// In the rare case that block headers were stored in a previous iteration, these will be fetched and saved to the FullSync state for full retrieval and execution
    async fn into_fullsync(self) -> Result<FullBlockSyncState, SyncError> {
        // For all collected hashes we must also have the corresponding headers stored
        // As this switch will only happen when the sync_head is 64 blocks away or less from our latest block
        // The headers to fetch will be at most 64, and none in the most common case
        let mut current_headers = Vec::new();
        for hash in self.block_hashes {
            let header = self
                .store
                .get_block_header_by_hash(hash)?
                .ok_or(SyncError::CorruptDB)?;
            current_headers.push(header);
        }
        self.store.clear_snap_state().await?;
        Ok(FullBlockSyncState {
            current_headers,
            current_blocks: Vec::new(),
            store: self.store,
        })
    }
}

impl Syncer {
    async fn snap_sync(
        &mut self,
        store: Store,
        block_sync_state: &mut BlockSyncState,
    ) -> Result<(), SyncError> {
        // snap-sync: launch tasks to fetch blocks and state in parallel
        // - Fetch each block's body and its receipt via eth p2p requests
        // - Fetch the pivot block's state via snap p2p requests
        // - Execute blocks after the pivot (like in full-sync)
        let all_block_hashes = block_sync_state.to_snap_block_hashes();
        let pivot_idx = all_block_hashes.len().saturating_sub(1);
        let mut pivot_header = store
            .get_block_header_by_hash(all_block_hashes[pivot_idx])?
            .ok_or(SyncError::CorruptDB)?;

        let mut staleness_timestamp: u64 = pivot_header.timestamp + (SNAP_LIMIT as u64 * 12);
        while current_unix_time() > staleness_timestamp {
            (pivot_header, staleness_timestamp) =
                update_pivot(pivot_header.number, &self.peers, block_sync_state).await?;
        }
        debug!(
            "Selected block {} as pivot for snap sync",
            pivot_header.number
        );

        let state_root = pivot_header.state_root;
        let account_state_snapshots_dir =
            get_account_state_snapshots_dir().ok_or(SyncError::AccountStateSnapshotsDirNotFound)?;
        let account_storages_snapshots_dir = get_account_storages_snapshots_dir()
            .ok_or(SyncError::AccountStoragesSnapshotsDirNotFound)?;

        let mut pivot_is_stale = true;
        let mut storage_accounts = AccountStorageRoots::default();
        if !std::env::var("SKIP_START_SNAP_SYNC").is_ok_and(|var| !var.is_empty()) {
            self.peers
                .request_account_range(
                    state_root,
                    H256::zero(),
                    H256::repeat_byte(0xff),
                    account_state_snapshots_dir,
                    staleness_timestamp,
                )
                .await;

            let empty = *EMPTY_TRIE_HASH;

            let mut chunk_index = 0;

            let account_state_snapshots_dir = get_account_state_snapshots_dir()
                .ok_or(SyncError::AccountStateSnapshotsDirNotFound)?;

            info!("Starting to download storage ranges from peers");

            let storage_tries_to_download = get_number_of_storage_tries_to_download().await?;
            *METRICS.storage_tries_to_download.lock().await = storage_tries_to_download;
            let mut downloaded_account_storages = 0;

            METRICS
                .storage_tries_download_start_time
                .lock()
                .await
                .replace(SystemTime::now());

            for entry in std::fs::read_dir(&account_state_snapshots_dir)
                .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
            {
                let entry = entry.map_err(|_| {
                    SyncError::SnapshotReadError(account_state_snapshots_dir.clone().into())
                })?;

                let snapshot_path = entry.path();

                let snapshot_contents = std::fs::read(&snapshot_path)
                    .map_err(|_| SyncError::SnapshotReadError(snapshot_path.clone()))?;

                let account_states_snapshot: Vec<(H256, AccountState)> =
                    RLPDecode::decode(&snapshot_contents)
                        .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

                let (account_hashes, account_states): (Vec<H256>, Vec<AccountState>) =
                    account_states_snapshot.iter().cloned().unzip();

                storage_accounts.accounts_with_storage_root.extend(
                    account_hashes
                        .iter()
                        .zip(account_states.iter())
                        .filter_map(|(hash, state)| {
                            (state.storage_root != empty).then_some((*hash, state.storage_root))
                        }),
                );

                chunk_index = self
                    .peers
                    .request_storage_ranges(
                        state_root,
                        account_states_snapshot
                            .iter()
                            .filter(|(_, acc)| acc.storage_root != empty)
                            .map(|(k, v)| (*k, v.storage_root))
                            .collect(),
                        account_storages_snapshots_dir.clone(),
                        chunk_index,
                        &mut downloaded_account_storages,
                        staleness_timestamp,
                    )
                    .await
                    .map_err(SyncError::PeerHandler)?;
                dbg!(&downloaded_account_storages);
            }
            info!("All account storages downloaded successfully");
            pivot_is_stale = current_unix_time() > staleness_timestamp;
            info!(
                "Finished downloading account ranges, total storage slots: {}",
                *METRICS.downloaded_storage_slots.lock().await
            );
            METRICS
                .storage_tries_download_end_time
                .lock()
                .await
                .replace(SystemTime::now());
            info!("Starting to compute the state root...");

            let account_store_start = Instant::now();

            let mut computed_state_root = *EMPTY_TRIE_HASH;

            for entry in std::fs::read_dir(&account_state_snapshots_dir)
                .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
            {
                let entry = entry.map_err(|_| {
                    SyncError::SnapshotReadError(account_state_snapshots_dir.clone().into())
                })?;

                let snapshot_path = entry.path();

                let snapshot_contents = std::fs::read(&snapshot_path)
                    .map_err(|_| SyncError::SnapshotReadError(snapshot_path.clone()))?;

                let account_state_snapshot: Vec<(H256, AccountState)> =
                    RLPDecode::decode(&snapshot_contents)
                        .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

                let trie = store.open_state_trie(computed_state_root)?;

                let current_state_root =
                    tokio::task::spawn_blocking(move || -> Result<H256, SyncError> {
                        let mut trie = trie;

                        for (account_hash, account) in account_state_snapshot {
                            trie.insert(account_hash.0.to_vec(), account.encode_to_vec())?;
                        }
                        let current_state_root = trie.hash()?;
                        Ok(current_state_root)
                    })
                    .await??;

                computed_state_root = current_state_root;
            }

            *METRICS.account_tries_state_root.lock().await = Some(computed_state_root);

            let account_store_time = Instant::now().saturating_duration_since(account_store_start);

            info!("Expected state root: {state_root:?}");
            info!("Computed state root: {computed_state_root:?} in {account_store_time:?}");

            let storages_store_start = Instant::now();

            METRICS
                .storage_tries_state_roots_start_time
                .lock()
                .await
                .replace(SystemTime::now());

            *METRICS.storage_tries_state_roots_to_compute.lock().await =
                downloaded_account_storages;

            let maybe_big_account_storage_state_roots: Arc<Mutex<HashMap<H256, H256>>> =
                Arc::new(Mutex::new(HashMap::new()));

            let account_storages_snapshots_dir = get_account_storages_snapshots_dir()
                .ok_or(SyncError::AccountStoragesSnapshotsDirNotFound)?;
            for entry in std::fs::read_dir(&account_storages_snapshots_dir)
                .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?
            {
                let entry = entry.map_err(|_| {
                    SyncError::SnapshotReadError(account_storages_snapshots_dir.clone().into())
                })?;

                let snapshot_path = entry.path();

                let snapshot_contents = std::fs::read(&snapshot_path)
                    .map_err(|_| SyncError::SnapshotReadError(snapshot_path.clone()))?;

                let account_storages_snapshot: Vec<(H256, Vec<(H256, U256)>)> =
                    RLPDecode::decode(&snapshot_contents)
                        .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

                let maybe_big_account_storage_state_roots_clone =
                    maybe_big_account_storage_state_roots.clone();
                let store_clone = store.clone();
                let pivot_hash_moved = pivot_header.hash();
                let storage_trie_node_changes = tokio::task::spawn_blocking(move || {
                    let store: Store = store_clone;

                    // TODO: Here we are filtering again the account with empty storage because we are adding empty accounts on purpose (it was the easiest thing to do)
                    // We need to fix this issue in request_storage_ranges and remove this filter.
                    account_storages_snapshot
                        .into_par_iter()
                        .filter(|(_account_hash, storage)| !storage.is_empty())
                        .map(|(account_hash, key_value_pairs)| {
                            compute_storage_roots(
                                maybe_big_account_storage_state_roots_clone.clone(),
                                store.clone(),
                                account_hash,
                                key_value_pairs,
                                pivot_hash_moved,
                            )
                        })
                        .collect::<Result<Vec<_>, SyncError>>()
                })
                .await??;

                store
                    .write_storage_trie_nodes_batch(storage_trie_node_changes)
                    .await?;
            }

            for (account_hash, computed_storage_root) in maybe_big_account_storage_state_roots
                .lock()
                .map_err(|_| SyncError::MaybeBigAccount)?
                .iter()
            {
                let account_state = store
                    .get_account_state_by_acc_hash(pivot_header.hash(), *account_hash)?
                    .ok_or(SyncError::AccountState(pivot_header.hash(), *account_hash))?;

                if *computed_storage_root != account_state.storage_root {
                    debug!(
                        "Incomplete or incorrect download for account hash {:?}, expected: {:?}, computed: {:?}",
                        *account_hash, account_state.storage_root, *computed_storage_root,
                    );
                }
            }

            METRICS
                .storage_tries_state_roots_end_time
                .lock()
                .await
                .replace(SystemTime::now());

            let storages_store_time =
                Instant::now().saturating_duration_since(storages_store_start);
            info!("Finished storing storage tries in: {storages_store_time:?}");
        }

        if pivot_is_stale {
            info!("Starting Fast Sync");
            let mut global_state_leafs_healed: u64 = 0;
            let mut global_storage_leafs_healed: u64 = 0;
            let mut healing_done = false;
            while !healing_done {
                // This if is an edge case for the skip snap sync scenario
                if current_unix_time() > staleness_timestamp {
                    (pivot_header, staleness_timestamp) =
                        update_pivot(pivot_header.number, &self.peers, block_sync_state).await?;
                }
                healing_done = heal_state_trie_wrap(
                    pivot_header.state_root,
                    store.clone(),
                    &self.peers,
                    staleness_timestamp,
                    &mut global_state_leafs_healed,
                    &mut storage_accounts,
                )
                .await?;
                if !healing_done {
                    continue;
                }
                healing_done = heal_storage_trie(
                    pivot_header.state_root,
                    &storage_accounts,
                    self.peers.clone(),
                    store.clone(),
                    HashMap::new(),
                    staleness_timestamp,
                    &mut global_storage_leafs_healed,
                )
                .await;
            }
            // TODO: 💀💀💀 either remove or change to a debug flag
            validate_state_root(store.clone(), pivot_header.state_root).await;
            validate_storage_root(store.clone(), pivot_header.state_root).await;
            info!("Finished healing");
        }

        let mut bytecode_iter = store
            .iter_accounts(pivot_header.state_root)
            .expect("we couldn't iterate over accounts")
            .map(|(_, state)| state.code_hash)
            .filter(|code_hash| *code_hash != *EMPTY_KECCACK_HASH);
        for mut bytecode_hashes in std::iter::from_fn(move || {
            Some(bytecode_iter.by_ref().take(10_000).collect())
                .filter(|chunk: &Vec<_>| !chunk.is_empty())
        }) {
            // Download bytecodes
            bytecode_hashes.sort();
            bytecode_hashes.dedup();
            info!(
                "Starting bytecode download of {} hashes",
                bytecode_hashes.len()
            );
            let bytecodes = self
                .peers
                .request_bytecodes(&bytecode_hashes)
                .await
                .map_err(SyncError::PeerHandler)?
                .ok_or(SyncError::BytecodesNotFound)?;
            store
                .write_account_code_batch(bytecode_hashes.into_iter().zip(bytecodes).collect())
                .await?;
        }

        store_block_bodies(vec![pivot_header.hash()], self.peers.clone(), store.clone()).await?;

        let block = store
            .get_block_by_hash(pivot_header.hash())
            .await?
            .ok_or(SyncError::CorruptDB)?;

        store.add_block(block).await?;

        let all_block_hashes = block_sync_state.to_snap_block_hashes();

        let numbers_and_hashes = all_block_hashes
            .into_iter()
            .rev()
            .enumerate()
            .map(|(i, hash)| (pivot_header.number - i as u64, hash))
            .collect::<Vec<_>>();

        store
            .forkchoice_update(
                Some(numbers_and_hashes),
                pivot_header.number,
                pivot_header.hash(),
                None,
                None,
            )
            .await?;
        Ok(())
    }
}

async fn get_number_of_storage_tries_to_download() -> Result<u64, SyncError> {
    let account_state_snapshots_dir =
        get_account_state_snapshots_dir().ok_or(SyncError::AccountStateSnapshotsDirNotFound)?;
    let mut number_of_accounts_with_non_empty_storage = 0;
    for entry in std::fs::read_dir(&account_state_snapshots_dir)
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
    {
        let entry = entry.map_err(|_| {
            SyncError::SnapshotReadError(account_state_snapshots_dir.clone().into())
        })?;
        let snapshot_path = entry.path();
        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|_| SyncError::SnapshotReadError(snapshot_path.clone()))?;

        let account_states_snapshot: Vec<(H256, AccountState)> =
            RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        // filter and the count accounts with non empty storage
        let accounts_with_non_empty_storage = account_states_snapshot
            .iter()
            .filter(|(_account_hash, account_state)| account_state.storage_root != *EMPTY_TRIE_HASH)
            .count() as u64;

        number_of_accounts_with_non_empty_storage += accounts_with_non_empty_storage;
    }
    Ok(number_of_accounts_with_non_empty_storage)
}

type StorageRoots = (H256, Vec<(NodeHash, Vec<u8>)>);

fn compute_storage_roots(
    maybe_big_account_storage_state_roots_clone: Arc<Mutex<HashMap<H256, H256>>>,
    store: Store,
    account_hash: H256,
    key_value_pairs: Vec<(H256, U256)>,
    pivot_hash: H256,
) -> Result<StorageRoots, SyncError> {
    let account_storage_root = match maybe_big_account_storage_state_roots_clone
        .lock()
        .map_err(|_| SyncError::MaybeBigAccount)?
        .entry(account_hash)
    {
        Entry::Occupied(occupied_entry) => *occupied_entry.get(),
        Entry::Vacant(_vacant_entry) => *EMPTY_TRIE_HASH,
    };

    let mut storage_trie = store.open_storage_trie(account_hash, account_storage_root)?;

    for (hashed_key, value) in key_value_pairs {
        if let Err(err) = storage_trie.insert(hashed_key.0.to_vec(), value.encode_to_vec()) {
            warn!(
                "Failed to insert hashed key {hashed_key:?} in account hash: {account_hash:?}, err={err:?}"
            );
        }
    }

    let (computed_state_root, changes) = storage_trie.collect_changes_since_last_hash();

    maybe_big_account_storage_state_roots_clone
        .lock()
        .map_err(|_| SyncError::MaybeBigAccount)?
        .insert(account_hash, computed_state_root);

    let account_state = store
        .get_account_state_by_acc_hash(pivot_hash, account_hash)?
        .ok_or(SyncError::AccountState(pivot_hash, account_hash))?;
    if computed_state_root == account_state.storage_root {
        METRICS.storage_tries_state_roots_computed.inc();
    }

    Ok((account_hash, changes))
}

async fn update_pivot(
    block_number: u64,
    peers: &PeerHandler,
    block_sync_state: &mut BlockSyncState,
) -> Result<(BlockHeader, u64), SyncError> {
    // We ask for a pivot which is slightly behind the limit. This is because our peers may not have the
    // latest one, or a slot was missed
    let new_pivot_block_number = block_number + SNAP_LIMIT as u64 - 11;
    loop {
        let mut scores = peers.peer_scores.lock().await;

        let (peer_id, mut peer_channel) = peers
            .get_peer_channel_with_highest_score(&SUPPORTED_ETH_CAPABILITIES, &mut scores)
            .await
            .map_err(SyncError::PeerHandler)?
            .ok_or(SyncError::NoPeers)?;

        let peer_score = scores.get(&peer_id).unwrap_or(&i64::MIN);
        info!(
            "Trying to update pivot to {new_pivot_block_number} with peer {peer_id} (score: {peer_score})"
        );
        let Some(pivot) = peers
            .get_block_header(&mut peer_channel, new_pivot_block_number)
            .await
            .map_err(SyncError::PeerHandler)?
        else {
            // Penalize peer
            scores.entry(peer_id).and_modify(|score| *score -= 1);
            let peer_score = scores.get(&peer_id).unwrap_or(&i64::MIN);
            warn!(
                "Received None pivot from peer {peer_id} (score after penalizing: {peer_score}). Retrying"
            );
            continue;
        };

        // Reward peer
        scores.entry(peer_id).and_modify(|score| {
            if *score < 10 {
                *score += 1;
            }
        });
        info!("Succesfully updated pivot");
        if let BlockSyncState::Snap(sync_state) = block_sync_state {
            let block_headers = peers
                .request_block_headers(block_number + 1, pivot.hash())
                .await
                .ok_or(SyncError::NoBlockHeaders)?;
            sync_state.process_incoming_headers(block_headers).await?;
        } else {
            return Err(SyncError::NotInSnapSync);
        }
        return Ok((pivot.clone(), pivot.timestamp + (SNAP_LIMIT as u64 * 12)));
    }
}

#[derive(Debug, Default)]
/// We store for optimization the accounts that need to heal storage
pub struct AccountStorageRoots {
    /// The accounts that have not been healed are guaranteed to have the original storage root
    /// we can read this storage root
    pub accounts_with_storage_root: HashMap<H256, H256>,
    /// If an account has been healed, it may return to a previous state, so we just store the account
    /// in a hashset
    pub healed_accounts: HashSet<H256>,
}

#[derive(thiserror::Error, Debug)]
enum SyncError {
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("{0}")]
    Send(String),
    #[error(transparent)]
    Trie(#[from] TrieError),
    #[error(transparent)]
    Rlp(#[from] RLPDecodeError),
    #[error(transparent)]
    JoinHandle(#[from] tokio::task::JoinError),
    #[error("Missing data from DB")]
    CorruptDB,
    #[error("No bodies were found for the given headers")]
    BodiesNotFound,
    #[error("Failed to fetch latest canonical block, unable to sync")]
    NoLatestCanonical,
    #[error("Range received is invalid")]
    InvalidRangeReceived,
    #[error("Failed to fetch block number for head {0}")]
    BlockNumber(H256),
    #[error("No blocks found")]
    NoBlocks,
    #[error("Failed to read snapshot from {0:?}")]
    SnapshotReadError(PathBuf),
    #[error("Failed to RLP decode account_state_snapshot from {0:?}")]
    SnapshotDecodeError(PathBuf),
    #[error("Failed to get account state for block {0:?} and account hash {1:?}")]
    AccountState(H256, H256),
    #[error("Failed to acquire lock on maybe_big_account_storage")]
    MaybeBigAccount,
    #[error("Failed to fetch bytecodes from peers")]
    BytecodesNotFound,
    #[error("Failed to get account state snapshots directory")]
    AccountStateSnapshotsDirNotFound,
    #[error("Failed to get account storages snapshots directory")]
    AccountStoragesSnapshotsDirNotFound,
    #[error("Got different state roots for account hash: {0:?}, expected: {1:?}, computed: {2:?}")]
    DifferentStateRoots(H256, H256, H256),
    #[error("We aren't finding get_peer_channel_with_retry")]
    NoPeers,
    #[error("Failed to get block headers")]
    NoBlockHeaders,
    #[error("Called update_pivot outside snapsync mode")]
    NotInSnapSync,
    #[error("Peer handler error: {0}")]
    PeerHandler(#[from] PeerHandlerError),
    #[error("Corrupt Path")]
    CorruptPath,
}

impl<T> From<SendError<T>> for SyncError {
    fn from(value: SendError<T>) -> Self {
        Self::Send(value.to_string())
    }
}

pub async fn validate_state_root(store: Store, state_root: H256) -> bool {
    info!("Starting validate_state_root");
    let computed_state_root = tokio::task::spawn_blocking(move || {
        Trie::compute_hash_from_unsorted_iter(
            store
                .iter_accounts(state_root)
                .expect("we couldn't iterate over accounts")
                .map(|(hash, state)| (hash.0.to_vec(), state.encode_to_vec())),
        )
    })
    .await
    .expect("We should be able to create threads");

    let tree_validated = state_root == computed_state_root;
    if tree_validated {
        info!("Succesfully validated tree, {state_root} found");
    } else {
        error!(
            "We have failed the validation of the state tree {state_root} expected but {computed_state_root} found"
        );
    }
    tree_validated
}

pub async fn validate_storage_root(store: Store, state_root: H256) {
    info!("Starting validate_storage_root");
    store
        .clone()
        .iter_accounts(state_root)
        .expect("We should be able to open the store")
        .par_bridge()
        .for_each(|(hashed_address, account_state)|
    {
        let store_clone = store.clone();
        let computed_storage_root = Trie::compute_hash_from_unsorted_iter(
                store_clone
                    .iter_storage(state_root, hashed_address)
                    .expect("we couldn't iterate over accounts")
                    .expect("This address should be valid")
                    .map(|(hash, state)| (hash.0.to_vec(), state.encode_to_vec())),
            );

        let tree_validated = account_state.storage_root == computed_storage_root;
        if tree_validated {
            //info!("Succesfully validated tree, {computed_storage_root} found");
        } else {
            error!(
                "We have failed the validation of the storage tree {} expected but {computed_storage_root} found",
                account_state.storage_root
            );
        }
    });
    info!("Finished validate_storage_root");
}
