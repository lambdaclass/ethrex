use crate::errors::DatabaseError;
use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{Account, ChainConfig},
};
use std::collections::HashMap;

pub mod gen_db;

pub type CacheDB = HashMap<Address, Account>;

pub trait Database: Send + Sync {
    fn get_account(&self, address: Address) -> Result<Account, DatabaseError>;
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError>;
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, DatabaseError>;
}
