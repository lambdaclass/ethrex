use ethrex_common::{types::Account, Address};
use keccak_hash::H256;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::StorageSlot;

#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheDB {
    pub cached_accounts: HashMap<Address, (Account, HashMap<H256, StorageSlot>)>,
}

impl CacheDB {
    pub fn get_account(&self, address: &Address) -> Option<&(Account, HashMap<H256, StorageSlot>)> {
        self.cached_accounts.get(address)
    }

    pub fn get_account_mut(
        &mut self,
        address: &Address,
    ) -> Option<&mut (Account, HashMap<H256, StorageSlot>)> {
        self.cached_accounts.get_mut(address)
    }

    pub fn insert_account(
        &mut self,
        address: Address,
        account: Account,
        storage: HashMap<H256, StorageSlot>,
    ) -> Option<(Account, HashMap<H256, StorageSlot>)> {
        self.cached_accounts.insert(address, (account, storage))
    }

    pub fn remove_account(
        &mut self,
        address: &Address,
    ) -> Option<(Account, HashMap<H256, StorageSlot>)> {
        self.cached_accounts.remove(address)
    }

    pub fn is_account_cached(&self, address: &Address) -> bool {
        self.cached_accounts.contains_key(address)
    }
}
