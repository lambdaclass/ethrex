mod code_collector;
mod state_healing;
mod storage_healing;

use crate::peer_handler::{BlockRequestOrder, PeerHandlerError, SNAP_LIMIT};
use crate::rlpx::p2p::SUPPORTED_ETH_CAPABILITIES;
use crate::sync::code_collector::CodeHashCollector;
use crate::sync::state_healing::heal_state_trie_wrap;
use crate::sync::storage_healing::heal_storage_trie;
use crate::utils::{
    current_unix_time, get_account_state_snapshots_dir, get_account_storages_snapshots_dir,
    get_code_hashes_snapshots_dir, get_rocksdb_temp_accounts_dir, get_rocksdb_temp_storage_dir,
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
use ethrex_trie::trie_sorted::TrieGenerationError;
use ethrex_trie::{NodeHash, Trie, TrieError};
use rayon::iter::{IntoParallelIterator, ParallelBridge, ParallelIterator};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use std::{
    array,
    cmp::min,
    collections::{HashMap, hash_map::Entry},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{sync::mpsc::error::SendError, time::Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// The minimum amount of blocks from the head that we want to full sync during a snap sync
const MIN_FULL_BLOCKS: usize = 64;
/// Amount of blocks to execute in a single batch during FullSync
const EXECUTE_BATCH_SIZE_DEFAULT: usize = 1024;
/// Amount of seconds between blocks
const SECONDS_PER_BLOCK: u64 = 12;

/// Bytecodes to downloader per batch
const BYTECODE_CHUNK_SIZE: usize = 50_000;

const MISSING_SLOTS_PERCENTAGE: f64 = 0.9;

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
#[derive(Debug)]
pub struct Syncer {
    /// This is also held by the SyncManager allowing it to track the latest syncmode, without modifying it
    /// No outside process should modify this value, only being modified by the sync cycle
    snap_enabled: Arc<AtomicBool>,
    peers: PeerHandler,
    // Used for cancelling long-living tasks upon shutdown
    cancel_token: CancellationToken,
    blockchain: Arc<Blockchain>,
    /// This string indicates a folder where the snap algorithm will store temporary files that are
    /// used during the syncing process
    datadir: PathBuf,
}

impl Syncer {
    pub fn new(
        peers: PeerHandler,
        snap_enabled: Arc<AtomicBool>,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
        datadir: PathBuf,
    ) -> Self {
        Self {
            snap_enabled,
            peers,
            cancel_token,
            blockchain,
            datadir,
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
                Store::new("", EngineType::InMemory).expect("Failed to start Store Engine"),
            )),
            datadir: ".".into(),
        }
    }

    /// Starts a sync cycle, updating the state with all blocks between the current head and the sync head
    /// Will perform either full or snap sync depending on the manager's `snap_mode`
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
        let mut current_head_number = store
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
                match block_sync_state {
                    BlockSyncState::Full(_) => 0,
                    BlockSyncState::Snap(ref snap_block_sync_state) =>
                        snap_block_sync_state.block_hashes.len(),
                }
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
            current_head_number = last_block_number;

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
                                sync_head,
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
        info!(
            "Syncing from current head {:?} to sync_head {:?}",
            current_head, sync_head
        );

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
                .request_block_headers_from_hash(current_head, BlockRequestOrder::OldToNew)
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
                    (finished, sync_head_found) = block_sync_state
                        .process_incoming_headers(
                            block_headers.clone(),
                            sync_head,
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
    mut peers: PeerHandler,
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
pub enum BlockSyncState {
    Full(FullBlockSyncState),
    Snap(SnapBlockSyncState),
}

/// Persisted State during the Block Sync phase for SnapSync
#[derive(Clone)]
pub struct SnapBlockSyncState {
    block_hashes: Vec<H256>,
    store: Store,
}

/// Persisted State during the Block Sync phase for FullSync
#[derive(Clone)]
pub struct FullBlockSyncState {
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
    /// Returns bool finish to know whether the amount of block headers was less than MAX_BLOCK_BODIES_TO_REQUEST
    /// to determine if there's still more blocks to download.
    /// Returns bool sync_head_found to know whether full sync was completed.
    async fn process_incoming_headers(
        &mut self,
        block_headers: Vec<BlockHeader>,
        sync_head: H256,
        sync_head_found_in_block_headers: bool,
        blockchain: Arc<Blockchain>,
        mut peers: PeerHandler,
        cancel_token: CancellationToken,
    ) -> Result<(bool, bool), SyncError> {
        info!("Processing incoming headers full sync");
        self.current_headers.extend(block_headers);

        let mut sync_head_found = sync_head_found_in_block_headers;
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

        // If we have the sync_head as a pending block from a new_payload request and its parent_hash matches the hash of the latest received header
        // we set the sync_head as found. Then we add it in current_blocks for execution.
        if let Some(block) = self.store.get_pending_block(sync_head).await? {
            if let Some(last_block) = self.current_blocks.last() {
                if last_block.hash() == block.header.parent_hash {
                    self.current_blocks.push(block);
                    sync_head_found = true;
                }
            }
        }
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

        let block_batch_hashes = block_batch
            .iter()
            .map(|block| block.hash())
            .collect::<Vec<_>>();

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
                // Since running the batch failed we set the failing block and it's descendants with having an invalid ancestor on the following cases.
                if let ChainError::InvalidBlock(_) = err {
                    let mut block_hashes_with_invalid_ancestor: Vec<H256> = vec![];
                    if let Some(index) = block_batch_hashes
                        .iter()
                        .position(|x| x == &batch_failure.failed_block_hash)
                    {
                        block_hashes_with_invalid_ancestor = block_batch_hashes[index..].to_vec();
                    }

                    for hash in block_hashes_with_invalid_ancestor {
                        self.store
                            .set_latest_valid_ancestor(hash, batch_failure.last_valid_hash)
                            .await?;
                    }
                    // We also set with having an invalid ancestor all the hashes remaining which are descendants as well.
                    for header in &self.current_headers {
                        self.store
                            .set_latest_valid_ancestor(header.hash(), batch_failure.last_valid_hash)
                            .await?;
                    }
                }
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
        Ok((finished, sync_head_found))
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

/// Safety function that frees all peer and logs an error if we found freed peers when not expectig to
/// Logs with where the function was when it found this error
/// TODO: remove this function once peer table has moved to spawned implementation
async fn free_peers_and_log_if_not_empty(peer_handler: &PeerHandler) {
    if peer_handler.peer_table.free_peers().await != 0 {
        let step = METRICS.current_step.lock().await.clone();
        error!(
            step = step,
            "Found peers marked as used even though we just finished this step"
        );
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
        let pivot_hash = match block_sync_state {
            BlockSyncState::Full(_) => return Err(SyncError::NotInSnapSync),
            BlockSyncState::Snap(snap_block_sync_state) => snap_block_sync_state
                .block_hashes
                .last()
                .ok_or(SyncError::NoBlockHeaders)?,
        };
        let mut pivot_header = store
            .get_block_header_by_hash(*pivot_hash)?
            .ok_or(SyncError::CorruptDB)?;

        while block_is_stale(&pivot_header) {
            pivot_header = update_pivot(
                pivot_header.number,
                pivot_header.timestamp,
                &mut self.peers,
                block_sync_state,
            )
            .await?;
        }
        debug!(
            "Selected block {} as pivot for snap sync",
            pivot_header.number
        );

        let state_root = pivot_header.state_root;
        let account_state_snapshots_dir = get_account_state_snapshots_dir(&self.datadir);
        let account_storages_snapshots_dir = get_account_storages_snapshots_dir(&self.datadir);

        let code_hashes_snapshot_dir = get_code_hashes_snapshots_dir(&self.datadir);
        std::fs::create_dir_all(&code_hashes_snapshot_dir).map_err(|_| SyncError::CorruptPath)?;

        // Create collector to store code hashes in files
        let mut code_hash_collector = CodeHashCollector::new(code_hashes_snapshot_dir.clone());

        let mut storage_accounts = AccountStorageRoots::default();
        if !std::env::var("SKIP_START_SNAP_SYNC").is_ok_and(|var| !var.is_empty()) {
            // We start by downloading all of the leafs of the trie of accounts
            // The function request_account_range writes the leafs into files in
            // account_state_snapshots_dir

            info!("Starting to download account ranges from peers");
            self.peers
                .request_account_range(
                    H256::zero(),
                    H256::repeat_byte(0xff),
                    account_state_snapshots_dir.as_ref(),
                    &mut pivot_header,
                    block_sync_state,
                )
                .await?;
            free_peers_and_log_if_not_empty(&self.peers).await;
            info!("Finish downloading account ranges from peers");

            *METRICS.account_tries_insert_start_time.lock().await = Some(SystemTime::now());
            // We read the account leafs from the files in account_state_snapshots_dir, write it into
            // the trie to compute the nodes and stores the accounts with storages for later use
            cfg_if::cfg_if! {
                if #[cfg(feature = "rocksdb")] {
                    let computed_state_root = insert_accounts_into_rocksdb(
                        store.clone(),
                        &mut storage_accounts,
                        &account_state_snapshots_dir,
                        &get_rocksdb_temp_accounts_dir(&self.datadir)
                    ).await?;
                    let accounts_with_storage: BTreeSet<H256> = BTreeSet::from_iter(storage_accounts.accounts_with_storage_root.keys().into_iter().map(|k| *k));
                } else {
                    let computed_state_root = insert_accounts_into_db(
                        store.clone(),
                        &mut storage_accounts,
                        &account_state_snapshots_dir,
                    ).await?;
                }
            }
            info!(
                "Finished inserting account ranges, total storage accounts: {}",
                storage_accounts.accounts_with_storage_root.len()
            );
            *METRICS.account_tries_insert_end_time.lock().await = Some(SystemTime::now());

            info!("Original state root: {state_root:?}");
            info!("Computed state root after request_account_rages: {computed_state_root:?}");

            *METRICS.storage_tries_download_start_time.lock().await = Some(SystemTime::now());
            METRICS.storage_accounts_initial.store(
                storage_accounts.accounts_with_storage_root.len() as u64,
                Ordering::Relaxed,
            );
            // We start downloading the storage leafs. To do so, we need to be sure that the storage root
            // is correct. To do so, we always heal the state trie before requesting storage rates
            let mut chunk_index = 0_u64;
            let mut state_leafs_healed = 0_u64;
            loop {
                while block_is_stale(&pivot_header) {
                    pivot_header = update_pivot(
                        pivot_header.number,
                        pivot_header.timestamp,
                        &mut self.peers,
                        block_sync_state,
                    )
                    .await?;
                }
                // heal_state_trie_wrap returns false if we ran out of time before fully healing the trie
                // We just need to update the pivot and start again
                if !heal_state_trie_wrap(
                    pivot_header.state_root,
                    store.clone(),
                    &self.peers,
                    calculate_staleness_timestamp(pivot_header.timestamp),
                    &mut state_leafs_healed,
                    &mut storage_accounts,
                    &mut code_hash_collector,
                )
                .await?
                {
                    continue;
                };
                free_peers_and_log_if_not_empty(&self.peers).await;

                info!(
                    "Started request_storage_ranges with {} accounts with storage root unchanged",
                    storage_accounts.accounts_with_storage_root.len()
                );
                chunk_index = self
                    .peers
                    .request_storage_ranges(
                        &mut storage_accounts,
                        account_storages_snapshots_dir.as_ref(),
                        chunk_index,
                        &mut pivot_header,
                    )
                    .await
                    .map_err(SyncError::PeerHandler)?;
                free_peers_and_log_if_not_empty(&self.peers).await;

                info!(
                    "Ended request_storage_ranges with {} accounts with storage root unchanged and not downloaded yet and with {} big/healed accounts",
                    storage_accounts.accounts_with_storage_root.len(),
                    // These accounts are marked as heals if they're a big account. This is
                    // because we don't know if the storage root is still valid
                    storage_accounts.healed_accounts.len(),
                );
                if !block_is_stale(&pivot_header) {
                    break;
                }
                info!("We stopped because of staleness, restarting loop");
            }
            info!("Finished request_storage_ranges");
            METRICS.storage_accounts_healed.store(
                storage_accounts.healed_accounts.len() as u64,
                Ordering::Relaxed,
            );
            *METRICS.storage_tries_download_end_time.lock().await = Some(SystemTime::now());

            *METRICS.storage_tries_insert_start_time.lock().await = Some(SystemTime::now());
            *METRICS.current_step.lock().await =
                "Inserting Storage Ranges - \x1b[31mWriting to DB\x1b[0m".to_string();
            let account_storages_snapshots_dir = get_account_storages_snapshots_dir(&self.datadir);

            cfg_if::cfg_if! {
                if #[cfg(feature = "rocksdb")] {
                    insert_storage_into_rocksdb(
                        store.clone(),
                        accounts_with_storage,
                        &account_storages_snapshots_dir,
                        &get_rocksdb_temp_storage_dir(&self.datadir)
                    ).await?;
                } else {
                    let maybe_big_account_storage_state_roots: Arc<Mutex<HashMap<H256, H256>>> =
                        Arc::new(Mutex::new(HashMap::new()));
                    insert_storages_into_db(
                        store.clone(),
                        &account_storages_snapshots_dir,
                        &maybe_big_account_storage_state_roots,
                        &pivot_header,
                    )
                    .await?;
                }
            }

            *METRICS.storage_tries_insert_end_time.lock().await = Some(SystemTime::now());

            info!("Finished storing storage tries");
        }

        *METRICS.heal_start_time.lock().await = Some(SystemTime::now());
        info!("Starting Healing Process");
        let mut global_state_leafs_healed: u64 = 0;
        let mut global_storage_leafs_healed: u64 = 0;
        let mut healing_done = false;
        while !healing_done {
            // This if is an edge case for the skip snap sync scenario
            if block_is_stale(&pivot_header) {
                pivot_header = update_pivot(
                    pivot_header.number,
                    pivot_header.timestamp,
                    &mut self.peers,
                    block_sync_state,
                )
                .await?;
            }
            healing_done = heal_state_trie_wrap(
                pivot_header.state_root,
                store.clone(),
                &self.peers,
                calculate_staleness_timestamp(pivot_header.timestamp),
                &mut global_state_leafs_healed,
                &mut storage_accounts,
                &mut code_hash_collector,
            )
            .await?;
            if !healing_done {
                continue;
            }
            healing_done = heal_storage_trie(
                pivot_header.state_root,
                &storage_accounts,
                &mut self.peers,
                store.clone(),
                HashMap::new(),
                calculate_staleness_timestamp(pivot_header.timestamp),
                &mut global_storage_leafs_healed,
            )
            .await;

            free_peers_and_log_if_not_empty(&self.peers).await;
        }
        *METRICS.heal_end_time.lock().await = Some(SystemTime::now());

        debug_assert!(validate_state_root(store.clone(), pivot_header.state_root).await);
        debug_assert!(validate_storage_root(store.clone(), pivot_header.state_root).await);
        info!("Finished healing");

        // Finish code hash collection
        code_hash_collector.finish().await?;

        *METRICS.bytecode_download_start_time.lock().await = Some(SystemTime::now());

        let code_hashes_dir = get_code_hashes_snapshots_dir(&self.datadir);
        let mut seen_code_hashes = HashSet::new();
        let mut code_hashes_to_download = Vec::new();

        info!("Starting download code hashes from peers");
        for entry in std::fs::read_dir(&code_hashes_dir)
            .map_err(|_| SyncError::CodeHashesSnapshotsDirNotFound)?
        {
            let entry = entry.map_err(|_| SyncError::CorruptPath)?;
            let snapshot_contents = std::fs::read(entry.path())
                .map_err(|err| SyncError::SnapshotReadError(entry.path(), err))?;
            let code_hashes: Vec<H256> = RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::CodeHashesSnapshotDecodeError(entry.path()))?;

            for hash in code_hashes {
                // If we haven't seen the code hash yet, add it to the list of hashes to download
                if seen_code_hashes.insert(hash) {
                    code_hashes_to_download.push(hash);

                    if code_hashes_to_download.len() >= BYTECODE_CHUNK_SIZE {
                        info!(
                            "Starting bytecode download of {} hashes",
                            code_hashes_to_download.len()
                        );
                        let bytecodes = self
                            .peers
                            .request_bytecodes(&code_hashes_to_download)
                            .await
                            .map_err(SyncError::PeerHandler)?
                            .ok_or(SyncError::BytecodesNotFound)?;

                        store
                            .write_account_code_batch(
                                code_hashes_to_download.drain(..).zip(bytecodes).collect(),
                            )
                            .await?;
                    }
                }
            }
        }

        // Download remaining bytecodes if any
        if !code_hashes_to_download.is_empty() {
            let bytecodes = self
                .peers
                .request_bytecodes(&code_hashes_to_download)
                .await
                .map_err(SyncError::PeerHandler)?
                .ok_or(SyncError::BytecodesNotFound)?;
            store
                .write_account_code_batch(
                    code_hashes_to_download.into_iter().zip(bytecodes).collect(),
                )
                .await?;
        }

        *METRICS.bytecode_download_end_time.lock().await = Some(SystemTime::now());

        debug_assert!(validate_bytecodes(store.clone(), pivot_header.state_root).await);

        store_block_bodies(vec![pivot_header.hash()], self.peers.clone(), store.clone()).await?;

        let block = store
            .get_block_by_hash(pivot_header.hash())
            .await?
            .ok_or(SyncError::CorruptDB)?;

        store.add_block(block).await?;

        let numbers_and_hashes = match block_sync_state {
            BlockSyncState::Full(_) => return Err(SyncError::NotInSnapSync),
            BlockSyncState::Snap(snap_block_sync_state) => snap_block_sync_state
                .block_hashes
                .iter()
                .rev()
                .enumerate()
                .map(|(i, hash)| (pivot_header.number - i as u64, *hash))
                .collect::<Vec<_>>(),
        };

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

type StorageRoots = (H256, Vec<(NodeHash, Vec<u8>)>);

fn compute_storage_roots(
    maybe_big_account_storage_state_roots: Arc<Mutex<HashMap<H256, H256>>>,
    store: Store,
    account_hash: H256,
    key_value_pairs: Vec<(H256, U256)>,
    pivot_hash: H256,
) -> Result<StorageRoots, SyncError> {
    let account_storage_root = match maybe_big_account_storage_state_roots
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

    let (computed_storage_root, changes) = storage_trie.collect_changes_since_last_hash();

    let account_state = store
        .get_account_state_by_acc_hash(pivot_hash, account_hash)?
        .ok_or(SyncError::AccountState(pivot_hash, account_hash))?;
    if computed_storage_root == account_state.storage_root {
        METRICS.storage_tries_state_roots_computed.inc();
    } else {
        maybe_big_account_storage_state_roots
            .lock()
            .map_err(|_| SyncError::MaybeBigAccount)?
            .insert(account_hash, computed_storage_root);
    }

    Ok((account_hash, changes))
}

pub async fn update_pivot(
    block_number: u64,
    block_timestamp: u64,
    peers: &mut PeerHandler,
    block_sync_state: &mut BlockSyncState,
) -> Result<BlockHeader, SyncError> {
    // We multiply the estimation by 0.9 in order to account for missing slots (~9% in tesnets)
    let new_pivot_block_number = block_number
        + ((current_unix_time().saturating_sub(block_timestamp) / SECONDS_PER_BLOCK) as f64
            * MISSING_SLOTS_PERCENTAGE) as u64;
    debug!(
        "Current pivot is stale (number: {}, timestamp: {}). New pivot number: {}",
        block_number, block_timestamp, new_pivot_block_number
    );
    loop {
        let (peer_id, mut peer_channel) = peers
            .peer_table
            .get_peer_channel_with_highest_score(&SUPPORTED_ETH_CAPABILITIES)
            .await
            .ok_or(SyncError::NoPeers)?;

        let peer_score = peers.peer_table.get_score(&peer_id).await;
        info!(
            "Trying to update pivot to {new_pivot_block_number} with peer {peer_id} (score: {peer_score})"
        );
        let Some(pivot) = peers
            .get_block_header(&mut peer_channel, new_pivot_block_number)
            .await
            .map_err(SyncError::PeerHandler)?
        else {
            // Penalize peer
            peers.peer_table.record_failure(peer_id).await;
            let peer_score = peers.peer_table.get_score(&peer_id).await;
            warn!(
                "Received None pivot from peer {peer_id} (score after penalizing: {peer_score}). Retrying"
            );
            continue;
        };

        // Reward peer
        peers.peer_table.record_success(peer_id).await;
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
        return Ok(pivot.clone());
    }
}

pub fn block_is_stale(block_header: &BlockHeader) -> bool {
    calculate_staleness_timestamp(block_header.timestamp) < current_unix_time()
}

pub fn calculate_staleness_timestamp(timestamp: u64) -> u64 {
    timestamp + (SNAP_LIMIT as u64 * 12)
}
#[derive(Debug, Default)]
/// We store for optimization the accounts that need to heal storage
pub struct AccountStorageRoots {
    /// The accounts that have not been healed are guaranteed to have the original storage root
    /// we can read this storage root
    pub accounts_with_storage_root: BTreeMap<H256, H256>,
    /// If an account has been healed, it may return to a previous state, so we just store the account
    /// in a hashset
    pub healed_accounts: HashSet<H256>,
}

#[derive(thiserror::Error, Debug)]
pub enum SyncError {
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
    #[error("Failed to read snapshot from {0:?} with error {1:?}")]
    SnapshotReadError(PathBuf, std::io::Error),
    #[error("Failed to RLP decode account_state_snapshot from {0:?}")]
    SnapshotDecodeError(PathBuf),
    #[error("Failed to RLP decode code_hashes_snapshot from {0:?}")]
    CodeHashesSnapshotDecodeError(PathBuf),
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
    #[error("Failed to get code hashes snapshots directory")]
    CodeHashesSnapshotsDirNotFound,
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
    #[error("Sorted Trie Generation Error: {0}")]
    TrieGenerationError(#[from] TrieGenerationError),
    #[error("Failed to get account temp db directory")]
    AccountTempDBDirNotFound,
    #[error("Failed to get storage temp db directory")]
    StorageTempDBDirNotFound,
    #[error("RocksDB Error: {0}")]
    RocksDBError(String),
    #[error("Bytecode file error")]
    BytecodeFileError,
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

pub async fn validate_storage_root(store: Store, state_root: H256) -> bool {
    info!("Starting validate_storage_root");
    let is_valid = store
        .clone()
        .iter_accounts(state_root)
        .expect("We should be able to open the store")
        .par_bridge()
        .map(|(hashed_address, account_state)|
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
        if !tree_validated {
            error!(
                "We have failed the validation of the storage tree {:x} expected but {computed_storage_root:x} found for the account {:x}",
                account_state.storage_root,
                hashed_address
            );
        }
        tree_validated
    })
    .all(|valid| valid);
    info!("Finished validate_storage_root");
    if !is_valid {
        std::process::exit(-1);
    }
    is_valid
}

pub async fn validate_bytecodes(store: Store, state_root: H256) -> bool {
    info!("Starting validate_bytecodes");
    let mut is_valid = true;
    for (account_hash, account_state) in store
        .iter_accounts(state_root)
        .expect("we couldn't iterate over accounts")
    {
        if account_state.code_hash != *EMPTY_KECCACK_HASH
            && !store
                .get_account_code(account_state.code_hash)
                .is_ok_and(|code| code.is_some())
        {
            error!(
                "Missing code hash {:x} for account {:x}",
                account_state.code_hash, account_hash
            );
            is_valid = false
        }
    }
    if !is_valid {
        std::process::exit(-1);
    }
    is_valid
}

async fn insert_accounts_into_db(
    store: Store,
    storage_accounts: &mut AccountStorageRoots,
    account_state_snapshots_dir: &Path,
) -> Result<H256, SyncError> {
    let mut computed_state_root = *EMPTY_TRIE_HASH;
    for entry in std::fs::read_dir(account_state_snapshots_dir)
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
    {
        let entry = entry
            .map_err(|err| SyncError::SnapshotReadError(account_state_snapshots_dir.into(), err))?;
        info!("Reading account file from entry {entry:?}");
        let snapshot_path = entry.path();
        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|err| SyncError::SnapshotReadError(snapshot_path.clone(), err))?;
        let account_states_snapshot: Vec<(H256, AccountState)> =
            RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        storage_accounts.accounts_with_storage_root.extend(
            account_states_snapshot.iter().filter_map(|(hash, state)| {
                (state.storage_root != *EMPTY_TRIE_HASH).then_some((*hash, state.storage_root))
            }),
        );

        info!("Inserting accounts into the state trie");

        let store_clone = store.clone();
        let current_state_root: Result<H256, SyncError> =
            tokio::task::spawn_blocking(move || -> Result<H256, SyncError> {
                let mut trie = store_clone.open_state_trie(computed_state_root)?;

                for (account_hash, account) in account_states_snapshot {
                    trie.insert(account_hash.0.to_vec(), account.encode_to_vec())?;
                }
                info!("Comitting to disk");
                let current_state_root = trie.hash()?;
                Ok(current_state_root)
            })
            .await?;

        computed_state_root = current_state_root?;
    }
    info!("computed_state_root {computed_state_root}");
    Ok(computed_state_root)
}

async fn insert_storages_into_db(
    store: Store,
    account_storages_snapshots_dir: &Path,
    maybe_big_account_storage_state_roots: &Arc<Mutex<HashMap<H256, H256>>>,
    pivot_header: &BlockHeader,
) -> Result<(), SyncError> {
    for entry in std::fs::read_dir(account_storages_snapshots_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?
    {
        let entry = entry.map_err(|err| {
            SyncError::SnapshotReadError(account_storages_snapshots_dir.into(), err)
        })?;
        info!("Reading account storage file from entry {entry:?}");

        let snapshot_path = entry.path();

        let snapshot_contents = std::fs::read(&snapshot_path)
            .map_err(|err| SyncError::SnapshotReadError(snapshot_path.clone(), err))?;

        let account_storages_snapshot: Vec<(H256, Vec<(H256, U256)>)> =
            RLPDecode::decode(&snapshot_contents)
                .map_err(|_| SyncError::SnapshotDecodeError(snapshot_path.clone()))?;

        let maybe_big_account_storage_state_roots_clone =
            maybe_big_account_storage_state_roots.clone();
        let store_clone = store.clone();
        let pivot_hash_moved = pivot_header.hash();
        info!("Starting compute of account_storages_snapshot");
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
        info!("Writing to db");

        store
            .write_storage_trie_nodes_batch(storage_trie_node_changes)
            .await?;
    }
    Ok(())
}

#[cfg(feature = "rocksdb")]
async fn insert_accounts_into_rocksdb(
    store: Store,
    storage_accounts: &mut AccountStorageRoots,
    account_state_snapshots_dir: &Path,
    temp_db_dir: &Path,
) -> Result<H256, SyncError> {
    use ethrex_trie::trie_sorted::trie_from_sorted_accounts_wrap;

    let trie = store.open_state_trie(*EMPTY_TRIE_HASH)?;
    let mut db_options = rocksdb::Options::default();
    db_options.create_if_missing(true);
    let db = rocksdb::DB::open(&db_options, temp_db_dir)
        .map_err(|_| SyncError::AccountTempDBDirNotFound)?;
    let file_paths: Vec<PathBuf> = std::fs::read_dir(account_state_snapshots_dir)
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SyncError::AccountStateSnapshotsDirNotFound)?
        .into_iter()
        .map(|res| res.path())
        .collect();
    db.ingest_external_file(file_paths)
        .map_err(|err| SyncError::RocksDBError(err.into_string()))?;
    let iter = db.full_iterator(rocksdb::IteratorMode::Start);
    trie_from_sorted_accounts_wrap(
        trie.db(),
        &mut iter
            .map(|k| k.expect("We shouldn't have a rocksdb error here")) // TODO: remove unwrap
            .inspect(|(k, v)| {
                METRICS
                    .account_tries_inserted
                    .fetch_add(1, Ordering::Relaxed);
                let account_state = AccountState::decode(v).expect("We should have accounts here");
                if account_state.storage_root != *EMPTY_TRIE_HASH {
                    storage_accounts
                        .accounts_with_storage_root
                        .insert(H256::from_slice(k), account_state.storage_root);
                }
            })
            .map(|(k, v)| (H256::from_slice(&k), v.to_vec())),
    )
    .map_err(SyncError::TrieGenerationError)
}

#[cfg(feature = "rocksdb")]
async fn insert_storage_into_rocksdb(
    store: Store,
    accounts_with_storage: BTreeSet<H256>,
    account_state_snapshots_dir: &Path,
    temp_db_dir: &Path,
) -> Result<(), SyncError> {
    use ethrex_trie::trie_sorted::trie_from_sorted_accounts_wrap;
    use rayon::iter::IntoParallelRefIterator;

    let mut db_options = rocksdb::Options::default();
    db_options.create_if_missing(true);
    let db = rocksdb::DB::open(&db_options, temp_db_dir)
        .map_err(|_| SyncError::StorageTempDBDirNotFound)?;
    let file_paths: Vec<PathBuf> = std::fs::read_dir(account_state_snapshots_dir)
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SyncError::AccountStoragesSnapshotsDirNotFound)?
        .into_iter()
        .map(|res| res.path())
        .collect();
    db.ingest_external_file(file_paths)
        .map_err(|err| SyncError::RocksDBError(err.into_string()))?;

    accounts_with_storage.par_iter().for_each(|account_hash| {
        let trie = store
            .open_storage_trie(*account_hash, *EMPTY_TRIE_HASH)
            .expect("Should be able to open trie");
        let iter = db.prefix_iterator(account_hash.as_bytes());
        let _ = trie_from_sorted_accounts_wrap(
            trie.db(),
            &mut iter
                .map(|k| k.expect("We shouldn't have a rocksdb error here")) // TODO: remove unwrap
                .map(|(k, v)| (H256::from_slice(&k[32..]), v.to_vec())),
        )
        .inspect_err(|err: &TrieGenerationError| {
            error!(
                "we found an error while inserting the storage trie for the account {account_hash}, err {err}"
            )
        })
        .map_err(SyncError::TrieGenerationError);
    });
    Ok(())
}
