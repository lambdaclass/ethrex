use ethrex_common::{types::Account, Address};
use keccak_hash::H256;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::StorageSlot;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub struct CacheDB {
    pub cached_accounts: HashMap<Address, Account>,
    pub cached_storages: HashMap<Address, HashMap<H256, StorageSlot>>,
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

    pub fn get_storage_mut(
        &mut self,
        address: &Address,
    ) -> Option<&mut HashMap<H256, StorageSlot>> {
        self.cached_storages.get_mut(address)
    }
    pub fn get_storage(&self, address: &Address) -> Option<&HashMap<H256, StorageSlot>> {
        self.cached_storages.get(address)
    }

    pub fn is_storage_cached(&self, address: &Address) -> bool {
        self.cached_storages.contains_key(address)
    }

    pub fn insert_storage_slot(
        &mut self,
        address: Address,
        key: H256,
        storage_slot: StorageSlot,
    ) -> Option<StorageSlot> {
        self.cached_storages
            .entry(address)
            .or_default()
            .insert(key, storage_slot)
    }
}
