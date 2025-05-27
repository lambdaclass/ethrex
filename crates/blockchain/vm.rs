use bytes::Bytes;
use ethrex_common::{
    types::{AccountInfo, BlockHash, ChainConfig},
    Address, H256, U256,
};
use ethrex_storage::Store;
use ethrex_vm::{EvmError, VmDatabase};
use tracing::debug;

use crate::snapshot::SnapshotTree;

#[derive(Clone)]
pub struct StoreVmDatabase {
    pub store: Store,
    pub block_hash: BlockHash,
    #[cfg(feature = "snapshots")]
    pub snapshots: SnapshotTree,
}

impl StoreVmDatabase {
    #[cfg(not(feature = "snapshots"))]
    pub fn new(store: Store, block_hash: BlockHash) -> Self {
        StoreVmDatabase { store, block_hash }
    }

    #[cfg(feature = "snapshots")]
    pub fn new(store: Store, block_hash: BlockHash, snapshots: SnapshotTree) -> Self {
        StoreVmDatabase {
            store,
            block_hash,
            snapshots,
        }
    }
}

impl VmDatabase for StoreVmDatabase {
    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, EvmError> {
        #[cfg(feature = "snapshots")]
        match self.snapshots.get_account_info(self.block_hash, address) {
            Ok(Some(account_state)) => {
                return Ok(Some(AccountInfo {
                    code_hash: account_state.code_hash,
                    balance: account_state.balance,
                    nonce: account_state.nonce,
                }))
            }
            Ok(None) => {
                return Ok(None);
            }
            Err(snapshot_error) => {
                debug!("failed to fetch snapshot (state): {}", snapshot_error);
            }
        }

        self.store
            .get_account_info_by_hash(self.block_hash, address)
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        #[cfg(feature = "snapshots")]
        match self
            .snapshots
            .get_storage_at_hash(self.block_hash, address, key)
        {
            Ok(value) => return Ok(value),
            // snapshot errors are non-fatal
            Err(snapshot_error) => {
                debug!("failed to fetch snapshot (storage): {}", snapshot_error);
            }
        }

        self.store
            .get_storage_at_hash(self.block_hash, address, key)
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    fn get_block_hash(&self, block_number: u64) -> Result<Option<H256>, EvmError> {
        Ok(self
            .store
            .get_block_header(block_number)
            .map_err(|e| EvmError::DB(e.to_string()))?
            .map(|header| H256::from(header.compute_block_hash().0)))
    }

    fn get_chain_config(&self) -> ChainConfig {
        self.store.get_chain_config().unwrap()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, EvmError> {
        self.store
            .get_account_code(code_hash)
            .map_err(|e| EvmError::DB(e.to_string()))
    }
}
