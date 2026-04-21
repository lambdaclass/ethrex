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
use crate::snap::constants::EXECUTE_BATCH_SIZE_DEFAULT;
use crate::utils::delete_leaves_folder;
use ethrex_blockchain::{Blockchain, error::ChainError};
use ethrex_common::H256;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::TrieError;
use ethrex_trie::trie_sorted::TrieGenerationError;
use spawned_concurrency::error::ActorError;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc::error::SendError;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

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

/// Diagnostic snapshot of the sync state, used by admin RPC endpoints.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SyncDiagnostics {
    pub sync_mode: String,
    pub current_phase: String,
    pub pivot_block_number: Option<u64>,
    pub pivot_timestamp: Option<u64>,
    pub pivot_age_seconds: Option<u64>,
    pub staleness_threshold_seconds: u64,
    pub phase_progress: std::collections::HashMap<String, u64>,
    pub recent_pivot_changes: std::collections::VecDeque<PivotChangeEvent>,
    pub recent_errors: std::collections::VecDeque<SyncErrorEvent>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PivotChangeEvent {
    pub timestamp: u64,
    pub old_pivot_number: u64,
    pub new_pivot_number: u64,
    pub outcome: String,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncErrorEvent {
    pub timestamp: u64,
    pub error_type: String,
    pub error_message: String,
    pub recoverable: bool,
}

impl SyncDiagnostics {
    const MAX_PIVOT_CHANGES: usize = 10;
    const MAX_ERRORS: usize = 20;

    pub fn push_pivot_change(&mut self, event: PivotChangeEvent) {
        if self.recent_pivot_changes.len() >= Self::MAX_PIVOT_CHANGES {
            self.recent_pivot_changes.pop_front();
        }
        self.recent_pivot_changes.push_back(event);
    }

    pub fn push_error(&mut self, event: SyncErrorEvent) {
        if self.recent_errors.len() >= Self::MAX_ERRORS {
            self.recent_errors.pop_front();
        }
        self.recent_errors.push_back(event);
    }
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
    diagnostics: Arc<tokio::sync::RwLock<SyncDiagnostics>>,
}

impl Syncer {
    pub fn new(
        peers: PeerHandler,
        snap_enabled: Arc<AtomicBool>,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
        datadir: PathBuf,
        diagnostics: Arc<tokio::sync::RwLock<SyncDiagnostics>>,
    ) -> Self {
        Self {
            snap_enabled,
            peers,
            cancel_token,
            blockchain,
            datadir,
            diagnostics,
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
                self.diagnostics.write().await.current_phase = "idle".to_string();
                info!(
                    time_elapsed_s = start_time.elapsed().as_secs(),
                    %sync_head,
                    "Sync cycle finished successfully",
                );
            }

            // If the error is irrecoverable, we exit ethrex
            Err(error) => {
                let recoverable = error.is_recoverable();
                self.diagnostics.write().await.current_phase = "idle".to_string();
                debug!(
                    error_type = %error,
                    recoverable = recoverable,
                    action = if recoverable { "retry" } else { "exit" },
                    "Sync cycle error classification"
                );
                self.diagnostics.write().await.push_error(SyncErrorEvent {
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                    error_type: format!("{:?}", std::mem::discriminant(&error)),
                    error_message: error.to_string(),
                    recoverable,
                });
                match recoverable {
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
                &self.diagnostics,
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

#[derive(Debug, Default)]
#[allow(clippy::type_complexity)]
/// We store for optimization the accounts that need to heal storage
pub struct AccountStorageRoots {
    /// The accounts that have not been healed are guaranteed to have the original storage root
    /// we can read this storage root
    pub accounts_with_storage_root: BTreeMap<H256, (Option<H256>, Vec<(H256, H256)>)>,
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
    PeerTableError(#[from] ActorError),
    #[error("Missing fullsync batch")]
    MissingFullsyncBatch,
    #[error("Snap error: {0}")]
    Snap(#[from] crate::snap::SnapError),
}

impl SyncError {
    pub fn is_recoverable(&self) -> bool {
        // PeerHandler delegates to its own classification so that transient
        // peer/network errors retry while structural errors (dead actor,
        // local storage full) still exit.
        if let SyncError::PeerHandler(e) = self {
            return e.is_recoverable();
        }
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
            | SyncError::NoBlocks
            | SyncError::NoBlockHeaders => true,
            // PeerHandler handled above by delegation
            SyncError::PeerHandler(_) => unreachable!(),
        }
    }
}

impl<T> From<SendError<T>> for SyncError {
    fn from(value: SendError<T>) -> Self {
        Self::Send(value.to_string())
    }
}
