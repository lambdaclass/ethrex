use ethrex_common::H256;
use ethrex_rlp::error::RLPDecodeError;

use crate::error::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("Snapshot with block hash {0} not found.")]
    SnapshotNotFound(H256),
    #[error("Snapshot with block hash {0} is the disk layer, wrong api usage.")]
    SnapshotIsdiskLayer(H256),
    #[error("Tried to create a snapshot cycle")]
    SnapshotCycle,
    #[error("Parent snaptshot not found, parent = {0}, block = {1}")]
    ParentSnapshotNotFound(H256, H256),
    #[error("Tried to use a stale snapshot")]
    StaleSnapshot,
    #[error("Tried to use flatten on a disk layer")]
    DiskLayerFlatten,
    #[error(transparent)]
    RLPDecodeError(#[from] RLPDecodeError),
    #[error("Error getting a lock: {0}")]
    LockError(String),
    #[error(transparent)]
    StoreError(#[from] StoreError),
}
