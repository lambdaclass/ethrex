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
use tokio::sync::{Mutex, watch};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    peer_handler::PeerHandler,
    sync::{SyncMode, Syncer},
};

/// Abstraction to interact with the active sync process without disturbing it
#[derive(Debug)]
pub struct SyncManager {
    /// This is also held by the Syncer and allows tracking it's latest syncmode
    /// It is a READ_ONLY value, as modifications will disrupt the current active sync progress
    snap_enabled: Arc<AtomicBool>,
    syncer: Arc<Mutex<Syncer>>,
    last_fcu_head: watch::Sender<H256>,
    store: Store,
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
        let syncer = Arc::new(Mutex::new(Syncer::new(
            peer_handler,
            snap_enabled.clone(),
            cancel_token,
            blockchain,
            datadir,
        )));
        let (tx, _) = watch::channel(H256::zero());
        let sync_manager = Self {
            snap_enabled,
            syncer,
            last_fcu_head: tx,
            store: store.clone(),
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

    /// Updates the last fcu head. This may be used on the next sync cycle if needed.
    /// Uses a watch channel so updates are never dropped due to lock contention.
    fn set_head(&self, fcu_head: H256) {
        // send() only fails if there are no receivers, which is fine — the next
        // start_sync() will read the current value via borrow().
        let _ = self.last_fcu_head.send(fcu_head);
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
        let mut rx = self.last_fcu_head.subscribe();

        tokio::spawn(async move {
            // If we can't get hold of the syncer, then it means that there is an active sync in process
            let Ok(mut syncer) = syncer.try_lock() else {
                return;
            };
            loop {
                let sync_head = *rx.borrow();
                // Edge case: If we are resuming a sync process after a node restart, wait until the next fcu to start
                if sync_head.is_zero() {
                    info!("Resuming sync after node restart, waiting for next FCU");
                    // Wait for a new FCU head — wakes immediately when set_head() is called
                    if rx.changed().await.is_err() {
                        // Sender dropped (node shutting down)
                        return;
                    }
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

    pub fn get_last_fcu_head(&self) -> H256 {
        *self.last_fcu_head.borrow()
    }
}
