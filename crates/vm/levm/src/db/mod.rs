use crate::errors::DatabaseError;
use bytes::Bytes;
pub use cache::CacheDB;
use ethrex_common::{
    types::{Account, ChainConfig},
    Address, H256, U256,
};

pub mod cache;
pub mod gen_db;

pub trait Database: Send + Sync {
    fn get_account(&self, address: Address) -> Result<Account, DatabaseError>;
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError>;
    fn account_exists(&self, address: Address) -> Result<bool, DatabaseError>;
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, DatabaseError>;
}
