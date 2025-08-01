mod state_healing;
mod storage_healing;

use crate::metrics::METRICS;
use crate::rlpx::p2p::SUPPORTED_ETH_CAPABILITIES;
use crate::sync::state_healing::{heal_state_trie, SHOW_PROGRESS_INTERVAL_DURATION};
use crate::sync::storage_healing::heal_storage_trie;
use crate::{
    peer_handler::{HASH_MAX, MAX_BLOCK_BODIES_TO_REQUEST, PeerHandler, SNAP_LIMIT},
    utils::current_unix_time,
};
use aes::cipher::consts::U2;
use ethrex_blockchain::{BatchBlockProcessingFailure, Blockchain, error::ChainError};
use ethrex_common::{
    BigEndianHash, H256, U256,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountState, Block, BlockHash, BlockHeader},
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{EngineType, STATE_TRIE_SEGMENTS, Store, error::StoreError};
use ethrex_trie::{Nibbles, Node, Trie, TrieDB, TrieError};
use futures::FutureExt;
use std::collections::HashSet;
use std::str::FromStr;
use std::thread::Scope;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
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
            self.sync_cycle_snap(sync_head, store).await
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
        let current_head_number = store.get_block_number(current_head).await.unwrap().unwrap();
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
                            .await?
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
            self.snap_sync(store, block_sync_state).await?;

            // Next sync will be full-sync
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
        let current_head_number = store.get_block_number(current_head).await.unwrap().unwrap();
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

    async fn snap_sync(
        &mut self,
        store: Store,
        block_sync_state: BlockSyncState,
    ) -> Result<(), SyncError> {
        // snap-sync: launch tasks to fetch blocks and state in parallel
        // - Fetch each block's body and its receipt via eth p2p requests
        // - Fetch the pivot block's state via snap p2p requests
        // - Execute blocks after the pivot (like in full-sync)
        let all_block_hashes = block_sync_state.into_snap_block_hashes();
        let pivot_idx = all_block_hashes.len().saturating_sub(1);
        let mut pivot_header = store
            .get_block_header_by_hash(all_block_hashes[pivot_idx])?
            .ok_or(SyncError::CorruptDB)?;

        let mut staleness_timestamp: u64 = pivot_header.timestamp + (SNAP_LIMIT as u64 * 12);
        while current_unix_time() > staleness_timestamp {
            (pivot_header, staleness_timestamp) =
                update_pivot(pivot_header.number, &self.peers).await;
        }

        let pivot_number = pivot_header.number;
        let pivot_hash = pivot_header.hash();
        debug!("Selected block {pivot_number} as pivot for snap sync");

        let state_root = pivot_header.state_root;

        let mut pivot_is_stale = self.peers
            .request_account_range(pivot_header.clone(), H256::zero(), H256::repeat_byte(0xff))
            .await;

        let empty = *EMPTY_TRIE_HASH;

        let mut chunk_index = 0;
        let mut downloaded_account_storages = 0;
        for entry in std::fs::read_dir("/home/admin/.local/share/ethrex/account_state_snapshots/")
            .expect("Failed to read account_state_snapshots dir")
        {
            if pivot_is_stale {
                info!("Skipping rest of storage downloads due to staleness");
                break;
            }
            let entry = entry.expect("Failed to read dir entry");

            let snapshot_path = entry.path();

            let snapshot_contents = std::fs::read(&snapshot_path)
                .unwrap_or_else(|_| panic!("Failed to read snapshot from {snapshot_path:?}"));

            let account_states_snapshot: Vec<(H256, AccountState)> =
                RLPDecode::decode(&snapshot_contents).unwrap_or_else(|_| {
                    panic!("Failed to RLP decode account_state_snapshot from {snapshot_path:?}")
                });

            let (account_hashes, account_states): (Vec<H256>, Vec<AccountState>) =
                account_states_snapshot.iter().cloned().unzip();
            
            let account_storage_roots: Vec<(H256, H256)> = account_hashes
                .iter()
                .zip(account_states.iter())
                .filter_map(|(hash, state)| {
                    (state.storage_root != empty).then_some((*hash, state.storage_root))
                })
                .collect();

            downloaded_account_storages += account_storage_roots.len();
    
            (chunk_index, pivot_is_stale) = self.peers
                .request_storage_ranges(pivot_header.clone(), account_storage_roots.clone(), chunk_index)
                .await;
        }


        info!("Starting to compute the state root...");

        let account_store_start = Instant::now();

        let mut computed_state_root = *EMPTY_TRIE_HASH;
        let mut bytecode_hashes = Vec::new();

        // for entry in std::fs::read_dir("/home/admin/.local/share/ethrex/account_state_snapshots/")
        //     .expect("Failed to read account_state_snapshots dir")
        // {
        //     let entry = entry.expect("Failed to read dir entry");
        //     info!("Started reading account_state_snapshots entry {}", entry.file_name().to_str().expect("we should have a name"));

        //     let snapshot_path = entry.path();

        //     let snapshot_contents = std::fs::read(&snapshot_path)
        //         .unwrap_or_else(|_| panic!("Failed to read snapshot from {snapshot_path:?}"));

        //     let account_state_snapshot: Vec<(H256, AccountState)> =
        //         RLPDecode::decode(&snapshot_contents).unwrap_or_else(|_| {
        //             panic!("Failed to RLP decode account_state_snapshot from {snapshot_path:?}")
        //         });

        //     let trie = store.open_state_trie(computed_state_root).unwrap();

        //     let (current_state_root, current_bytecode_hashes) =
        //         tokio::task::spawn_blocking(move || {
        //             let mut bytecode_hashes = vec![];
        //             let mut trie = trie;
        //             let mut counter = 0;
        //             let mut instant = Instant::now();

        //             for (account_hash, account) in account_state_snapshot {
        //                 counter += 1;
        //                 if instant.elapsed() > SHOW_PROGRESS_INTERVAL_DURATION {
        //                     instant = Instant::now();
        //                     info!("We have read and inserted {counter} accounts");
        //                 }
        //                 if account.code_hash != *EMPTY_KECCACK_HASH {
        //                     bytecode_hashes.push(account.code_hash);
        //                 }
        //                 trie.insert(account_hash.0.to_vec(), account.encode_to_vec())
        //                     .expect("We should be inserting");
        //             }
        //             info!("We have finished inserting in the trie, getting the hash");
        //             let current_state_root = trie.hash().unwrap();
        //             // TODO: readd this, potentialy in another place
        //             // bytecode_hashes.sort();
        //             // bytecode_hashes.dedup();
        //             (current_state_root, bytecode_hashes)
        //         })
        //         .await
        //         .expect("This shouldn't have an error");

        //     info!("We have finished computing the current state root {current_state_root}");
        //     computed_state_root = current_state_root;
        //     //bytecode_hashes.extend(&current_bytecode_hashes);
        // }

        // *METRICS.account_tries_state_root.lock().await = Some(computed_state_root);

        // let account_store_time = Instant::now().saturating_duration_since(account_store_start);

        // info!("Expected state root: {state_root:?}");
        // info!("Computed state root: {computed_state_root:?} in {account_store_time:?}");

        let storages_store_start = Instant::now();

        METRICS
            .storage_tries_state_roots_start_time
            .lock()
            .await
            .replace(SystemTime::now());

        *METRICS.storage_tries_state_roots_to_compute.lock().await =
            downloaded_account_storages as u64;

        let maybe_big_account_storage_state_roots: Arc<Mutex<HashMap<H256, H256>>> =
            Arc::new(Mutex::new(HashMap::new()));

        for entry in
            std::fs::read_dir("/home/admin/.local/share/ethrex/account_storages_snapshots/")
                .expect("Failed to read account_storages_snapshots dir")
        {
            let entry = entry.expect("Failed to read dir entry");

            let snapshot_path = entry.path();

            let snapshot_contents = std::fs::read(&snapshot_path)
                .unwrap_or_else(|_| panic!("Failed to read snapshot from {snapshot_path:?}"));

            let account_storages_snapshot: Vec<(H256, Vec<(H256, U256)>)> = RLPDecode::decode(&snapshot_contents).unwrap_or_else(|_| {
                panic!("Failed to RLP decode account_state_snapshot from {snapshot_path:?}")
            });

            let maybe_big_account_storage_state_roots_clone =
                maybe_big_account_storage_state_roots.clone();
            let store_clone = store.clone();
            let storage_trie_node_changes = tokio::task::spawn_blocking(move || {
                let store: Store = store_clone;

                let (sender, receiver) = std::sync::mpsc::channel();

                // TODO: Here we are filtering again the account with empty storage because we are adding empty accounts on purpose (it was the easiest thing to do)
                // We need to fix this issue in request_storage_ranges and remove this filter.
                account_storages_snapshot
                    .into_par_iter()
                    .filter(|(_account_hash, storage)| !storage.is_empty())
                    .for_each_with(sender, |sender, (account_hash, key_value_pairs)| {
                        let account_storage_root = match maybe_big_account_storage_state_roots_clone.lock().expect("Failed to acquire lock").entry(account_hash) {
                            Entry::Occupied(occupied_entry) => *occupied_entry.get(),
                            Entry::Vacant(_vacant_entry) => *EMPTY_TRIE_HASH,
                        };
    
                        let mut storage_trie = store
                            .open_storage_trie(account_hash, account_storage_root)
                            .unwrap_or_else(|_| panic!("Failed to open trie storage for account hash {account_hash}"));
    
                        for (hashed_key, value) in key_value_pairs {
                            if let Err(err) = storage_trie.insert(hashed_key.0.to_vec(), value.encode_to_vec()) {
                                error!(
                                    "Failed to insert hashed key {hashed_key:?} in account hash: {account_hash:?}, err={err:?}"
                                );
                                continue;
                            }
                        }
    
                        let (computed_state_root, changes) =
                            storage_trie.collect_changes_since_last_hash();
    
                        maybe_big_account_storage_state_roots_clone.lock().expect("Failed to acquire lock").insert(account_hash, computed_state_root);
    
                        METRICS.storage_tries_state_roots_computed.inc();
    
                        sender.send((account_hash, changes)).expect("Failed to send changes");
                    });

                receiver
                    .iter()
                    .collect::<Vec<_>>()
            }).await.expect("");

            store
                .write_storage_trie_nodes_batch(storage_trie_node_changes)
                .await?;
        }

        // for (account_hash, expected_storage_root) in &account_storage_roots {
        //     let mut binding = maybe_big_account_storage_state_roots
        //         .lock()
        //         .expect("Failed to acquire lock");

        //     let computed_storage_root = binding.entry(*account_hash).or_default();

        //     if *computed_storage_root != *expected_storage_root {
        //         error!(
        //             "Got different state roots for account hash: {account_hash:?}, expected: {expected_storage_root:?}, computed: {computed_storage_root:?}"
        //         );
        //     }
        // }

        METRICS
            .storage_tries_state_roots_end_time
            .lock()
            .await
            .replace(SystemTime::now());

        let storages_store_time = Instant::now().saturating_duration_since(storages_store_start);
        info!("Finished storing storage tries in: {storages_store_time:?}");

        // If we need to, we star to heal now.
        if pivot_is_stale {
            info!("pivot is stale, starting healing process");

            let mut healing_done = false;
            while !healing_done {
                (pivot_header, staleness_timestamp) =
                    update_pivot(pivot_header.number, &self.peers).await;
                healing_done = heal_state_trie_wrap(
                    pivot_header.state_root,
                    store.clone(),
                    &self.peers,
                    staleness_timestamp,
                )
                .await?;
                if !healing_done {
                    continue;
                }
                // TODO: 💀💀💀 either remove or change to a debug flag
                validate_state_root(store.clone(), pivot_header.state_root).await;
                healing_done = heal_storage_trie_wrap(
                    pivot_header.state_root,
                    store.clone(),
                    &self.peers,
                    staleness_timestamp,
                )
                .await?;
            }
            info!("Finished healing");
        }

        // Download bytecodes
        info!(
            "Starting bytecode download of {} hashes",
            bytecode_hashes.len()
        );
        let bytecodes = self
            .peers
            .request_bytecodes(&bytecode_hashes)
            .await
            .unwrap();

        store
            .write_account_code_batch(bytecode_hashes.into_iter().zip(bytecodes).collect())
            .await?;

        Ok(())
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
    ) -> Result<(), SyncError> {
        info!("Processing incoming headers full sync");
        self.current_headers.extend(block_headers);
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
            self.current_blocks.first().unwrap().hash(),
            self.current_blocks.last().unwrap().hash()
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
        // Mark chain as canonical & last block as latest
        self.store
            .mark_chain_as_canonical(&numbers_and_hashes)
            .await?;
        self.store
            .update_latest_block_number(last_block_number)
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

async fn heal_state_trie_wrap(
    state_root: H256,
    store: Store,
    peers: &PeerHandler,
    staleness_timestamp: u64,
) -> Result<bool, SyncError> {
    let mut healing_done = false;
    info!("Starting state healing");
    while !healing_done {
        healing_done = heal_state_trie(
            state_root,
            store.clone(),
            peers.clone(),
            staleness_timestamp,
        )
        .await?;
        if current_unix_time() > staleness_timestamp {
            info!("Stopped state healing due to staleness");
            break;
        }
    }
    info!("Stopped state healing");
    Ok(healing_done)
}


async fn heal_storage_trie_wrap(
    state_root: H256,
    store: Store,
    peers: &PeerHandler,
    time_limit: u64,
) -> Result<bool, SyncError> {
    let mut healing_done = false;
    info!("Starting storage healing");
    while !healing_done {
        healing_done = heal_storage_trie(
            state_root,
            peers.clone(),
            store.clone(),
            CancellationToken::new(),
            Arc::new(AtomicBool::new(true)),
        )
        .await?;
        if current_unix_time() > time_limit {
            info!("Stopped storage healing due to staleness");
            break;
        }
    }
    info!("Stopped storage healing");
    Ok(healing_done)
}

async fn update_pivot(block_number: u64, peers: &PeerHandler) -> (BlockHeader, u64) {
    // We ask for a pivot which is slightly behind the limit. This is because our peers may not have the
    // latest one, or a slot was missed
    let new_pivot_block_number = block_number + SNAP_LIMIT as u64 - 3;
    loop {
        let mut scores = peers.peer_scores.lock().await;

        let (peer_id, mut peer_channel) = peers
            .get_peer_channel_with_highest_score(&SUPPORTED_ETH_CAPABILITIES, &mut scores)
            .await
            .ok_or_else(|| error!("We aren't finding get_peer_channel_with_retry"))
            .expect("Error");

        let peer_score = scores.get(&peer_id).unwrap_or(&i64::MIN);
        info!(
            "Trying to update pivot to {new_pivot_block_number} with peer {peer_id} (score: {peer_score})"
        );
        let Some(pivot) = peers
            .get_block_header(&mut peer_channel, new_pivot_block_number)
            .await
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
        return (pivot.clone(), pivot.timestamp + (SNAP_LIMIT as u64 * 12));
    }
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
    #[error("Corrupt Path")]
    CorruptPath,
}

impl<T> From<SendError<T>> for SyncError {
    fn from(value: SendError<T>) -> Self {
        Self::Send(value.to_string())
    }
}

/// Returns the partial paths to the node's children if they are not already part of the trie state
pub fn node_missing_children(
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

pub async fn validate_state_root(store: Store, state_root: H256) -> bool {
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
            "We have failed the validation of the tree {state_root} expected but {computed_state_root} found"
        );
    }
    tree_validated
}
