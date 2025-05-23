use bytes::Bytes;
use ethrex_common::{
    types::{AccountInfo, BlockHash, ChainConfig},
    Address, H256, U256,
};
use ethrex_storage::Store;
use ethrex_vm::{EvmError, VmDatabase};

#[derive(Clone)]
pub struct StoreVmDatabase {
    pub store: Store,
    pub block_hash: BlockHash,
}

impl StoreVmDatabase {
    pub fn new(store: Store, block_hash: BlockHash) -> Self {
        StoreVmDatabase { store, block_hash }
    }
}

impl VmDatabase for StoreVmDatabase {
    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, EvmError> {
        self.store
            .get_account_info_by_hash(self.block_hash, address)
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
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
