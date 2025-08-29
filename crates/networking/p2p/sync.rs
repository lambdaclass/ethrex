mod bytecode_fetcher;
mod fetcher_queue;
mod state_healing;
mod state_sync;
mod storage_fetcher;
mod storage_healing;
mod trie_rebuild;

use crate::peer_handler::{BlockRequestOrder, HASH_MAX, MAX_BLOCK_BODIES_TO_REQUEST, PeerHandler};
use bytecode_fetcher::bytecode_fetcher;
use ethrex_blockchain::{BatchBlockProcessingFailure, Blockchain, error::ChainError};
use ethrex_common::{
    BigEndianHash, H256, U256, U512,
    types::{Block, BlockHash, BlockHeader},
};
use ethrex_rlp::error::RLPDecodeError;
use ethrex_storage::{EngineType, STATE_TRIE_SEGMENTS, Store, error::StoreError};
use ethrex_trie::{Nibbles, Node, TrieDB, TrieError};
use state_healing::heal_state_trie;
use state_sync::state_sync;
use std::{
    array,
    cmp::min,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use storage_healing::storage_healer;
use tokio::{
    sync::mpsc::error::SendError,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use trie_rebuild::TrieRebuilder;

/// The minimum amount of blocks from the head that we want to full sync during a snap sync
const MIN_FULL_BLOCKS: u32 = 64;
/// Max size of batch to start a bytecode fetch request in queues
const BYTECODE_BATCH_SIZE: usize = 70;
/// Max size of a batch to start a storage fetch request in queues
const STORAGE_BATCH_SIZE: usize = 300;
/// Max size of a batch to start a node fetch request in queues
const NODE_BATCH_SIZE: usize = 900;
/// Maximum amount of concurrent paralell fetches for a queue
const MAX_PARALLEL_FETCHES: u32 = 10;
/// Maximum amount of messages in a channel
const MAX_CHANNEL_MESSAGES: usize = 500;
/// Maximum amount of messages to read from a channel at once
const MAX_CHANNEL_READS: usize = 200;
/// Pace at which progress is shown via info tracing
const SHOW_PROGRESS_INTERVAL_DURATION: Duration = Duration::from_secs(30);
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
    /// The last block number used as a pivot for snap-sync
    /// Syncing beyond this pivot should re-enable snap-sync (as we will not have that state stored)
    /// TODO: Reorgs
    last_snap_pivot: u64,
    trie_rebuilder: Option<TrieRebuilder>,
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
            last_snap_pivot: 0,
            trie_rebuilder: None,
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
            last_snap_pivot: 0,
            trie_rebuilder: None,
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
        let mut sync_mode = if self.snap_enabled.load(Ordering::Relaxed) {
            SyncMode::Snap
        } else {
            SyncMode::Full
        };
        // Request all block headers between the current head and the sync head
        // We will begin from the current head so that we download the earliest state first
        // This step is not parallelized
        let mut block_sync_state = BlockSyncState::new(&sync_mode, store.clone());
        // Check if we have some blocks downloaded from a previous sync attempt
        // This applies only to snap sync—full sync always starts fetching headers
        // from the canonical block, which updates as new block headers are fetched.
        let mut current_head = block_sync_state.get_current_head().await?;
        info!(
            "Syncing from current head {:?} to sync_head {:?}",
            current_head, sync_head
        );
        let pending_block = match store.get_pending_block(sync_head).await {
            Ok(res) => res,
            Err(e) => return Err(e.into()),
        };

        loop {
            debug!("Requesting Block Headers from {current_head}");

            let Some(mut block_headers) = self
                .peers
                .request_block_headers(current_head, BlockRequestOrder::OldToNew)
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
                block_sync_state
                    .process_incoming_headers(
                        block_headers,
                        sync_head_found,
                        self.blockchain.clone(),
                        self.peers.clone(),
                        self.cancel_token.clone(),
                    )
                    .await?;
            }

            if sync_head_found {
                break;
            };
        }
        match sync_mode {
            SyncMode::Snap => {
                // snap-sync: launch tasks to fetch blocks and state in parallel
                // - Fetch each block's body and its receipt via eth p2p requests
                // - Fetch the pivot block's state via snap p2p requests
                // - Execute blocks after the pivot (like in full-sync)
                let all_block_hashes = block_sync_state.into_snap_block_hashes();
                let pivot_idx = all_block_hashes
                    .len()
                    .saturating_sub(MIN_FULL_BLOCKS as usize);
                let pivot_header = store
                    .get_block_header_by_hash(all_block_hashes[pivot_idx])?
                    .ok_or(SyncError::CorruptDB)?;
                debug!(
                    "Selected block {} as pivot for snap sync",
                    pivot_header.number
                );
                let store_bodies_handle = tokio::spawn(store_block_bodies(
                    all_block_hashes[pivot_idx + 1..].to_vec(),
                    self.peers.clone(),
                    store.clone(),
                ));
                // Perform snap sync
                if !self
                    .snap_sync(pivot_header.state_root, store.clone())
                    .await?
                {
                    // Snap sync was not completed, abort and resume it on the next cycle
                    return Ok(());
                }
                // Wait for all bodies to be downloaded
                store_bodies_handle.await??;
                // For all blocks before the pivot: Store the bodies and fetch the receipts (TODO)
                // For all blocks after the pivot: Process them fully
                for hash in &all_block_hashes[pivot_idx + 1..] {
                    let block = store
                        .get_block_by_hash(*hash)
                        .await?
                        .ok_or(SyncError::CorruptDB)?;
                    let block_number = block.header.number;
                    self.blockchain.add_block(&block).await?;
                    store
                        .forkchoice_update(None, block_number, *hash, None, None)
                        .await?;
                }
                self.last_snap_pivot = pivot_header.number;
                // Finished a sync cycle without aborting halfway, clear current checkpoint
                store.clear_snap_state().await?;
                // Next sync will be full-sync
                self.snap_enabled.store(false, Ordering::Relaxed);
            }
            // Full sync stores and executes blocks as it asks for the headers
            SyncMode::Full => {}
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
enum BlockSyncState {
    Full(FullBlockSyncState),
    Snap(SnapBlockSyncState),
}

/// Persisted State during the Block Sync phase for SnapSync
struct SnapBlockSyncState {
    block_hashes: Vec<H256>,
    store: Store,
}

/// Persisted State during the Block Sync phase for FullSync
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

    /// Processes the incoming batch of headers from BlockSync
    /// For FullSync: Request corresponding bodies and execute the full blocks
    /// For SnapSync: Store headers
    async fn process_incoming_headers(
        &mut self,
        block_headers: Vec<BlockHeader>,
        sync_head_found: bool,
        blockchain: Arc<Blockchain>,
        peers: PeerHandler,
        cancel_token: CancellationToken,
    ) -> Result<(), SyncError> {
        match self {
            BlockSyncState::Full(state) => {
                state
                    .process_incoming_headers(
                        block_headers,
                        sync_head_found,
                        blockchain,
                        peers,
                        cancel_token,
                    )
                    .await
            }
            BlockSyncState::Snap(state) => state.process_incoming_headers(block_headers).await,
        }
    }

    /// Consumes the current state and returns the contained block hashes if the state is a SnapSynd state
    /// If it is a FullSync state, returns an empty vector
    pub fn into_snap_block_hashes(self) -> Vec<BlockHash> {
        match self {
            BlockSyncState::Full(_) => vec![],
            BlockSyncState::Snap(state) => state.block_hashes,
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

    /// Saves incoming headers, requests as many block bodies as needed to complete an execution batch and executes it
    /// An incomplete batch may be executed if the sync_head was already found
    async fn process_incoming_headers(
        &mut self,
        block_headers: Vec<BlockHeader>,
        sync_head_found: bool,
        blockchain: Arc<Blockchain>,
        peers: PeerHandler,
        cancel_token: CancellationToken,
    ) -> Result<(), SyncError> {
        self.current_headers.extend(block_headers);
        if self.current_headers.len() < *EXECUTE_BATCH_SIZE && !sync_head_found {
            // We don't have enough headers to fill up a batch, lets request more
            return Ok(());
        }
        // If we have enough headers to fill execution batches, request the matching bodies
        while self.current_headers.len() >= *EXECUTE_BATCH_SIZE
            || !self.current_headers.is_empty() && sync_head_found
        {
            // Download block bodies
            let headers = &self.current_headers
                [..min(MAX_BLOCK_BODIES_TO_REQUEST, self.current_headers.len())];
            let bodies = peers
                .request_and_validate_block_bodies(headers)
                .await
                .ok_or(SyncError::BodiesNotFound)?;
            debug!("Obtained: {} block bodies", bodies.len());
            let blocks = self
                .current_headers
                .drain(..bodies.len())
                .zip(bodies)
                .map(|(header, body)| Block::new(header, body));
            self.current_blocks.extend(blocks);
        }
        // Execute full blocks
        while self.current_blocks.len() >= *EXECUTE_BATCH_SIZE
            || (!self.current_blocks.is_empty() && sync_head_found)
        {
            // Now that we have a full batch, we can execute and store the blocks in batch
            let execution_start = Instant::now();
            let block_batch: Vec<Block> = self
                .current_blocks
                .drain(..min(*EXECUTE_BATCH_SIZE, self.current_blocks.len()))
                .collect();
            // Copy some values for later
            let blocks_len = block_batch.len();
            let mut numbers_and_hashes = block_batch
                .iter()
                .map(|b| (b.header.number, b.hash()))
                .collect::<Vec<_>>();
            let (last_block_number, last_block_hash) = numbers_and_hashes
                .pop()
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
            // Mark chain as canonical & last block as latest
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
        }
        Ok(())
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
    // Downloads the latest state trie and all associated storage tries & bytecodes from peers
    // Rebuilds the state trie and all storage tries based on the downloaded data
    // Performs state healing in order to fix all inconsistencies with the downloaded state
    // Returns the success status, if it is true, then the state is fully consistent and
    // new blocks can be executed on top of it, if false then the state is still inconsistent and
    // snap sync must be resumed on the next sync cycle
    async fn snap_sync(&mut self, state_root: H256, store: Store) -> Result<bool, SyncError> {
        // Begin the background trie rebuild process if it is not active yet or if it crashed
        if !self
            .trie_rebuilder
            .as_ref()
            .is_some_and(|rebuilder| rebuilder.alive())
        {
            self.trie_rebuilder = Some(TrieRebuilder::startup(
                self.cancel_token.clone(),
                store.clone(),
            ));
        };
        // Spawn storage healer earlier so we can start healing stale storages
        // Create a cancellation token so we can end the storage healer when finished, make it a child so that it also ends upon shutdown
        let storage_healer_cancell_token = self.cancel_token.child_token();
        // Create an AtomicBool to signal to the storage healer whether state healing has ended
        let state_healing_ended = Arc::new(AtomicBool::new(false));
        let storage_healer_handler = tokio::spawn(storage_healer(
            state_root,
            self.peers.clone(),
            store.clone(),
            storage_healer_cancell_token.clone(),
            state_healing_ended.clone(),
        ));
        // Perform state sync if it was not already completed on a previous cycle
        // Retrieve storage data to check which snap sync phase we are in
        let key_checkpoints = store.get_state_trie_key_checkpoint().await?;
        // If we have no key checkpoints or if the key checkpoints are lower than the segment boundaries we are in state sync phase
        if key_checkpoints.is_none()
            || key_checkpoints.is_some_and(|ch| {
                ch.into_iter()
                    .zip(STATE_TRIE_SEGMENTS_END.into_iter())
                    .any(|(ch, end)| ch < end)
            })
        {
            let storage_trie_rebuilder_sender = self
                .trie_rebuilder
                .as_ref()
                .ok_or(SyncError::Trie(TrieError::InconsistentTree))?
                .storage_rebuilder_sender
                .clone();

            let stale_pivot = state_sync(
                state_root,
                store.clone(),
                self.peers.clone(),
                key_checkpoints,
                storage_trie_rebuilder_sender,
            )
            .await?;
            if stale_pivot {
                warn!("Stale Pivot, aborting state sync");
                storage_healer_cancell_token.cancel();
                storage_healer_handler.await??;
                return Ok(false);
            }
        }
        // Wait for the trie rebuilder to finish
        info!("Waiting for the trie rebuild to finish");
        let rebuild_start = Instant::now();
        let rebuilder = self
            .trie_rebuilder
            .take()
            .ok_or(SyncError::Trie(TrieError::InconsistentTree))?;
        rebuilder.complete().await?;

        info!(
            "State trie rebuilt from snapshot, overtime: {}",
            rebuild_start.elapsed().as_secs()
        );
        // Clear snapshot
        store.clear_snapshot().await?;

        // Perform Healing
        let state_heal_complete =
            heal_state_trie(state_root, store.clone(), self.peers.clone()).await?;
        // Wait for storage healer to end
        if state_heal_complete {
            state_healing_ended.store(true, Ordering::Relaxed);
        } else {
            storage_healer_cancell_token.cancel();
        }
        let storage_heal_complete = storage_healer_handler.await??;
        if !(state_heal_complete && storage_heal_complete) {
            warn!("Stale pivot, aborting healing");
        }
        Ok(state_heal_complete && storage_heal_complete)
    }
}

/// Returns the partial paths to the node's children if they are not already part of the trie state
fn node_missing_children(
    node: &Node,
    parent_path: &Nibbles,
    trie_state: &dyn TrieDB,
) -> Result<Vec<Nibbles>, TrieError> {
    let mut paths = Vec::new();
    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                if child.is_valid() && child.get_node(trie_state)?.is_none() {
                    paths.push(parent_path.append_new(index as u8));
                }
            }
        }
        Node::Extension(node) => {
            if node.child.is_valid() && node.child.get_node(trie_state)?.is_none() {
                paths.push(parent_path.concat(node.prefix.clone()));
            }
        }
        _ => {}
    }
    Ok(paths)
}

fn seconds_to_readable(seconds: U512) -> String {
    let (days, rest) = seconds.div_mod(U512::from(60 * 60 * 24));
    let (hours, rest) = rest.div_mod(U512::from(60 * 60));
    let (minutes, seconds) = rest.div_mod(U512::from(60));
    if days > U512::zero() {
        if days > U512::from(15) {
            return "unknown".to_string();
        }
        return format!("Over {days} days");
    }
    format!("{hours}h{minutes}m{seconds}s")
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
    #[error("Corrupt path during state healing")]
    CorruptPath,
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
}

impl<T> From<SendError<T>> for SyncError {
    fn from(value: SendError<T>) -> Self {
        Self::Send(value.to_string())
    }
}
