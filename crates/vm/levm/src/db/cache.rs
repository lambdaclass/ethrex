use ethrex_common::{types::Account, Address};
use keccak_hash::H256;
use std::collections::HashMap;

use crate::StorageSlot;

#[derive(Clone)]
pub struct CacheDB {
    cached_accounts: HashMap<Address, Account>,
    cached_storages: HashMap<Address, HashMap<H256, StorageSlot>>,
}

impl CacheDB {
    pub fn get_account(&self, address: &Address) -> Option<&Account> {
        self.cached_accounts.get(address)
    }

    pub fn get_account_mut(&mut self, address: &Address) -> Option<&mut Account> {
        self.cached_accounts.get_mut(address)
    }

    pub fn insert_account(&mut self, address: Address, account: Account) -> Option<Account> {
        self.cached_accounts.insert(address, account)
    }

    pub fn remove_account(&mut self, address: &Address) -> Option<Account> {
        self.cached_accounts.remove(address)
    }

    pub fn is_account_cached(&self, address: &Address) -> bool {
        self.cached_accounts.contains_key(address)
    }

    pub fn get_storage_slot(&self, address: &Address, key: H256) -> Option<&StorageSlot> {
        self.cached_storages
            .get(address)
            .and_then(|storage| storage.get(&key))
    }
}
