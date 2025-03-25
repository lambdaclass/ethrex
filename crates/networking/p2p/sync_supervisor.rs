use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
use ethrex_storage::{error::StoreError, Store};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    kademlia::KademliaTable,
    sync::{SyncManager, SyncMode},
};

pub enum SyncStatus {
    Active(SyncMode),
    Inactive,
}

/// Abstraction to interact with the active sync process without disturbing it
#[derive(Debug)]
pub struct SyncSupervisor {
    /// This is also held by the SyncManager and allows tracking it's latest syncmode
    /// It is a READ_ONLY value, as modifications will disrupt the current active sync progress
    snap_enabled: Arc<AtomicBool>,
    syncer: Arc<Mutex<SyncManager>>,
    last_fcu_head: RwLock<H256>,
    store: Store,
}

impl SyncSupervisor {
    pub fn new(
        peer_table: Arc<Mutex<KademliaTable>>,
        sync_mode: SyncMode,
        cancel_token: CancellationToken,
        blockchain: Arc<Blockchain>,
        store: Store,
    ) -> Self {
        let snap_enabled = Arc::new(AtomicBool::new(matches!(sync_mode, SyncMode::Snap)));
        let syncer = Arc::new(Mutex::new(SyncManager::new(
            peer_table,
            snap_enabled.clone(),
            cancel_token,
            blockchain,
        )));
        Self {
            snap_enabled,
            syncer,
            last_fcu_head: RwLock::new(H256::zero()),
            store,
        }
    }

    /// Creates a dummy SyncSupervisor for tests where syncing is not needed
    /// This should only be used in tests as it won't be able to connect to the p2p network
    pub fn dummy() -> Self {
        Self {
            snap_enabled: Arc::new(AtomicBool::new(false)),
            syncer: Arc::new(Mutex::new(SyncManager::dummy())),
            last_fcu_head: RwLock::new(H256::zero()),
            store: Store::new("temp.db", ethrex_storage::EngineType::InMemory)
                .expect("Failed to create test DB"),
        }
    }

    /// Updates the last fcu head. This may be used on the next sync cycle if needed
    pub fn set_head(&self, fcu_head: H256) {
        if let Ok(mut last_fcu_head) = self.last_fcu_head.write() {
            *last_fcu_head = fcu_head
        }
    }

    /// Returns the current sync status, either active or inactive and what the current syncmode is in the case of active
    /// It will also start the next cycle if there is a pending sync
    pub fn status(&self) -> Result<SyncStatus, StoreError> {
        // Check current sync status and act accordingly
        Ok(match self.sync_status_internal()? {
            SyncStatusInternal::Inactive => SyncStatus::Inactive,
            SyncStatusInternal::Active => SyncStatus::Active(self.sync_mode()),
            SyncStatusInternal::Pending => {
                // Start next cycle
                self.start_sync();
                SyncStatus::Active(self.sync_mode())
            }
        })
    }

    /// Attempts to sync to the last received fcu head
    /// Will do nothing if the syncer is already involved in a sync process
    pub fn start_sync(&self) {
        let syncer = self.syncer.clone();
        let Ok(sync_head) = self.last_fcu_head.read() else {
            tracing::error!("Poisoned RwLock, unable to sync");
            return;
        };
        let sync_head = *sync_head;
        let store = self.store.clone();
        let Ok(Some(current_head)) = self.store.get_latest_canonical_block_hash() else {
            tracing::error!("Failed to fecth latest canonical block, unable to sync");
            return;
        };
        tokio::spawn(async move {
            // If we can't get hold of the syncer, then it means that there is an active sync in process
            if let Ok(mut syncer) = syncer.try_lock() {
                syncer.start_sync(current_head, sync_head, store).await
            }
        });
    }

    /// Returns the internal sync status, either active, inactive, or pending (aka, the current cycle stopped due to staleness but the sync is not yet complete)
    fn sync_status_internal(&self) -> Result<SyncStatusInternal, StoreError> {
        // Try to get hold of the sync manager, if we can't then it means it is currently involved in a sync process
        Ok(if self.syncer.try_lock().is_err() {
            SyncStatusInternal::Active
        // Check if there is a checkpoint left from a previous aborted sync
        } else if self.store.get_header_download_checkpoint()?.is_some() {
            SyncStatusInternal::Pending
        // No trace of a sync being handled
        } else {
            SyncStatusInternal::Inactive
        })
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

/// Describes the client's current sync status:
/// Inactive: There is no active sync process
/// Active: The client is currently syncing
/// Pending: The previous sync process became stale, awaiting restart
#[derive(Debug)]
pub enum SyncStatusInternal {
    Inactive,
    Active,
    Pending,
}
