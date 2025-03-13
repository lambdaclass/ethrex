use ethrex_common::U256 as CoreU256;
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::db::Database as LevmDatabase;

use crate::db::StoreWrapper;

impl LevmDatabase for StoreWrapper {
    fn get_account_info(&self, address: CoreAddress) -> Option<ethrex_levm::account::AccountInfo> {
        let acc_info = self
            .store
            .get_account_info_by_hash(self.block_hash, address).ok()??;

        let acc_code = self
            .store
            .get_account_code(acc_info.code_hash).ok()??;

        Some(ethrex_levm::account::AccountInfo {
            balance: acc_info.balance,
            nonce: acc_info.nonce,
            bytecode: acc_code,
        })
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
}
