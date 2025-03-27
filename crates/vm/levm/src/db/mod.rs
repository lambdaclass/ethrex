use crate::account::AccountInfo;
use ethrex_common::{types::ChainConfig, Address, H256, U256};

pub mod cache;
pub use cache::CacheDB;

pub trait Database {
    fn get_account_info(&self, address: Address) -> AccountInfo;
    fn get_storage_slot(&self, address: Address, key: H256) -> U256;
    fn get_block_hash(&self, block_number: u64) -> Option<H256>;
    fn account_exists(&self, address: Address) -> bool;
    fn get_chain_config(&self) -> ChainConfig;
}
