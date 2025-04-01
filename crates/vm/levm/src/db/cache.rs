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
    pub fn get_account(
        &self,
        address: &Address,
    ) -> Option<(&Account, &HashMap<H256, StorageSlot>)> {
        if let Some(account) = self.cached_accounts.get(address) {
            if let Some(account_storage) = self.cached_storages.get(address) {
                return Some((account, account_storage));
            }
        }
        None
    }

    pub fn get_account_mut(
        &mut self,
        address: &Address,
    ) -> Option<(&mut Account, &mut HashMap<H256, StorageSlot>)> {
        if let Some(account) = self.cached_accounts.get_mut(address) {
            if let Some(account_storage) = self.cached_storages.get_mut(address) {
                return Some((account, account_storage));
            }
        }
        None
    }

    pub fn insert_account(
        &mut self,
        address: Address,
        account: Account,
        storage: HashMap<H256, StorageSlot>,
    ) -> Option<Account> {
        self.cached_storages.insert(address, storage);
        self.cached_accounts.insert(address, account)
    }

    pub fn remove_account(&mut self, address: &Address) -> Option<Account> {
        self.cached_storages.remove(address);
        self.cached_accounts.remove(address)
    }

    pub fn is_account_cached(&self, address: &Address) -> bool {
        self.cached_accounts.contains_key(address)
    }

    // check this behavior
    pub fn extend_cache(&mut self, new_state: CacheDB) {
        self.cached_accounts.extend(new_state.cached_accounts);
        self.cached_storages.extend(new_state.cached_storages);
    }
}
