use crate::account::{Account, AccountInfo, StorageSlot};
use ethereum_rust_core::{Address, H256, U256};
use std::collections::HashMap;

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

#[derive(Debug, Default, Clone)]
pub struct Cache {
    pub accounts: HashMap<Address, Account>,
}

impl Cache {
    pub fn get_account(&self, address: Address) -> Option<&Account> {
        self.accounts.get(&address)
    }

    pub fn get_mut_account(&mut self, address: Address) -> Option<&mut Account> {
        self.accounts.get_mut(&address)
    }

    pub fn get_storage_slot(&self, address: Address, key: H256) -> Option<StorageSlot> {
        self.get_account(address)
            .expect("Account should have been cached")
            .storage
            .get(&key)
            .cloned()
    }

    pub fn add_account(&mut self, address: &Address, account: &Account) {
        self.accounts.insert(*address, account.clone());
    }

    pub fn write_account_storage(&mut self, address: &Address, key: H256, slot: StorageSlot) {
        self.accounts
            .get_mut(address)
            .expect("Account should have been cached")
            .storage
            .insert(key, slot);
    }

    pub fn increment_account_nonce(&mut self, address: &Address) {
        if let Some(account) = self.accounts.get_mut(address) {
            account.info.nonce += 1;
        }
    }

    pub fn is_account_cached(&self, address: &Address) -> bool {
        self.accounts.contains_key(address)
    }

    pub fn is_slot_cached(&self, address: &Address, key: H256) -> bool {
        self.is_account_cached(address)
            && self
                .get_account(*address)
                .map(|account| account.storage.contains_key(&key))
                .unwrap_or(false)
    }
}
