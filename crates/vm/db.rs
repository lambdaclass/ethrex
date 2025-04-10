use bytes::Bytes;
use ethrex_common::types::BlockHash;
use ethrex_common::{
    types::{AccountInfo, ChainConfig},
    Address, H256, U256,
};
use ethrex_storage::error::StoreError;
use ethrex_storage::Store;

#[derive(Clone)]
pub struct StoreWrapper {
    pub store: Store,
    pub block_hash: BlockHash,
}

pub struct Wrapper<T>(pub T);

// impl<T> Deref for Wrapper<T> {
//     type Target = T;

//     fn deref(&self) -> &Self::Target {
//         &self.0
//     }
// }

pub trait Database: Send + Sync {
    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, StoreError>;
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, StoreError>;
    fn get_block_hash(&self, block_number: u64) -> Result<Option<H256>, StoreError>;
    fn get_chain_config(&self) -> ChainConfig;
    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError>;
    fn get_account_info_by_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError>;
}

impl Database for StoreWrapper {
    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, StoreError> {
        self.store
            .get_account_info_by_hash(self.block_hash, address)
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, StoreError> {
        self.store
            .get_storage_at_hash(self.block_hash, address, key)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<Option<H256>, StoreError> {
        self.store
            .get_block_header(block_number)
            .map(|maybe_header| {
                maybe_header.map(|header| H256::from(header.compute_block_hash().0))
            })
    }

    fn get_chain_config(&self) -> ChainConfig {
        self.store.get_chain_config().unwrap()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        self.store.get_account_code(code_hash)
    }

    fn get_account_info_by_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        self.store.get_account_info_by_hash(block_hash, address)
    }
}
