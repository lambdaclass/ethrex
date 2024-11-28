use crate::account::{Account, AccountInfo, StorageSlot};
use ethrex_core::{Address, H256, U256};
use std::collections::HashMap;

pub mod cache;
pub use cache::CacheDB;

pub trait Database {
    fn get_account_info(&self, address: Address) -> AccountInfo;
    fn get_storage_slot(&self, address: Address, key: H256) -> U256;
    fn get_block_hash(&self, block_number: u64) -> Option<H256>;
}

#[derive(Debug, Default)]
pub struct Db {
    pub accounts: HashMap<Address, Account>,
    pub block_hashes: HashMap<u64, H256>,
}

// Methods here are for testing purposes only, for initializing the Db with some values
impl Db {
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
            block_hashes: HashMap::new(),
        }
    }

    /// Add accounts to database
    pub fn add_accounts(&mut self, accounts: Vec<(Address, Account)>) {
        self.accounts.extend(accounts);
    }

    /// Add block hashes to database
    pub fn add_block_hashes(&mut self, block_hashes: Vec<(u64, H256)>) {
        self.block_hashes.extend(block_hashes);
    }

    /// Builder method with accounts [for testing only]
    pub fn with_accounts(mut self, accounts: HashMap<Address, Account>) -> Self {
        self.accounts = accounts;
        self
    }

    /// Builder method with block hashes [for testing only]
    pub fn with_block_hashes(mut self, block_hashes: HashMap<u64, H256>) -> Self {
        self.block_hashes = block_hashes;
        self
    }
}

impl Database for Db {
    fn get_account_info(&self, address: Address) -> AccountInfo {
        self.accounts
            .get(&address)
            .unwrap_or(&Account::default())
            .info
            .clone()
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> U256 {
        // both `original_value` and `current_value` should work here because they have the same values on Db
        self.accounts
            .get(&address)
            .unwrap_or(&Account::default())
            .storage
            .get(&key)
            .unwrap_or(&StorageSlot::default())
            .original_value
    }

    fn get_block_hash(&self, block_number: u64) -> Option<H256> {
        self.block_hashes.get(&block_number).cloned()
    }
}
