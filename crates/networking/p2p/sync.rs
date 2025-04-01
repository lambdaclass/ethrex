mod bytecode_fetcher;
mod state_healing;
mod state_sync;
mod storage_fetcher;
mod storage_healing;
mod trie_rebuild;

use bytecode_fetcher::bytecode_fetcher;
use ethrex_blockchain::{error::ChainError, BatchBlockProcessingFailure, Blockchain};
use ethrex_common::{
    types::{Block, BlockHash, BlockHeader},
    BigEndianHash, H256, U256, U512,
};
use ethrex_rlp::error::RLPDecodeError;
use ethrex_storage::{error::StoreError, EngineType, Store, STATE_TRIE_SEGMENTS};
use ethrex_trie::{Nibbles, Node, TrieError, TrieState};
use state_healing::heal_state_trie;
use state_sync::state_sync;
use std::{array, sync::Arc};
use storage_healing::storage_healer;
use tokio::{
    sync::{
        mpsc::{self, error::SendError},
        Mutex,
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use trie_rebuild::TrieRebuilder;

use crate::{
    kademlia::KademliaTable,
    peer_handler::{BlockRequestOrder, PeerHandler, HASH_MAX, MAX_BLOCK_BODIES_TO_REQUEST},
};

/// The minimum amount of blocks from the head that we want to full sync during a snap sync
const MIN_FULL_BLOCKS: usize = 64;
/// Max size of a bach to stat a fetch request in queues
const BATCH_SIZE: usize = 300;
/// Max size of a bach to stat a fetch request in queues for nodes
const NODE_BATCH_SIZE: usize = 900;
/// Maximum amount of concurrent paralell fetches for a queue
const MAX_PARALLEL_FETCHES: usize = 10;
/// Maximum amount of messages in a channel
const MAX_CHANNEL_MESSAGES: usize = 500;
/// Maximum amount of messages to read from a channel at once
const MAX_CHANNEL_READS: usize = 200;
/// Pace at which progress is shown via info tracing
const SHOW_PROGRESS_INTERVAL_DURATION: Duration = Duration::from_secs(30);

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
pub struct SyncManager {
    sync_mode: SyncMode,
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

impl SyncManager {
    pub fn new(
        peer_table: Arc<Mutex<KademliaTable>>,
        sync_mode: SyncMode,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
    ) -> Self {
        Self {
            sync_mode,
            peers: PeerHandler::new(peer_table),
            last_snap_pivot: 0,
            trie_rebuilder: None,
            cancel_token,
            blockchain,
        }
    }

    /// Creates a dummy SyncManager for tests where syncing is not needed
    /// This should only be used in tests as it won't be able to connect to the p2p network
    pub fn dummy() -> Self {
        let dummy_peer_table = Arc::new(Mutex::new(KademliaTable::new(Default::default())));
        Self {
            sync_mode: SyncMode::Full,
            peers: PeerHandler::new(dummy_peer_table),
            last_snap_pivot: 0,
            trie_rebuilder: None,
            // This won't be used
            cancel_token: CancellationToken::new(),
            blockchain: Arc::new(Blockchain::default_with_store(
                Store::new("", EngineType::InMemory).unwrap(),
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
    pub async fn start_sync(&mut self, current_head: H256, sync_head: H256, store: Store) {
        info!("Syncing from current head {current_head} to sync_head {sync_head}");
        let start_time = Instant::now();
        match self.sync_cycle(current_head, sync_head, store).await {
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
    async fn sync_cycle(
        &mut self,
        mut current_head: H256,
        sync_head: H256,
        store: Store,
    ) -> Result<(), SyncError> {
        // Request all block headers between the current head and the sync head
        // We will begin from the current head so that we download the earliest state first
        // This step is not parallelized
        let mut all_block_hashes = vec![];
        // Check if we have some blocks downloaded from a previous sync attempt
        // This applies only to snap sync—full sync always starts fetching headers
        // from the canonical block, which updates as new block headers are fetched.
        if matches!(self.sync_mode, SyncMode::Snap) {
            if let Some(last_header) = store.get_header_download_checkpoint()? {
                // Set latest downloaded header as current head for header fetching
                current_head = last_header;
            }
        }

        let pending_block = match store.get_pending_block(sync_head) {
            Ok(res) => res,
            Err(e) => return Err(e.into()),
        };

        // TODO(#2126): To avoid modifying the current_head while backtracking we use a separate search_head
        let mut search_head = current_head;

        loop {
            debug!("Requesting Block Headers from {search_head}");

            let Some(mut block_headers) = self
                .peers
                .request_block_headers(search_head, BlockRequestOrder::OldToNew)
                .await
            else {
                warn!("Sync failed to find target block header, aborting");
                return Ok(());
            };

            let first_block_header = match block_headers.first() {
                Some(header) => header.clone(),
                None => continue,
            };
            let last_block_header = match block_headers.last() {
                Some(header) => header.clone(),
                None => continue,
            };
            // TODO(#2126): This is just a temporary solution to avoid a bug where the sync would get stuck
            // on a loop when the target head is not found, i.e. on a reorg with a side-chain.
            if first_block_header == last_block_header
                && first_block_header.compute_block_hash() == search_head
                && search_head != sync_head
            {
                // There is no path to the sync head this goes back until it find a common ancerstor
                warn!("Sync failed to find target block header, going back to the previous parent");
                search_head = first_block_header.parent_hash;
                continue;
            }

            let mut block_hashes = block_headers
                .iter()
                .map(|header| header.compute_block_hash())
                .collect::<Vec<_>>();

            debug!(
                "Received {} block headers| First Number: {} Last Number: {}",
                block_headers.len(),
                first_block_header.number,
                last_block_header.number
            );

            // If we have a pending block from new_payload request
            // attach it to the end if it matches the parent_hash of the latest received header
            if let Some(ref block) = pending_block {
                if block.header.parent_hash == last_block_header.compute_block_hash() {
                    block_hashes.push(block.hash());
                    block_headers.push(block.header.clone());
                }
            }

            // Filter out everything after the sync_head
            let mut sync_head_found = false;
            if let Some(index) = block_hashes.iter().position(|&hash| hash == sync_head) {
                sync_head_found = true;
                block_hashes = block_hashes.iter().take(index + 1).cloned().collect();
            }

            // Update current fetch head if needed
            let last_block_hash = last_block_header.compute_block_hash();
            if !sync_head_found {
                debug!(
                    "Syncing head not found, updated current_head {:?}",
                    last_block_hash
                );
                search_head = last_block_hash;
                current_head = last_block_hash;
                if self.sync_mode == SyncMode::Snap {
                    store.set_header_download_checkpoint(current_head)?;
                }
            }

            // If the sync head is less than 64 blocks away from our current head switch to full-sync
            if self.sync_mode == SyncMode::Snap {
                let latest_block_number = store.get_latest_block_number()?;
                if last_block_header.number.saturating_sub(latest_block_number)
                    < MIN_FULL_BLOCKS as u64
                {
                    // Too few blocks for a snap sync, switching to full sync
                    store.clear_snap_state()?;
                    self.sync_mode = SyncMode::Full
                }
            }

            // Discard the first header as we already have it
            block_hashes.remove(0);
            block_headers.remove(0);
            // Store headers and save hashes for full block retrieval
            all_block_hashes.extend_from_slice(&block_hashes[..]);
            // This step is necessary for full sync because some opcodes depend on previous blocks during execution.
            store.add_block_headers(block_hashes.clone(), block_headers.clone())?;

            if self.sync_mode == SyncMode::Full {
                let last_block_hash = self
                    .download_and_run_blocks(
                        &block_hashes,
                        &block_headers,
                        sync_head,
                        sync_head_found,
                        store.clone(),
                    )
                    .await?;
                if let Some(last_block_hash) = last_block_hash {
                    current_head = last_block_hash;
                    search_head = current_head;
                }
            }

            if sync_head_found {
                break;
            };
        }
        match self.sync_mode {
            SyncMode::Snap => {
                // snap-sync: launch tasks to fetch blocks and state in parallel
                // - Fetch each block's body and its receipt via eth p2p requests
                // - Fetch the pivot block's state via snap p2p requests
                // - Execute blocks after the pivot (like in full-sync)
                let pivot_idx = all_block_hashes.len().saturating_sub(MIN_FULL_BLOCKS);
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
                        .get_block_by_hash(*hash)?
                        .ok_or(SyncError::CorruptDB)?;
                    let block_number = block.header.number;
                    self.blockchain.add_block(&block)?;
                    store.set_canonical_block(block_number, *hash)?;
                    store.update_latest_block_number(block_number)?;
                }
                self.last_snap_pivot = pivot_header.number;
                // Finished a sync cycle without aborting halfway, clear current checkpoint
                store.clear_snap_state()?;
                // Next sync will be full-sync
                self.sync_mode = SyncMode::Full;
            }
            // Full sync stores and executes blocks as it asks for the headers
            SyncMode::Full => {}
        }
        Ok(())
    }

    /// Attempts to fetch up to 1024 block bodies from peers via P2P, starting from the sync head.
    /// Executes and stores the retrieved blocks.
    ///
    /// Returns an error if execution or validation fails.
    /// On success, returns the hash of the last successfully executed block body.
    async fn download_and_run_blocks(
        &mut self,
        block_hashes: &[BlockHash],
        block_headers: &[BlockHeader],
        sync_head: BlockHash,
        sync_head_found: bool,
        store: Store,
    ) -> Result<Option<H256>, SyncError> {
        let mut current_chunk_idx = 0;
        let block_hashes_chunks: Vec<Vec<BlockHash>> = block_hashes
            .chunks(MAX_BLOCK_BODIES_TO_REQUEST)
            .map(|chunk| chunk.to_vec())
            .collect();

        let mut current_block_hashes_chunk = match block_hashes_chunks.get(current_chunk_idx) {
            Some(res) => res.clone(),
            None => return Ok(None),
        };
        let mut headers_iter = block_headers.iter();
        let mut blocks: Vec<Block> = vec![];

        let since = Instant::now();
        loop {
            debug!("Requesting Block Bodies");
            let Some(block_bodies) = self
                .peers
                .request_block_bodies(current_block_hashes_chunk.clone())
                .await
            else {
                break;
            };

            let block_bodies_len = block_bodies.len();

            let first_block_hash = current_block_hashes_chunk
                .first()
                .map_or(H256::default(), |a| *a);

            debug!(
                "Received {} Block Bodies, starting from block hash {:?}",
                block_bodies_len, first_block_hash
            );

            // Push blocks
            for (_, body) in current_block_hashes_chunk
                .drain(..block_bodies_len)
                .zip(block_bodies)
            {
                let header = headers_iter.next().ok_or(SyncError::BodiesNotFound)?;
                let block = Block::new(header.clone(), body);
                blocks.push(block);
            }

            if current_block_hashes_chunk.is_empty() {
                current_chunk_idx += 1;
                current_block_hashes_chunk = match block_hashes_chunks.get(current_chunk_idx) {
                    Some(res) => res.clone(),
                    None => break,
                };
            };
        }

        let blocks_len = blocks.len();
        debug!(
            "Starting to execute and validate {} blocks in batch",
            blocks_len
        );
        let Some(first_block) = blocks.first().cloned() else {
            return Err(SyncError::BodiesNotFound);
        };
        let Some(last_block) = blocks.last().cloned() else {
            return Err(SyncError::BodiesNotFound);
        };

        // To ensure proper execution, we set the chain as canonical before processing the blocks.
        // Some opcodes rely on previous block hashes, and due to our current setup, we only support a single chain (no sidechains).
        // As a result, we must store the headers and set the chain upfront to writing to the database during execution.
        // Each write operation introduces overhead no matter how small.
        //
        // For more details, refer to the `get_block_hash` function in [`LevmDatabase`] and the [`revm::Database`].
        store
            .mark_chain_as_canonical(&blocks)
            .map_err(SyncError::Store)?;

        // Executing blocks is a CPU heavy operation
        // Spawn a blocking task to not block the tokio runtime
        let res: Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> = {
            let blockchain = self.blockchain.clone();
            tokio::task::spawn_blocking(move || {
                Self::add_blocks(blockchain, &blocks, sync_head_found)
            })
            .await
            .map_err(SyncError::JoinHandle)
        }?;

        if let Err((error, failure)) = res {
            warn!("Failed to add block during FullSync: {error}");
            if let Some(BatchBlockProcessingFailure {
                failed_block_hash,
                last_valid_hash,
            }) = failure
            {
                store.set_invalid_ancestor(failed_block_hash, last_valid_hash)?;

                // TODO(#2127): Just marking the failing ancestor and the sync head is enough
                // to fix the Missing Ancestors hive test, we want to look at a more robust
                // solution in the future if needed.
                store.set_invalid_ancestor(sync_head, last_valid_hash)?;
            }

            return Err(error.into());
        }

        store.update_latest_block_number(last_block.header.number)?;

        let elapsed_secs: f64 = since.elapsed().as_millis() as f64 / 1000.0;
        let blocks_per_second = blocks_len as f64 / elapsed_secs;

        info!(
            "[SYNCING] Requested, stored, and executed {} blocks in {:.3} seconds.\n\
            Started at block with hash {} (number {}).\n\
            Finished at block with hash {} (number {}).\n\
            Blocks per second: {:.3}",
            blocks_len,
            elapsed_secs,
            first_block.hash(),
            first_block.header.number,
            last_block.hash(),
            last_block.header.number,
            blocks_per_second
        );

        Ok(Some(last_block.hash()))
    }

    fn add_blocks(
        blockchain: Arc<Blockchain>,
        blocks: &[Block],
        sync_head_found: bool,
    ) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
        // If we found the sync head, run the blocks sequentially to store all the blocks's state
        if sync_head_found {
            let mut last_valid_hash = H256::default();
            for block in blocks {
                blockchain.add_block(block).map_err(|e| {
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
            blockchain.add_blocks_in_batch(blocks)
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
                store.add_block_body(hash, body)?;
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
                store.add_receipts(block_hash, receipts)?;
            }
            // Check if we need to ask for another batch
            if block_hashes.is_empty() {
                break;
            }
        }
    }
    Ok(())
}

impl SyncManager {
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
        let (storage_healer_sender, storage_healer_receiver) =
            mpsc::channel::<Vec<H256>>(MAX_CHANNEL_MESSAGES);
        let storage_healer_handler = tokio::spawn(storage_healer(
            state_root,
            storage_healer_receiver,
            self.peers.clone(),
            store.clone(),
        ));
        // Perform state sync if it was not already completed on a previous cycle
        // Retrieve storage data to check which snap sync phase we are in
        let key_checkpoints = store.get_state_trie_key_checkpoint()?;
        // If we have no key checkpoints or if the key checkpoints are lower than the segment boundaries we are in state sync phase
        if key_checkpoints.is_none()
            || key_checkpoints.is_some_and(|ch| {
                ch.into_iter()
                    .zip(STATE_TRIE_SEGMENTS_END.into_iter())
                    .any(|(ch, end)| ch < end)
            })
        {
            let stale_pivot = state_sync(
                state_root,
                store.clone(),
                self.peers.clone(),
                key_checkpoints,
                self.trie_rebuilder
                    .as_ref()
                    .unwrap()
                    .storage_rebuilder_sender
                    .clone(),
                storage_healer_sender.clone(),
            )
            .await?;
            if stale_pivot {
                warn!("Stale Pivot, aborting state sync");
                storage_healer_sender.send(vec![]).await?;
                storage_healer_handler.await??;
                return Ok(false);
            }
        }
        // Wait for the trie rebuilder to finish
        info!("Waiting for the trie rebuild to finish");
        let rebuild_start = Instant::now();
        self.trie_rebuilder.take().unwrap().complete().await?;
        info!(
            "State trie rebuilt from snapshot, overtime: {}",
            rebuild_start.elapsed().as_secs()
        );
        // Clear snapshot
        store.clear_snapshot()?;

        // Perform Healing
        let state_heal_complete = heal_state_trie(
            state_root,
            store.clone(),
            self.peers.clone(),
            storage_healer_sender.clone(),
        )
        .await?;
        // Send empty batch to signal that no more batches are incoming
        storage_healer_sender.send(vec![]).await?;
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
    trie_state: &TrieState,
) -> Result<Vec<Nibbles>, TrieError> {
    let mut paths = Vec::new();
    match &node {
        Node::Branch(node) => {
            for (index, child) in node.choices.iter().enumerate() {
                if child.is_valid() && trie_state.get_node(child.clone())?.is_none() {
                    paths.push(parent_path.append_new(index as u8));
                }
            }
        }
        Node::Extension(node) => {
            if node.child.is_valid() && trie_state.get_node(node.child.clone())?.is_none() {
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
}

impl<T> From<SendError<T>> for SyncError {
    fn from(value: SendError<T>) -> Self {
        Self::Send(value.to_string())
    }
}
