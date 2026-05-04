use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
use ethrex_storage::Store;
use tokio::{
    sync::Mutex,
    time::{Duration, sleep},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    peer_handler::PeerHandler,
    sync::{SyncDiagnostics, SyncMode, Syncer},
};

/// Abstraction to interact with the active sync process without disturbing it
#[derive(Debug)]
pub struct SyncManager {
    /// This is also held by the Syncer and allows tracking it's latest syncmode
    /// It is a READ_ONLY value, as modifications will disrupt the current active sync progress
    snap_enabled: Arc<AtomicBool>,
    syncer: Arc<Mutex<Syncer>>,
    last_fcu_head: Arc<Mutex<H256>>,
    /// The finalized block hash and number from the most recent FCU message.
    /// Populated whenever `engine_forkchoiceUpdated` is received.
    /// Default is `(H256::zero(), 0)` until the first FCU arrives.
    last_fcu_finalized: Arc<Mutex<(H256, u64)>>,
    /// One-shot latch: true once the follower has seen its head at or beyond
    /// the CL-reported finalized block. Never reset to false.
    /// Set with `Ordering::Release`; read with `Ordering::Acquire`.
    caught_up: Arc<AtomicBool>,
    store: Store,
    diagnostics: Arc<tokio::sync::RwLock<SyncDiagnostics>>,
}

impl SyncManager {
    pub async fn new(
        peer_handler: PeerHandler,
        sync_mode: &SyncMode,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
        store: Store,
        datadir: PathBuf,
    ) -> Self {
        let snap_enabled = Arc::new(AtomicBool::new(matches!(sync_mode, SyncMode::Snap)));

        // Fetch checkpoint once to avoid duplicate DB reads
        let has_checkpoint = store
            .get_header_download_checkpoint()
            .await
            .unwrap_or_else(|e| {
                warn!("Failed to read header download checkpoint: {e}");
                None
            })
            .is_some();

        // Auto-switch from snap to full sync if node already has synced state.
        // For post-merge networks (terminal_total_difficulty_passed), any stored
        // block > 0 means the node has previously synced. For pre-merge networks,
        // use merge_netsplit_block as threshold to avoid false positives in hive tests.
        if snap_enabled.load(Ordering::Acquire) {
            let latest_block = store.get_latest_block_number().await.unwrap_or(0);
            let chain_config = store.get_chain_config();
            let is_synced = if chain_config.terminal_total_difficulty_passed {
                latest_block > 0
            } else if let Some(merge_block) = chain_config.merge_netsplit_block {
                latest_block > merge_block
            } else {
                false
            };
            if is_synced {
                info!("Node has synced state (block {latest_block}), switching to full sync");
                snap_enabled.store(false, Ordering::Release);
                if has_checkpoint && let Err(e) = store.clear_snap_state().await {
                    warn!("Failed to clear stale snap state: {e}");
                }
            }
        }

        let diagnostics = Arc::new(tokio::sync::RwLock::new(SyncDiagnostics::default()));
        let syncer = Arc::new(Mutex::new(Syncer::new(
            peer_handler,
            snap_enabled.clone(),
            cancel_token,
            blockchain,
            datadir,
            diagnostics.clone(),
        )));
        let sync_manager = Self {
            snap_enabled,
            syncer,
            last_fcu_head: Arc::new(Mutex::new(H256::zero())),
            last_fcu_finalized: Arc::new(Mutex::new((H256::zero(), 0))),
            caught_up: Arc::new(AtomicBool::new(false)),
            store: store.clone(),
            diagnostics,
        };
        // If the node was in the middle of a sync and then re-started we must resume syncing
        // Otherwise we will incorreclty assume the node is already synced and work on invalid state
        // Skip if the auto-switch already transitioned to full sync (snap_enabled is now false)
        if has_checkpoint && sync_manager.snap_enabled.load(Ordering::Acquire) {
            sync_manager.start_sync();
        }
        sync_manager
    }

    /// Sets the latest fcu head and starts the next sync cycle if the syncer is currently inactive
    pub fn sync_to_head(&self, fcu_head: H256) {
        self.set_head(fcu_head);
        if !self.is_active() {
            self.start_sync();
        }
    }

    /// Returns the syncer's current syncmode (either snap or full)
    pub fn sync_mode(&self) -> SyncMode {
        // Acquire pairs with the Release stores in disable_snap / SyncManager::new
        // and snap_sync.rs, ensuring callers see a consistent view.
        if self.snap_enabled.load(Ordering::Acquire) {
            SyncMode::Snap
        } else {
            SyncMode::Full
        }
    }

    /// Disables snapsync mode
    pub fn disable_snap(&self) {
        self.snap_enabled.store(false, Ordering::Release);
    }

    /// Returns a snapshot of the current sync diagnostics with live values.
    pub async fn get_sync_diagnostics(&self) -> SyncDiagnostics {
        use crate::metrics::METRICS;
        use std::sync::atomic::Ordering::Relaxed;

        let mut diag = self.diagnostics.read().await.clone();

        // Compute live pivot age
        if let Some(ts) = diag.pivot_timestamp {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            diag.pivot_age_seconds = Some(now.saturating_sub(ts));
        }

        // Populate live progress from METRICS atomics
        let headers = METRICS.downloaded_headers.get();
        let accounts_downloaded = METRICS.downloaded_account_tries.load(Relaxed);
        let accounts_inserted = METRICS.account_tries_inserted.load(Relaxed);
        let storage_downloaded = METRICS.storage_leaves_downloaded.get();
        let storage_inserted = METRICS.storage_leaves_inserted.get();

        if headers > 0 {
            diag.phase_progress
                .insert("headers_downloaded".into(), headers);
        }
        if accounts_downloaded > 0 {
            diag.phase_progress
                .insert("accounts_downloaded".into(), accounts_downloaded);
        }
        if accounts_inserted > 0 {
            diag.phase_progress
                .insert("accounts_inserted".into(), accounts_inserted);
        }
        if storage_downloaded > 0 {
            diag.phase_progress
                .insert("storage_slots_downloaded".into(), storage_downloaded);
        }
        if storage_inserted > 0 {
            diag.phase_progress
                .insert("storage_slots_inserted".into(), storage_inserted);
        }

        diag
    }

    /// Returns a reference to the diagnostics RwLock for updating from the sync code.
    pub fn diagnostics(&self) -> &Arc<tokio::sync::RwLock<SyncDiagnostics>> {
        &self.diagnostics
    }

    /// Updates the last fcu head. This may be used on the next sync cycle if needed
    fn set_head(&self, fcu_head: H256) {
        if let Ok(mut latest_fcu_head) = self.last_fcu_head.try_lock() {
            *latest_fcu_head = fcu_head;
        } else {
            warn!("Failed to update latest fcu head for syncing")
        }
    }

    /// Returns true is the syncer is active
    fn is_active(&self) -> bool {
        self.syncer.try_lock().is_err()
    }

    /// Attempts to sync to the last received fcu head
    /// Will do nothing if the syncer is already involved in a sync process
    /// If the sync process would require multiple sync cycles (such as snap sync), starts all required sync cycles until the sync is complete
    fn start_sync(&self) {
        let syncer = self.syncer.clone();
        let store = self.store.clone();
        let sync_head = self.last_fcu_head.clone();

        tokio::spawn(async move {
            // If we can't get hold of the syncer, then it means that there is an active sync in process
            let Ok(mut syncer) = syncer.try_lock() else {
                return;
            };
            loop {
                let sync_head = {
                    // Read latest fcu head without holding the lock for longer than needed
                    let Ok(sync_head) = sync_head.try_lock() else {
                        error!("Failed to read latest fcu head, unable to sync");
                        return;
                    };
                    *sync_head
                };
                // Edge case: If we are resuming a sync process after a node restart, wait until the next fcu to start
                if sync_head.is_zero() {
                    info!("Resuming sync after node restart, waiting for next FCU");
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
                // Start the sync cycle
                syncer.start_sync(sync_head, store.clone()).await;
                // Continue to the next sync cycle if we have an ongoing snap sync (aka if we still have snap sync checkpoints stored)
                if store
                    .get_header_download_checkpoint()
                    .await
                    .ok()
                    .flatten()
                    .is_none()
                {
                    break;
                }
            }
        });
    }

    pub fn get_last_fcu_head(&self) -> Result<H256, tokio::sync::TryLockError> {
        Ok(*self.last_fcu_head.try_lock()?)
    }

    /// Returns a clone of the `snap_enabled` atomic so callers can observe
    /// (read-only) whether snap sync is still active.
    pub fn snap_enabled(&self) -> Arc<AtomicBool> {
        self.snap_enabled.clone()
    }

    /// Returns a clone of the `caught_up` atomic. Set to true (one-shot latch)
    /// once the follower's committed head reaches or exceeds the CL-reported
    /// finalized block number. Never reset to false.
    pub fn caught_up(&self) -> Arc<AtomicBool> {
        self.caught_up.clone()
    }

    /// Returns true if the follower has been caught up to finalized head at
    /// least once since this process started.
    pub fn is_caught_up(&self) -> bool {
        self.caught_up.load(Ordering::Acquire)
    }

    /// Returns a clone of the `last_fcu_finalized` mutex so callers can read
    /// the most recently received finalized block hash and number.
    pub fn last_fcu_finalized(&self) -> Arc<Mutex<(H256, u64)>> {
        self.last_fcu_finalized.clone()
    }

    /// Records the finalized block from an FCU message.
    ///
    /// Called from the FCU handler each time `engine_forkchoiceUpdated` fires.
    /// Stores the finalized block hash and number so that the `TransitionActivator`
    /// can compare against the committed head when checking `caught_up`.
    pub fn update_fcu_finalized(&self, finalized_hash: H256, finalized_number: u64) {
        if let Ok(mut guard) = self.last_fcu_finalized.try_lock() {
            *guard = (finalized_hash, finalized_number);
        } else {
            warn!("Failed to update last_fcu_finalized: lock contended");
        }
    }

    /// Checks whether the given committed block number meets or exceeds the
    /// CL-reported finalized head. If so, latches `caught_up` to true.
    ///
    /// This is a one-shot latch: once true it never returns to false.
    /// Call this after each successful block commit.
    pub fn check_and_latch_caught_up(&self, committed_number: u64) {
        // Already latched — nothing to do.
        if self.caught_up.load(Ordering::Acquire) {
            return;
        }
        let finalized_number = match self.last_fcu_finalized.try_lock() {
            Ok(guard) => guard.1,
            Err(_) => return, // Can't read; skip this tick.
        };
        // Only latch if we have received at least one real FCU (number > 0).
        if finalized_number > 0 && committed_number >= finalized_number {
            self.caught_up.store(true, Ordering::Release);
            // Plain "caught up" log — does NOT imply binary transition will fire.
            // Whether anything observes this latch depends on whether a
            // TransitionActivator was installed at startup (only when
            // --binary-transition is set AND the DB is in Mpt mode).
            info!(
                committed_number,
                finalized_number, "Follower caught up to finalized head."
            );
        }
    }
}
