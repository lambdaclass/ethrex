mod bytecode_fetcher;
mod state_healing;
mod state_sync;
mod storage_fetcher;
mod storage_healing;
mod trie_rebuild;

use bytecode_fetcher::bytecode_fetcher;
use ethrex_blockchain::{error::ChainError, Blockchain};
use ethrex_common::{
    types::{Block, BlockHash},
    BigEndianHash, H256, U256, U512,
};
use ethrex_rlp::error::RLPDecodeError;
use ethrex_storage::{error::StoreError, EngineType, Store, STATE_TRIE_SEGMENTS};
use ethrex_trie::{Nibbles, Node, TrieError, TrieState};
use state_healing::heal_state_trie;
use state_sync::state_sync;
use std::{array, collections::HashMap, sync::Arc};
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
    peer_handler::{
        BlockRequestOrder, PeerHandler, HASH_MAX, MAX_BLOCK_BODIES_TO_REQUEST,
        MAX_BLOCK_HEADERS_TO_REQUEST,
    },
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

#[derive(Debug, PartialEq)]
pub enum SyncMode {
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
    /// The `forkchoice_update` and `new_payload` methods require the `latest_valid_hash`
    /// when processing an invalid payload. To provide this, we must track invalid chains.
    ///
    /// We only store the last known valid head upon encountering a bad block,
    /// rather than tracking every subsequent invalid block.
    ///
    /// This map stores the bad block hash with and latest valid block hash of the chain corresponding to the bad block
    pub invalid_ancestors: HashMap<BlockHash, BlockHash>,
    trie_rebuilder: Option<TrieRebuilder>,
    // Used for cancelling long-living tasks upon shutdown
    cancel_token: CancellationToken,
    pub blockchain: Blockchain,
}

impl SyncManager {
    pub fn new(
        peer_table: Arc<Mutex<KademliaTable>>,
        sync_mode: SyncMode,
        cancel_token: CancellationToken,
        blockchain: Blockchain,
    ) -> Self {
        Self {
            sync_mode,
            peers: PeerHandler::new(peer_table),
            last_snap_pivot: 0,
            invalid_ancestors: HashMap::new(),
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
            invalid_ancestors: HashMap::new(),
            trie_rebuilder: None,
            // This won't be used
            cancel_token: CancellationToken::new(),
            blockchain: Blockchain::default_with_store(
                Store::new("", EngineType::InMemory).unwrap(),
            ),
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
        // We only check for snap syncing as full sync will start fetching headers starting from the canonical block
        // which is update every time a new block is added which is done as we fetch block headers
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

        loop {
            debug!("Requesting Block Headers from {current_head}");
            let block_header_limit = match self.sync_mode {
                SyncMode::Snap => MAX_BLOCK_HEADERS_TO_REQUEST,
                // In Full sync mode, request the same number of block bodies as headers,
                // since they are processed together at the same rate.
                SyncMode::Full => MAX_BLOCK_BODIES_TO_REQUEST,
            } as u64;

            let Some(mut block_headers) = self
                .peers
                .request_block_headers(
                    current_head,
                    BlockRequestOrder::OldToNew,
                    block_header_limit,
                )
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

            // Check if we already found the sync head
            let sync_head_found = block_hashes.contains(&sync_head);
            // Update current fetch head if needed
            let last_block_hash = last_block_header.compute_block_hash();
            if !sync_head_found {
                debug!(
                    "Syncing head not found, updated current_head {:?}",
                    last_block_hash
                );
                current_head = last_block_hash;
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
            store.add_block_headers(block_hashes.clone(), block_headers)?;

            if self.sync_mode == SyncMode::Full {
                if sync_head_found {
                    // Filter out everything after the sync_head
                    block_hashes = block_hashes
                        .iter()
                        .take_while(|&hash| *hash != sync_head)
                        .cloned()
                        .collect();
                }
                self.download_and_run_blocks(&mut block_hashes, store.clone())
                    .await?;
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
                    self.blockchain.add_block(&block)?;
                    store.set_canonical_block(block.header.number, *hash)?;
                    store.update_latest_block_number(block.header.number)?;
                }
                self.last_snap_pivot = pivot_header.number;
                // Finished a sync cycle without aborting halfway, clear current checkpoint
                store.clear_snap_state()?;
                // Next sync will be full-sync
                self.sync_mode = SyncMode::Full;
            }
            // Full sync stores and executes blocks as it ask for the headers
            SyncMode::Full => {}
        }
        Ok(())
    }

    /// Requests block bodies from peers via p2p, executes and stores them
    /// Returns an error if there was a problem while executing or validating the blocks
    async fn download_and_run_blocks(
        &mut self,
        block_hashes: &mut [BlockHash],
        store: Store,
    ) -> Result<(), SyncError> {
        let mut last_valid_hash = H256::default();

        let mut current_chunk_idx = 0;
        let chunks: Vec<Vec<BlockHash>> = block_hashes
            .chunks(MAX_BLOCK_BODIES_TO_REQUEST)
            .map(|chunk| chunk.to_vec())
            .collect();

        let mut chunk = match chunks.get(current_chunk_idx) {
            Some(res) => res.clone(),
            None => return Ok(()),
        };

        loop {
            debug!("Requesting Block Bodies");
            if let Some(block_bodies) = self.peers.request_block_bodies(chunk.clone()).await {
                let block_bodies_len = block_bodies.len();

                let first_block_hash = chunk.first().map_or(H256::default(), |a| *a);
                let first_block_header_number = store
                    .get_block_header_by_hash(first_block_hash)?
                    .map_or(0, |h| h.number);

                debug!(
                    "Received {} Block Bodies, starting from block hash {:?} with number: {}",
                    block_bodies_len, first_block_hash, first_block_header_number
                );

                // Execute and store blocks
                for (hash, body) in chunk
                    .drain(..block_bodies_len)
                    .zip(block_bodies.into_iter())
                {
                    let header = store
                        .get_block_header_by_hash(hash)?
                        .ok_or(SyncError::CorruptDB)?;
                    let number = header.number;
                    let block = Block::new(header, body);
                    if let Err(error) = self.blockchain.add_block(&block) {
                        warn!("Failed to add block during FullSync: {error}");
                        self.invalid_ancestors.insert(hash, last_valid_hash);
                        return Err(error.into());
                    }
                    store.set_canonical_block(number, hash)?;
                    store.update_latest_block_number(number)?;
                    last_valid_hash = hash;
                    debug!(
                        "Executed and stored block number {} with hash {}",
                        number, hash
                    );
                }
                debug!("Executed & stored {} blocks", block_bodies_len);

                if chunk.is_empty() {
                    current_chunk_idx += 1;
                    chunk = match chunks.get(current_chunk_idx) {
                        Some(res) => res.clone(),
                        None => return Ok(()),
                    };
                };
            }
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
    #[error(transparent)]
    SendHashes(#[from] SendError<Vec<H256>>),
    #[error(transparent)]
    SendStorage(#[from] SendError<Vec<(H256, H256)>>),
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
}
