use std::cell::RefCell;
use std::collections::HashMap;

use ethrex_common::U256 as CoreU256;
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::db::Database as LevmDatabase;
use ethrex_levm::AccountInfo;

use crate::db::{ExecutionDB, StoreWrapper};

pub struct StoreLogger {
    pub accounts: RefCell<HashMap<CoreAddress, AccountInfo>>,
    pub block_hashes: RefCell<HashMap<u64, CoreH256>>,
    pub storage_slots: RefCell<HashMap<CoreAddress, HashMap<CoreH256, CoreU256>>>,
    pub store_wrapper: StoreWrapper,
}

impl StoreLogger {
    pub fn new(store_wrapper: StoreWrapper) -> Self {
        Self {
            accounts: RefCell::new(HashMap::new()),
            block_hashes: RefCell::new(HashMap::new()),
            storage_slots: RefCell::new(HashMap::new()),
            store_wrapper,
        }
    }
}

impl LevmDatabase for StoreWrapper {
    fn get_account_info(&self, address: CoreAddress) -> ethrex_levm::account::AccountInfo {
        let acc_info = self
            .store
            .get_account_info_by_hash(self.block_hash, address)
            .unwrap_or(None)
            .unwrap_or_default();

        let acc_code = self
            .store
            .get_account_code(acc_info.code_hash)
            .unwrap()
            .unwrap_or_default();

        ethrex_levm::account::AccountInfo {
            balance: acc_info.balance,
            nonce: acc_info.nonce,
            bytecode: acc_code,
        }
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        let acc_info = self
            .store
            .get_account_info_by_hash(self.block_hash, address)
            .unwrap();

        acc_info.is_some()
    }

    fn get_storage_slot(&self, address: CoreAddress, key: CoreH256) -> CoreU256 {
        self.store
            .get_storage_at_hash(self.block_hash, address, key)
            .unwrap()
            .unwrap_or_default()
    }

    fn get_block_hash(&self, block_number: u64) -> Option<CoreH256> {
        let a = self.store.get_block_header(block_number).unwrap();

        a.map(|a| CoreH256::from(a.compute_block_hash().0))
    }

    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        self.store.get_chain_config().unwrap()
    }
}

impl LevmDatabase for ExecutionDB {
    fn get_account_info(&self, address: CoreAddress) -> ethrex_levm::AccountInfo {
        let Some(acc_info) = self.accounts.get(&address) else {
            return ethrex_levm::AccountInfo::default();
        };
        let acc_code = self.code.get(&acc_info.code_hash).unwrap();
        ethrex_levm::AccountInfo {
            balance: acc_info.balance,
            bytecode: acc_code.clone(),
            nonce: acc_info.nonce,
        }
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        self.accounts.contains_key(&address)
    }

    fn get_block_hash(&self, block_number: u64) -> Option<CoreH256> {
        self.block_hashes.get(&block_number).cloned()
    }

    fn get_storage_slot(&self, address: CoreAddress, key: CoreH256) -> CoreU256 {
        let Some(storage) = self.storage.get(&address) else {
            return CoreU256::default();
        };
        *storage.get(&key).unwrap_or(&CoreU256::default())
    }

    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        self.chain_config
    }
}

impl LevmDatabase for StoreLogger {
    fn get_account_info(&self, address: CoreAddress) -> ethrex_levm::AccountInfo {
        let acc_info = self.store_wrapper.get_account_info(address);
        self.accounts.borrow_mut().insert(address, acc_info.clone());
        acc_info
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        self.store_wrapper.account_exists(address)
    }

    fn get_block_hash(&self, block_number: u64) -> Option<CoreH256> {
        let block_hash = self.store_wrapper.get_block_hash(block_number);
        self.block_hashes
            .borrow_mut()
            .insert(block_number, block_hash.unwrap());
        block_hash
    }

    fn get_storage_slot(&self, address: CoreAddress, key: CoreH256) -> CoreU256 {
        let storage_slot = self.store_wrapper.get_storage_slot(address, key);
        self.storage_slots
            .borrow_mut()
            .entry(address)
            .or_default()
            .insert(key, storage_slot);
        storage_slot
    }

    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        self.store_wrapper.get_chain_config()
    }
}
