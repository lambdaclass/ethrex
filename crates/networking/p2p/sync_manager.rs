use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
use ethrex_storage::{error::StoreError, Store};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    kademlia::KademliaTable,
    sync::{SyncMode, Syncer},
};

pub enum SyncStatus {
    Active(SyncMode),
    Inactive,
}

/// Abstraction to interact with the active sync process without disturbing it
#[derive(Debug)]
pub struct SyncManager {
    /// This is also held by the Syncer and allows tracking it's latest syncmode
    /// It is a READ_ONLY value, as modifications will disrupt the current active sync progress
    snap_enabled: Arc<AtomicBool>,
    syncer: Arc<Mutex<Syncer>>,
    last_fcu_head: Arc<Mutex<H256>>,
    store: Store,
}

impl SyncManager {
    pub fn new(
        peer_table: Arc<Mutex<KademliaTable>>,
        sync_mode: SyncMode,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
        store: Store,
    ) -> Self {
        let snap_enabled = Arc::new(AtomicBool::new(matches!(sync_mode, SyncMode::Snap)));
        let syncer = Arc::new(Mutex::new(Syncer::new(
            peer_table,
            snap_enabled.clone(),
            cancel_token,
            blockchain,
        )));
        Self {
            snap_enabled,
            syncer,
            last_fcu_head: Arc::new(Mutex::new(H256::zero())),
            store,
        }
    }

    /// Creates a dummy SyncManager for tests where syncing is not needed
    /// This should only be used in tests as it won't be able to connect to the p2p network
    pub fn dummy() -> Self {
        Self {
            snap_enabled: Arc::new(AtomicBool::new(false)),
            syncer: Arc::new(Mutex::new(Syncer::dummy())),
            last_fcu_head: Arc::new(Mutex::new(H256::zero())),
            store: Store::new("temp.db", ethrex_storage::EngineType::InMemory)
                .expect("Failed to create test DB"),
        }
    }

    /// Updates the last fcu head. This may be used on the next sync cycle if needed
    pub fn set_head(&self, fcu_head: H256) {
        if let Ok(mut latest_fcu_head) = self.last_fcu_head.try_lock() {
            *latest_fcu_head = fcu_head;
        } else {
            warn!("Failed to update latest fcu head for syncing")
        }
    }

    /// Returns the current sync status, either active or inactive and what the current syncmode is in the case of active
    pub fn status(&self) -> Result<SyncStatus, StoreError> {
        Ok(if self.syncer.try_lock().is_err() {
            SyncStatus::Active(self.sync_mode())
        } else {
            SyncStatus::Inactive
        })
    }

    /// Attempts to sync to the last received fcu head
    /// Will do nothing if the syncer is already involved in a sync process
    /// If the sync process would require multiple sync cycles (such as snap sync), starts all required sync cycles until the sync is complete
    pub fn start_sync(&self) {
        let syncer = self.syncer.clone();
        let store = self.store.clone();
        let Ok(Some(current_head)) = self.store.get_latest_canonical_block_hash() else {
            tracing::error!("Failed to fecth latest canonical block, unable to sync");
            return;
        };
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
                        tracing::error!("Failed to read latest fcu head, unable to sync");
                        return;
                    };
                    *sync_head
                };
                // Start the sync cycle
                syncer
                    .start_sync(current_head, sync_head, store.clone())
                    .await;
                // Continue to the next sync cycle if we have an ongoing snap sync (aka if we still have snap sync checkpoints stored)
                if store
                    .get_header_download_checkpoint()
                    .ok()
                    .flatten()
                    .is_none()
                {
                    break;
                }
            }
        });
    }

    /// Returns the syncer's current syncmode (either snap or full)
    fn sync_mode(&self) -> SyncMode {
        if self.snap_enabled.load(Ordering::Relaxed) {
            SyncMode::Snap
        } else {
            SyncMode::Full
        }
    }

    /// TODO: Very dirty method that should be removed asap once we move invalid ancestors to the store
    /// Returns a copy of the invalid ancestors if the syncer is not busy
    pub fn invalid_ancestors(&self) -> Option<std::collections::HashMap<H256, H256>> {
        self.syncer
            .try_lock()
            .map(|syncer| syncer.invalid_ancestors.clone())
            .ok()
    }

    /// TODO: Very dirty method that should be removed asap once we move invalid ancestors to the store
    /// Adds a key value pair to invalid ancestors if the syncer is not busy
    pub fn add_invalid_ancestor(&self, k: H256, v: H256) -> bool {
        self.syncer
            .try_lock()
            .map(|mut syncer| syncer.invalid_ancestors.insert(k, v))
            .is_ok()
    }
}
