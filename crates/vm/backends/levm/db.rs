use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ethrex_common::types::Block;
use ethrex_storage::AccountUpdate;

use ethrex_common::U256 as CoreU256;
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::db::Database as LevmDatabase;

use crate::db::{ExecutionDB, StoreWrapper};
use crate::errors::ExecutionDBError;

#[derive(Clone)]
pub struct BlockLogger {
    pub block_hashes_accessed: Arc<Mutex<HashMap<u64, CoreH256>>>,
    pub db: StoreWrapper,
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
        let block_header = self.store.get_block_header(block_number).unwrap();

        block_header.map(|header| CoreH256::from(header.compute_block_hash().0))
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

impl BlockLogger {
    pub fn new(db: StoreWrapper) -> Self {
        Self {
            block_hashes_accessed: Arc::new(Mutex::new(HashMap::new())),
            db,
        }
    }
}

impl LevmDatabase for BlockLogger {
    fn get_account_info(&self, address: CoreAddress) -> ethrex_levm::AccountInfo {
        self.db.get_account_info(address)
    }
    fn account_exists(&self, address: CoreAddress) -> bool {
        self.db.account_exists(address)
    }
    fn get_block_hash(&self, block_number: u64) -> Option<CoreH256> {
        let block_hash = self.db.get_block_hash(block_number);
        self.block_hashes_accessed
            .lock()
            .unwrap()
            .insert(block_number, block_hash.unwrap());
        block_hash
    }
    fn get_chain_config(&self) -> ethrex_common::types::ChainConfig {
        self.db.get_chain_config()
    }
    fn get_storage_slot(&self, address: CoreAddress, key: CoreH256) -> CoreU256 {
        self.db.get_storage_slot(address, key)
    }
}

impl ExecutionDB {
    pub fn pre_execute_levm(
        block: &Block,
        store_wrapper: &StoreWrapper,
    ) -> Result<(Vec<AccountUpdate>, BlockLogger), ExecutionDBError> {
        // this code was copied from the L1
        // TODO: if we change EvmState so that it accepts a CacheDB<RpcDB> then we can
        // simply call execute_block().

        let db = BlockLogger::new(store_wrapper.clone());

        let mut account_updates = vec![];
        // beacon root call
        #[cfg(not(feature = "l2"))]
        {
            let mut cache = HashMap::new();
            crate::backends::levm::LEVM::beacon_root_contract_call(
                &block.header,
                Arc::new(db.clone()),
                &mut cache,
            )
            .map_err(|e| ExecutionDBError::Evm(Box::new(e)))?;
            let account_updates_beacon = crate::backends::levm::LEVM::get_state_transitions(
                None,
                Arc::new(db.clone()),
                &block.header,
                &cache,
            )
            .map_err(|e| ExecutionDBError::Evm(Box::new(e)))?;

            db.db
                .store
                .apply_account_updates(block.hash(), &account_updates_beacon)
                .map_err(ExecutionDBError::Store)?;

            account_updates.extend(account_updates_beacon);
        }

        // execute block
        let report = crate::backends::levm::LEVM::execute_block(block, Arc::new(db.clone()))
            .map_err(Box::new)?;
        account_updates.extend(report.account_updates);

        Ok((account_updates, db))
    }
}
