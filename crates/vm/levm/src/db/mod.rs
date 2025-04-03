use crate::account::AccountInfo;
use bytes::Bytes;
use error::DatabaseError;
use ethrex_common::{
    types::{BlockHash, ChainConfig},
    Address, H256, U256,
};

pub mod cache;
pub use cache::CacheDB;
pub mod error;

pub trait Database {
    fn get_account_info(&self, address: Address) -> Result<AccountInfo, DatabaseError>;
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    fn get_block_hash(&self, block_number: u64) -> Result<Option<H256>, DatabaseError>;
    fn account_exists(&self, address: Address) -> bool;
    fn get_chain_config(&self) -> ChainConfig;
    fn get_account_info_by_hash(
        &self,
        block_hash: BlockHash,
        address: Address,
    ) -> Result<Option<ethrex_common::types::AccountInfo>, DatabaseError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, DatabaseError>;
}
