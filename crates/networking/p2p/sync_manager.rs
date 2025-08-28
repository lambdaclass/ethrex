use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
use ethrex_storage::{Store, error::StoreError};
use tokio::{
    sync::Mutex,
    task::{JoinHandle, spawn},
    time::{Duration, interval, sleep},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

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
    last_fcu_head: Arc<Mutex<H256>>,
    store: Store,
    cancel_token: CancellationToken,
}

impl SyncManager {
    pub async fn new(
        peer_handler: PeerHandler,
        sync_mode: SyncMode,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
        store: Store,
    ) -> Self {
        let snap_enabled = Arc::new(AtomicBool::new(matches!(sync_mode, SyncMode::Snap)));
        let syncer = Arc::new(Mutex::new(Syncer::new(
            peer_handler,
            snap_enabled.clone(),
            cancel_token.clone(),
            blockchain,
        )));
        let sync_manager = Self {
            snap_enabled,
            syncer,
            last_fcu_head: Arc::new(Mutex::new(H256::zero())),
            store: store.clone(),
            cancel_token,
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

    /// Creates a dummy SyncManager for tests where syncing is not needed
    /// This should only be used in tests as it won't be able to connect to the p2p network
    pub fn dummy() -> Self {
        Self {
            snap_enabled: Arc::new(AtomicBool::new(false)),
            syncer: Arc::new(Mutex::new(Syncer::dummy())),
            last_fcu_head: Arc::new(Mutex::new(H256::zero())),
            store: Store::new("temp.db", ethrex_storage::EngineType::InMemory)
                .expect("Failed to start Storage Engine"),
            cancel_token: CancellationToken::new(),
        }
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
        let cancel_token = self.cancel_token.clone();

        // Should we prune when are syncing? NO - we'll prune AFTER syncing is complete

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
                    // Sync is complete, start the pruning task
                    info!("[SYNC] Sync process completed, starting background pruning task");
                    Self::start_pruner_task(store.clone(), cancel_token.clone());
                    break;
                }
            }
        });
    }

    pub fn get_last_fcu_head(&self) -> Result<H256, tokio::sync::TryLockError> {
        Ok(*self.last_fcu_head.try_lock()?)
    }

    /// Start the pruning task in the background after sync completion
    fn start_pruner_task(
        store: Store,
        cancellation_token: CancellationToken,
    ) -> JoinHandle<Result<(), StoreError>> {
        const KEEP_BLOCKS: u64 = 128;
        const PRUNING_INTERVAL: Duration = Duration::from_secs(60);

        spawn(async move {
            let mut interval = interval(PRUNING_INTERVAL);
            info!("[PRUNING] Starting pruning task after sync completion");

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let result = tokio::task::spawn_blocking({
                            let store = store.clone();
                            move || store.prune_state_and_storage_log(KEEP_BLOCKS)
                        }).await;

                        match result {
                            Ok(Ok(())) => debug!("[PRUNING] Pruning completed"),
                            Ok(Err(e)) => error!("[PRUNING] Pruning error: {:?}", e),
                            Err(e) => error!("[PRUNING] Task join error: {:?}", e),
                        }
                    }
                    _ = cancellation_token.cancelled() => {
                        info!("[PRUNING] Pruner task shutting down");
                        break;
                    }
                }
            }
            Ok(())
        })
    }
}
