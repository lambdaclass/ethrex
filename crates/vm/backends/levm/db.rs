use bytes::Bytes;
use ethrex_common::types::{AccountInfo, BlockHash, BlockHeader, ChainConfig};
use ethrex_common::U256 as CoreU256;
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use ethrex_levm::db::Database as LevmDatabase;
use ethrex_storage::error::StoreError;
use ethrex_storage::AccountUpdate;
use ethrex_trie::Trie;

use crate::db::StoreWrapper;

impl LevmDatabase for StoreWrapper {
    fn get_account_info(&self, address: CoreAddress) -> ethrex_levm::account::AccountInfo {
        match self {
            StoreWrapper::StoreDB(store, block_hash) => {
                let acc_info = store
                    .get_account_info_by_hash(*block_hash, address)
                    .unwrap_or(None)
                    .unwrap_or_default();

                let acc_code = store
                    .get_account_code(acc_info.code_hash)
                    .unwrap()
                    .unwrap_or_default();

                ethrex_levm::account::AccountInfo {
                    balance: acc_info.balance,
                    nonce: acc_info.nonce,
                    bytecode: acc_code,
                }
            }
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }

    fn account_exists(&self, address: CoreAddress) -> bool {
        match self {
            StoreWrapper::StoreDB(store, block_hash) => {
                let acc_info = store
                    .get_account_info_by_hash(*block_hash, address)
                    .unwrap();

                acc_info.is_some()
            }
            StoreWrapper::ExecutionCache(_, _call_block_variable) => unimplemented!(),
        }
    }

    fn get_storage_slot(&self, address: CoreAddress, key: CoreH256) -> CoreU256 {
        match self {
            StoreWrapper::StoreDB(store, block_hash) => store
                .get_storage_at_hash(*block_hash, address, key)
                .unwrap()
                .unwrap_or_default(),
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }

    fn get_block_hash(&self, block_number: u64) -> Option<CoreH256> {
        match self {
            StoreWrapper::StoreDB(store, _) => {
                let block_header = store.get_block_header(block_number).unwrap();

                block_header.map(|header| CoreH256::from(header.compute_block_hash().0))
            }
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }
}

impl StoreWrapper {
    pub fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        match self {
            StoreWrapper::StoreDB(store, _) => store.get_chain_config(),
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }

    pub fn get_account_info_by_hash(
        &self,
        block_hash: BlockHash,
        address: CoreAddress,
    ) -> Result<Option<AccountInfo>, StoreError> {
        match self {
            StoreWrapper::StoreDB(store, _) => store.get_account_info_by_hash(block_hash, address),
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }

    pub fn get_account_code(&self, code_hash: CoreH256) -> Result<Option<Bytes>, StoreError> {
        match self {
            StoreWrapper::StoreDB(store, _) => store.get_account_code(code_hash),
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        match self {
            StoreWrapper::StoreDB(store, _) => store.get_block_header_by_hash(block_hash),
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }

    pub fn get_storage_at_hash(
        &self,
        block_hash: BlockHash,
        address: CoreAddress,
        key: CoreH256,
    ) -> Result<Option<CoreU256>, StoreError> {
        match self {
            StoreWrapper::StoreDB(store, _) => store.get_storage_at_hash(block_hash, address, key),
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }

    pub fn get_block_header(&self, block_number: u64) -> Result<Option<BlockHeader>, StoreError> {
        match self {
            StoreWrapper::StoreDB(store, _) => store.get_block_header(block_number),
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }

    pub fn state_trie(&self, block_hash: BlockHash) -> Result<Option<Trie>, StoreError> {
        match self {
            StoreWrapper::StoreDB(store, _) => store.state_trie(block_hash),
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }

    pub fn storage_trie(
        &self,
        block_hash: BlockHash,
        address: CoreAddress,
    ) -> Result<Option<Trie>, StoreError> {
        match self {
            StoreWrapper::StoreDB(store, _) => store.storage_trie(block_hash, address),
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }

    pub fn apply_account_updates(
        &mut self,
        block_hash: BlockHash,
        account_updates: &[AccountUpdate],
    ) -> Result<Option<CoreH256>, StoreError> {
        match self {
            StoreWrapper::StoreDB(store, _) => {
                store.apply_account_updates(block_hash, account_updates)
            }
            StoreWrapper::ExecutionCache(_, _) => unimplemented!(),
        }
    }
}
