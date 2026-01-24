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
    sync::{SyncMode, Syncer, spawn_header_backfill},
};

/// Abstraction to interact with the active sync process without disturbing it
#[derive(Debug)]
pub struct SyncManager {
    /// This is also held by the Syncer and allows tracking it's latest syncmode
    /// It is a READ_ONLY value, as modifications will disrupt the current active sync progress
    snap_enabled: Arc<AtomicBool>,
    syncer: Arc<Mutex<Syncer>>,
    last_fcu_head: Arc<Mutex<H256>>,
    store: Store,
    /// Flag indicating whether header backfill is currently running in the background
    backfill_in_progress: Arc<AtomicBool>,
    /// Token to cancel the backfill task when shutting down
    backfill_cancel_token: CancellationToken,
    /// PeerHandler for backfill operations (cloned from syncer's peer handler)
    peer_handler: PeerHandler,
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
        let backfill_in_progress = Arc::new(AtomicBool::new(false));
        let backfill_cancel_token = CancellationToken::new();
        let peer_handler_clone = peer_handler.clone();

        let syncer = Arc::new(Mutex::new(Syncer::new(
            peer_handler,
            snap_enabled.clone(),
            cancel_token,
            blockchain,
            datadir,
        )));
        let sync_manager = Self {
            snap_enabled,
            syncer,
            last_fcu_head: Arc::new(Mutex::new(H256::zero())),
            store: store.clone(),
            backfill_in_progress,
            backfill_cancel_token,
            peer_handler: peer_handler_clone,
        };

        // If the node was in the middle of a sync and then re-started we must resume syncing
        // Otherwise we will incorreclty assume the node is already synced and work on invalid state
        if store
            .get_header_download_checkpoint()
            .await
            .is_ok_and(|res| res.is_some())
        {
            sync_manager.start_sync();
        }

        // Check if we need to resume header backfill
        sync_manager.resume_backfill_if_needed().await;

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
        if self.snap_enabled.load(Ordering::Relaxed) {
            SyncMode::Snap
        } else {
            SyncMode::Full
        }
    }

    /// Disables snapsync mode
    pub fn disable_snap(&self) {
        self.snap_enabled.store(false, Ordering::Relaxed);
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

    /// Returns true if header backfill is currently running in the background
    pub fn is_backfill_active(&self) -> bool {
        self.backfill_in_progress.load(Ordering::Relaxed)
    }

    /// Check if backfill needs to be resumed and spawn the task if necessary
    async fn resume_backfill_if_needed(&self) {
        // Check if backfill is already running
        if self.is_backfill_active() {
            return;
        }

        // Check if backfill is complete
        match self.store.is_header_backfill_complete().await {
            Ok(true) => {
                // Backfill already complete, nothing to do
                return;
            }
            Ok(false) => {
                // Backfill is not complete, check progress
            }
            Err(e) => {
                warn!("Failed to check backfill status: {}", e);
                return;
            }
        }

        // Check if there's backfill progress to resume from
        let backfill_progress = match self.store.get_header_backfill_progress().await {
            Ok(Some(progress)) => progress,
            Ok(None) => {
                // No backfill in progress, nothing to resume
                return;
            }
            Err(e) => {
                warn!("Failed to get backfill progress: {}", e);
                return;
            }
        };

        // Only resume if there are still blocks to backfill
        if backfill_progress > 0 {
            info!(
                "Resuming header backfill from block {} (blocks remaining to genesis)",
                backfill_progress
            );
            let chain_id = self.store.get_chain_config().chain_id;
            spawn_header_backfill(
                self.peer_handler.clone(),
                self.store.clone(),
                backfill_progress,
                self.backfill_cancel_token.clone(),
                self.backfill_in_progress.clone(),
                chain_id,
            );
        }
    }

    /// Cancels the backfill task if it's running
    pub fn cancel_backfill(&self) {
        if self.is_backfill_active() {
            info!("Cancelling header backfill task");
            self.backfill_cancel_token.cancel();
        }
    }
}
