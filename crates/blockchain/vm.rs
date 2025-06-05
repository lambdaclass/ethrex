use bytes::Bytes;
use ethrex_common::{
    types::{AccountInfo, BlockHash, ChainConfig, EMPTY_KECCACK_HASH},
    Address, H256, U256,
};
use ethrex_storage::Store;
use ethrex_vm::{EvmError, VmDatabase};
use std::cmp::Ordering;

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

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        for ancestor_res in self.store.ancestors(self.block_hash) {
            let (hash, ancestor) = ancestor_res.map_err(|e| EvmError::DB(e.to_string()))?;
            match ancestor.number.cmp(&block_number) {
                Ordering::Greater => continue,
                Ordering::Equal => return Ok(hash),
                Ordering::Less => {
                    return Err(EvmError::DB(format!(
                        "Block number requested {} is higher than the current block number {}",
                        block_number, ancestor.number
                    )))
                }
            }
        }

        Err(EvmError::DB(format!(
            "Block hash not found for block number {block_number}"
        )))
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.store
            .get_chain_config()
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, EvmError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Bytes::new());
        }
        match self.store.get_account_code(code_hash) {
            Ok(Some(code)) => Ok(code),
            Ok(None) => Err(EvmError::DB(format!(
                "Code not found for hash: {:?}",
                code_hash
            ))),
            Err(e) => Err(EvmError::DB(e.to_string())),
        }
    }
}
