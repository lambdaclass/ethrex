//! Sync module - orchestrates full and snap synchronization
//!
//! This module provides the main `Syncer` type that coordinates synchronization
//! between full sync mode (all blocks executed) and snap sync mode (state fetched
//! via snap protocol).

mod code_collector;
mod full;
mod healing;
mod snap_sync;

use crate::metrics::METRICS;
use crate::peer_handler::{PeerHandler, PeerHandlerError};
use crate::peer_table::PeerTableError;
use crate::snap::constants::EXECUTE_BATCH_SIZE_DEFAULT;
use crate::utils::delete_leaves_folder;
use ethrex_blockchain::{Blockchain, error::ChainError};
use ethrex_common::{BigEndianHash, H256, U256};
use ethrex_rlp::error::RLPDecodeError;
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::TrieError;
use ethrex_trie::trie_sorted::TrieGenerationError;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc::error::SendError;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

// Re-export types used by submodules
pub use snap_sync::{
    SnapBlockSyncState, block_is_stale, calculate_staleness_timestamp, update_pivot,
    validate_bytecodes, validate_state_root, validate_storage_root,
};

#[cfg(feature = "sync-test")]
lazy_static::lazy_static! {
    static ref EXECUTE_BATCH_SIZE: usize = std::env::var("EXECUTE_BATCH_SIZE").map(|var| var.parse().expect("Execute batch size environmental variable is not a number")).unwrap_or(EXECUTE_BATCH_SIZE_DEFAULT);
}
#[cfg(not(feature = "sync-test"))]
lazy_static::lazy_static! {
    static ref EXECUTE_BATCH_SIZE: usize = EXECUTE_BATCH_SIZE_DEFAULT;
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
                    time_elapsed_s = start_time.elapsed().as_secs(),
                    %sync_head,
                    "Sync cycle finished successfully",
                );
            }

            // If the error is irrecoverable, we exit ethrex
            Err(error) => {
                match error.is_recoverable() {
                    false => {
                        // We exit the node, as we can't recover this error
                        error!(
                            time_elapsed_s = start_time.elapsed().as_secs(),
                            %sync_head,
                            %error, "Sync cycle failed, exiting as the error is irrecoverable",
                        );
                        std::process::exit(2);
                    }
                    true => {
                        // We do nothing, as the error is recoverable
                        error!(
                            time_elapsed_s = start_time.elapsed().as_secs(),
                            %sync_head,
                            %error, "Sync cycle failed, retrying",
                        );
                    }
                }
            }
        }
    }

    /// Performs the sync cycle described in `start_sync`, returns an error if the sync fails at any given step and aborts all active processes
    async fn sync_cycle(&mut self, sync_head: H256, store: Store) -> Result<(), SyncError> {
        // Take picture of the current sync mode, we will update the original value when we need to
        if self.snap_enabled.load(Ordering::Relaxed) {
            METRICS.enable().await;
            // We validate that we have the folders that are being used empty, as we currently assume
            // they are. If they are not empty we empty the folder
            delete_leaves_folder(&self.datadir);
            let sync_cycle_result = snap_sync::sync_cycle_snap(
                &mut self.peers,
                self.blockchain.clone(),
                &self.snap_enabled,
                sync_head,
                store,
                &self.datadir,
            )
            .await;
            METRICS.disable().await;
            sync_cycle_result
        } else {
            full::sync_cycle_full(
                &mut self.peers,
                self.blockchain.clone(),
                self.cancel_token.clone(),
                sync_head,
                store,
            )
            .await
        }
    }
}

/// An inclusive hash range representing a portion of a storage trie that needs to be downloaded.
#[derive(Debug, Clone)]
pub struct Interval {
    pub start: H256,
    pub end: H256,
}

/// A single storage slot: a hashed key and its value.
#[derive(Debug, Clone)]
pub struct Slot {
    pub hash: H256,
    pub value: U256,
}

/// A storage trie that fits in a single `request_storage_ranges` request.
#[derive(Debug, Clone)]
pub struct SmallTrie {
    pub accounts: Vec<H256>,
    pub slots: Vec<Slot>,
}

/// A storage trie too large to fit in a single `request_storage_ranges` request.
/// It is downloaded in multiple sub-range requests tracked by `intervals`.
#[derive(Debug, Clone)]
pub struct BigTrie {
    pub accounts: Vec<H256>,
    pub slots: Vec<Slot>,
    pub intervals: Vec<Interval>,
}

/// Tracks the download state of storage tries during snap sync.
///
/// All tries start as small (in `small_tries`). When a request fails to download
/// a trie in its entirety, it is promoted to big (moved to `big_tries`) and split
/// into sub-range intervals that are downloaded independently.
///
/// Both maps are keyed by the storage trie root hash. A healing function is
/// responsible for reconciling partially downloaded tries after the download phase.
#[derive(Debug, Default)]
pub struct StorageTrieTracker {
    pub small_tries: HashMap<H256, SmallTrie>,
    pub big_tries: HashMap<H256, BigTrie>,
    /// Accounts that need storage trie healing (big accounts, healed accounts, etc.)
    pub healed_accounts: HashSet<H256>,
    /// Reverse lookup: account hash -> current storage root
    pub account_to_root: HashMap<H256, H256>,
}

impl StorageTrieTracker {
    /// Inserts an account into the tracker, grouping by storage root.
    /// If the root already exists (in small or big), appends the account to it.
    pub fn insert_account(&mut self, account_hash: H256, storage_root: H256) {
        self.account_to_root.insert(account_hash, storage_root);
        if let Some(big) = self.big_tries.get_mut(&storage_root) {
            big.accounts.push(account_hash);
            return;
        }
        self.small_tries
            .entry(storage_root)
            .or_insert_with(|| SmallTrie {
                accounts: Vec::new(),
                slots: Vec::new(),
            })
            .accounts
            .push(account_hash);
    }

    /// Promotes a small trie to a big trie with downloaded slots and computed intervals.
    pub fn promote_to_big(
        &mut self,
        root: H256,
        first_slots: Vec<Slot>,
        intervals: Vec<Interval>,
    ) {
        let small = self.small_tries.remove(&root);
        let accounts = small.map(|s| s.accounts).unwrap_or_default();
        self.big_tries.insert(
            root,
            BigTrie {
                accounts,
                slots: first_slots,
                intervals,
            },
        );
    }

    /// Drains up to `batch_size` entries from `small_tries`, returning owned data.
    pub fn take_small_batch(&mut self, batch_size: usize) -> Vec<(H256, SmallTrie)> {
        let keys: Vec<H256> = self.small_tries.keys().take(batch_size).copied().collect();
        let mut batch = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(trie) = self.small_tries.remove(&key) {
                batch.push((key, trie));
            }
        }
        batch
    }

    /// Re-inserts failed small tries back into `small_tries`.
    pub fn return_small_tries(&mut self, tries: Vec<(H256, SmallTrie)>) {
        for (root, trie) in tries {
            let entry = self.small_tries.entry(root).or_insert_with(|| SmallTrie {
                accounts: Vec::new(),
                slots: Vec::new(),
            });
            entry.accounts.extend(trie.accounts);
            if entry.slots.is_empty() {
                entry.slots = trie.slots;
            }
        }
    }

    /// Called by state healing when an account's storage root changes.
    pub fn handle_healed_account(
        &mut self,
        account_hash: H256,
        old_root: H256,
        new_root: H256,
    ) {
        // Always mark for healing
        self.healed_accounts.insert(account_hash);

        if old_root == new_root {
            return;
        }

        // Determine where the old root lives
        let in_big = self.big_tries.contains_key(&old_root);

        if in_big {
            // Old root was in big_tries
            let big = self
                .big_tries
                .get(&old_root)
                .expect("big_tries should contain old_root");
            let is_only_account = big.accounts.len() == 1 && big.accounts[0] == account_hash;
            if is_only_account {
                // Only account: re-key
                let big = self
                    .big_tries
                    .remove(&old_root)
                    .expect("big_tries should contain old_root");
                if let Some(existing) = self.big_tries.get_mut(&new_root) {
                    existing.accounts.push(account_hash);
                } else {
                    self.big_tries.insert(new_root, big);
                }
            } else {
                // Multiple accounts: remove account from old entry, add to new root
                let new_root_exists = self.big_tries.contains_key(&new_root);
                if new_root_exists {
                    // new_root already tracked: just move the account, no clone needed
                    self.big_tries
                        .get_mut(&old_root)
                        .expect("big_tries should contain old_root")
                        .accounts
                        .retain(|a| *a != account_hash);
                    self.big_tries
                        .get_mut(&new_root)
                        .expect("big_tries should contain new_root")
                        .accounts
                        .push(account_hash);
                } else {
                    // new_root not tracked: clone slots/intervals for the new entry
                    let big = self
                        .big_tries
                        .get_mut(&old_root)
                        .expect("big_tries should contain old_root");
                    let new_big = BigTrie {
                        accounts: vec![account_hash],
                        slots: big.slots.clone(),
                        intervals: big.intervals.clone(),
                    };
                    big.accounts.retain(|a| *a != account_hash);
                    self.big_tries.insert(new_root, new_big);
                }
            }
        } else {
            // Old root was in small_tries or not registered
            if let Some(small) = self.small_tries.get_mut(&old_root) {
                small.accounts.retain(|a| *a != account_hash);
                if small.accounts.is_empty() {
                    self.small_tries.remove(&old_root);
                }
            }

            // Add to new root's trie
            if let Some(big) = self.big_tries.get_mut(&new_root) {
                big.accounts.push(account_hash);
            } else {
                self.small_tries
                    .entry(new_root)
                    .or_insert_with(|| SmallTrie {
                        accounts: Vec::new(),
                        slots: Vec::new(),
                    })
                    .accounts
                    .push(account_hash);
            }
        }

        self.account_to_root.insert(account_hash, new_root);
    }

    /// Moves all accounts from both maps into `healed_accounts` and clears the maps.
    pub fn drain_all_to_healed(&mut self) {
        for small in self.small_tries.values() {
            self.healed_accounts.extend(small.accounts.iter());
        }
        for big in self.big_tries.values() {
            self.healed_accounts.extend(big.accounts.iter());
        }
        self.small_tries.clear();
        self.big_tries.clear();
    }

    /// Returns the total number of remaining tries to download.
    pub fn remaining_count(&self) -> usize {
        self.small_tries.len() + self.big_tries.len()
    }
}

impl BigTrie {
    /// Computes download intervals for a big trie based on its download progress.
    pub fn compute_intervals(
        last_downloaded_hash: H256,
        slot_count: usize,
        slots_per_chunk: usize,
    ) -> Vec<Interval> {
        let start_hash_u256 = U256::from_big_endian(&last_downloaded_hash.0);
        let missing_storage_range = U256::MAX - start_hash_u256;
        let slot_count = slot_count.max(1);
        let storage_density = start_hash_u256 / slot_count;
        let chunk_size = storage_density
            .checked_mul(U256::from(slots_per_chunk))
            .unwrap_or(U256::MAX);
        // chunk_size is zero only when last_downloaded_hash < slot_count (integer division
        // floors to zero). In practice this requires either empty slots (H256::zero() fallback)
        // or keccak256 hashes smaller than the slot count, both of which indicate an unexpected
        // state earlier in the pipeline. We fall back to a single interval but warn so the
        // root cause can be investigated.
        let chunk_size = if chunk_size.is_zero() {
            warn!("compute_intervals: chunk_size is zero (last_downloaded_hash={last_downloaded_hash:?}, slot_count={slot_count}), falling back to single interval");
            U256::MAX
        } else {
            chunk_size
        };
        let chunk_count = (missing_storage_range / chunk_size).as_usize().max(1);

        let mut intervals = Vec::with_capacity(chunk_count);
        for i in 0..chunk_count {
            let interval_start_u256 = start_hash_u256 + chunk_size * i;
            let interval_start = H256::from_uint(&interval_start_u256);
            let interval_end = if i == chunk_count - 1 {
                H256([0xFF; 32])
            } else {
                let end_u256 = interval_start_u256
                    .checked_add(chunk_size)
                    .unwrap_or(U256::MAX);
                H256::from_uint(&end_u256)
            };
            intervals.push(Interval {
                start: interval_start,
                end: interval_end,
            });
        }
        intervals
    }
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
    #[error("Failed to get block headers")]
    NoBlockHeaders,
    #[error("Peer handler error: {0}")]
    PeerHandler(#[from] PeerHandlerError),
    #[error("Parent not found in healing queue. Parent: {0}, path: {1}")]
    HealingQueueInconsistency(String, String),
    #[error("Filesystem error: {0}")]
    FileSystem(String),
    #[error("Sorted Trie Generation Error: {0}")]
    TrieGenerationError(#[from] TrieGenerationError),
    #[error("Failed to get account temp db directory: {0}")]
    AccountTempDBDirNotFound(String),
    #[error("Failed to get storage temp db directory: {0}")]
    StorageTempDBDirNotFound(String),
    #[error("RocksDB Error: {0}")]
    RocksDBError(String),
    #[error("Bytecode file error")]
    BytecodeFileError,
    #[error("Error in Peer Table: {0}")]
    PeerTableError(#[from] PeerTableError),
    #[error("Missing fullsync batch")]
    MissingFullsyncBatch,
    #[error("Snap error: {0}")]
    Snap(#[from] crate::snap::SnapError),
}

impl SyncError {
    pub fn is_recoverable(&self) -> bool {
        match self {
            SyncError::SnapshotReadError(_, _)
            | SyncError::SnapshotDecodeError(_)
            | SyncError::CodeHashesSnapshotDecodeError(_)
            | SyncError::AccountState(_, _)
            | SyncError::BytecodesNotFound
            | SyncError::AccountStateSnapshotsDirNotFound
            | SyncError::AccountStoragesSnapshotsDirNotFound
            | SyncError::CodeHashesSnapshotsDirNotFound
            | SyncError::DifferentStateRoots(_, _, _)
            | SyncError::NoBlockHeaders
            | SyncError::PeerHandler(_)
            | SyncError::HealingQueueInconsistency(_, _)
            | SyncError::TrieGenerationError(_)
            | SyncError::AccountTempDBDirNotFound(_)
            | SyncError::StorageTempDBDirNotFound(_)
            | SyncError::RocksDBError(_)
            | SyncError::BytecodeFileError
            | SyncError::NoLatestCanonical
            | SyncError::PeerTableError(_)
            | SyncError::MissingFullsyncBatch
            | SyncError::Snap(_)
            | SyncError::FileSystem(_) => false,
            SyncError::Chain(_)
            | SyncError::Store(_)
            | SyncError::Send(_)
            | SyncError::Trie(_)
            | SyncError::Rlp(_)
            | SyncError::JoinHandle(_)
            | SyncError::CorruptDB
            | SyncError::BodiesNotFound
            | SyncError::InvalidRangeReceived
            | SyncError::BlockNumber(_)
            | SyncError::NoBlocks => true,
        }
    }
}

impl<T> From<SendError<T>> for SyncError {
    fn from(value: SendError<T>) -> Self {
        Self::Send(value.to_string())
    }
}
