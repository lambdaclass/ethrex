use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
use ethrex_storage::Store;
#[cfg(feature = "l2")]
use ethrex_storage_rollup::StoreRollup;
use tokio::{
    sync::Mutex,
    time::{Duration, sleep},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    peer_handler::PeerHandler,
    sync::{SyncMode, Syncer},
};

/// Abstraction to interact with the active sync process without disturbing it
#[derive(Debug, Clone)]
pub struct SyncManager {
    /// This is also held by the Syncer and allows tracking it's latest syncmode
    /// It is a READ_ONLY value, as modifications will disrupt the current active sync progress
    snap_enabled: Arc<AtomicBool>,
    syncer: Arc<Mutex<Syncer>>,
    last_fcu_head: Arc<Mutex<H256>>,
    store: Store,
    #[cfg(feature = "l2")]
    rollup_store: StoreRollup,
    /// The batch number to be synced to
    new_batch_head: Arc<Mutex<u64>>,
    /// The batch number it is currently syncing to
    #[cfg(feature = "l2")]
    last_batch_number: Arc<Mutex<u64>>,
}

impl SyncManager {
    pub async fn new(
        peer_handler: PeerHandler,
        sync_mode: SyncMode,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
        store: Store,
        #[cfg(feature = "l2")] rollup_store: StoreRollup,
    ) -> Self {
        let snap_enabled = Arc::new(AtomicBool::new(matches!(sync_mode, SyncMode::Snap)));
        let syncer = Arc::new(Mutex::new(Syncer::new(
            peer_handler,
            snap_enabled.clone(),
            cancel_token,
            blockchain,
        )));
        let sync_manager = Self {
            snap_enabled,
            syncer,
            last_fcu_head: Arc::new(Mutex::new(H256::zero())),
            store: store.clone(),
            #[cfg(feature = "l2")]
            rollup_store,
            #[cfg(feature = "l2")]
            last_batch_number: Arc::new(Mutex::new(1)),
            new_batch_head: Arc::new(Mutex::new(1)),
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
            #[cfg(feature = "l2")]
            rollup_store: StoreRollup::default(),
            #[cfg(feature = "l2")]
            last_batch_number: Arc::new(Mutex::new(1)),
            // #[cfg(feature = "l2")]
            new_batch_head: Arc::new(Mutex::new(1)),
        }
    }

    /// Sets the latest fcu head and starts the next sync cycle if the syncer is currently inactive
    pub fn sync_to_head(&self, fcu_head: H256, batch_number: u64) {
        self.set_head(fcu_head);
        self.set_batch_number(batch_number);
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

    /// Updates the last batch number. This may be used on the next sync cycle if needed
    fn set_batch_number(&self, batch_number: u64) {
        if let Ok(mut new_batch_head) = self.new_batch_head.try_lock() {
            *new_batch_head = batch_number;
        } else {
            warn!("Failed to update latest batch number for syncing")
        }
    }

    /// Returns true is the syncer is active
    pub fn is_active(&self) -> bool {
        self.syncer.try_lock().is_err()
    }

    /// Attempts to sync to the last received fcu head
    /// Will do nothing if the syncer is already involved in a sync process
    /// If the sync process would require multiple sync cycles (such as snap sync), starts all required sync cycles until the sync is complete
    fn start_sync(&self) {
        let syncer = self.syncer.clone();
        let store = self.store.clone();
        let sync_head = self.last_fcu_head.clone();
        #[cfg(feature = "l2")]
        let rollup_store = self.rollup_store.clone();
        #[cfg(feature = "l2")]
        let last_batch_number = self.last_batch_number.clone();
        #[cfg(feature = "l2")]
        let new_batch_head = self.new_batch_head.clone();

        tokio::spawn(async move {
            let Ok(Some(current_head)) = store.get_latest_canonical_block_hash().await else {
                error!("Failed to fetch latest canonical block, unable to sync");
                return;
            };

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
                #[cfg(feature = "l2")]
                let last_batch_number_value = {
                    let Ok(last_batch_number) = last_batch_number.try_lock() else {
                        error!("Failed to read latest batch number, unable to sync");
                        return;
                    };
                    let last_batch_number_value = *last_batch_number;
                    let last_batch_number_on_store =
                        rollup_store.get_latest_batch_number().await.unwrap_or(0);
                    last_batch_number_value.max(last_batch_number_on_store)
                };
                #[cfg(feature = "l2")]
                let new_batch_head = {
                    let Ok(new_batch_head) = new_batch_head.try_lock() else {
                        error!("Failed to read new batch head, unable to sync");
                        return;
                    };
                    *new_batch_head
                };
                // Start the sync cycle
                syncer
                    .start_sync(
                        current_head,
                        sync_head,
                        store.clone(),
                        #[cfg(feature = "l2")]
                        rollup_store.clone(),
                        #[cfg(feature = "l2")]
                        last_batch_number_value,
                        #[cfg(feature = "l2")]
                        new_batch_head,
                    )
                    .await;
                #[cfg(feature = "l2")]
                {
                    last_batch_number
                        .try_lock()
                        .map(|mut last_batch_number| {
                            *last_batch_number = new_batch_head;
                        })
                        .unwrap_or_else(|_| {
                            error!("Failed to update last batch number after sync");
                        });
                }
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
}
