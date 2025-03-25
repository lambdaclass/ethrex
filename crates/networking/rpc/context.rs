use crate::eth::filter;
use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_p2p::types::Node;
use ethrex_p2p::{sync::SyncManager, types::NodeRecord};
use ethrex_storage::{error::StoreError, Store};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;

cfg_if::cfg_if! {
    if #[cfg(feature = "l2")] {
        use ethrex_common::Address;
        use secp256k1::SecretKey;
    }
}

#[derive(Debug, Clone)]
pub struct RpcApiContext {
    pub storage: Store,
    pub blockchain: Arc<Blockchain>,
    pub jwt_secret: Bytes,
    pub local_p2p_node: Node,
    pub local_node_record: NodeRecord,
    pub active_filters: Arc<Mutex<HashMap<u64, (std::time::Instant, filter::PollableFilter)>>>,
    pub syncer: Arc<TokioMutex<SyncManager>>,
    #[cfg(feature = "based")]
    pub gateway_eth_client: crate::clients::EthClient,
    #[cfg(feature = "based")]
    pub gateway_auth_client: crate::clients::EngineClient,
    #[cfg(feature = "l2")]
    pub valid_delegation_addresses: Vec<Address>,
    #[cfg(feature = "l2")]
    pub sponsor_pk: SecretKey,
}

/// Describes the client's current sync status:
/// Inactive: There is no active sync process
/// Active: The client is currently syncing
/// Pending: The previous sync process became stale, awaiting restart
#[derive(Debug)]
pub enum SyncStatus {
    Inactive,
    Active,
    Pending,
}

impl RpcApiContext {
    /// Returns the engine's current sync status, see [SyncStatus]
    pub fn sync_status(&self) -> Result<SyncStatus, StoreError> {
        // Try to get hold of the sync manager, if we can't then it means it is currently involved in a sync process
        Ok(if self.syncer.try_lock().is_err() {
            SyncStatus::Active
        // Check if there is a checkpoint left from a previous aborted sync
        } else if self.storage.get_header_download_checkpoint()?.is_some() {
            SyncStatus::Pending
        // No trace of a sync being handled
        } else {
            SyncStatus::Inactive
        })
    }
}

pub const FILTER_DURATION: Duration = {
    if cfg!(test) {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(5 * 60)
    }
};