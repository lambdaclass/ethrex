use ethrex_common::types::AccountInfo;
use ethrex_common::U256 as CoreU256;
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::db::Database as LevmDatabase;

use crate::db::{ExecutionDB, StoreWrapper};

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

    fn get_account_info_by_hash(
        &self,
        block_hash: ethrex_common::types::BlockHash,
        address: CoreAddress,
    ) -> Option<AccountInfo> {
        self.store
            .get_account_info_by_hash(block_hash, address)
            .unwrap()
    }

    fn get_account_code(&self, code_hash: CoreH256) -> Option<bytes::Bytes> {
        self.store.get_account_code(code_hash).unwrap()
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
        self.get_chain_config()
    }

    fn get_account_info_by_hash(
        &self,
        _block_hash: ethrex_common::types::BlockHash,
        _address: CoreAddress,
    ) -> Option<ethrex_common::types::AccountInfo> {
        unreachable!()
    }

    fn get_account_code(&self, _code_hash: CoreH256) -> Option<bytes::Bytes> {
        unreachable!()
    }
}
