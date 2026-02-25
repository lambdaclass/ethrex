use dashmap::DashMap;
use ethrex_common::types::{AccountState, Code};
use ethrex_common::{Address, H256, U256};

/// Lock-free shared cache for prewarmed state.
/// Populated by warmer thread, read by executor thread.
/// Uses DashMap (sharded concurrent hashmap) for fine-grained concurrency.
pub struct WarmCache {
    accounts: DashMap<Address, AccountState>,
    storage: DashMap<(Address, H256), U256>,
    code: DashMap<H256, Code>,
}

impl WarmCache {
    pub fn new() -> Self {
        Self {
            accounts: DashMap::new(),
            storage: DashMap::new(),
            code: DashMap::new(),
        }
    }

    pub fn get_account(&self, addr: &Address) -> Option<AccountState> {
        self.accounts.get(addr).map(|r| *r)
    }

    pub fn get_storage(&self, addr: &Address, key: &H256) -> Option<U256> {
        self.storage.get(&(*addr, *key)).map(|r| *r)
    }

    pub fn get_code(&self, hash: &H256) -> Option<Code> {
        self.code.get(hash).map(|r| r.clone())
    }

    pub fn insert_account(&self, addr: Address, state: AccountState) {
        self.accounts.insert(addr, state);
    }

    pub fn insert_storage(&self, addr: Address, key: H256, value: U256) {
        self.storage.insert((addr, key), value);
    }

    pub fn insert_code(&self, hash: H256, code: Code) {
        self.code.insert(hash, code);
    }
}

impl Default for WarmCache {
    fn default() -> Self {
        Self::new()
    }
}
