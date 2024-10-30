use std::collections::HashMap;

use ethereum_rust_core::types::{Block, ChainConfig};
use ethereum_rust_storage::Store;
use revm::{
    primitives::{
        AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
        B256 as RevmB256, U256 as RevmU256,
    },
    Database, DatabaseRef,
};
use serde::{Deserialize, Serialize};

use crate::{
    db::StoreWrapper, errors::ExecutionDBError, evm_state, execute_block, get_state_transitions,
};

/// In-memory EVM database for caching execution data.
///
/// This is mainly used to store the relevant state data for executing a particular block and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionDB {
    /// indexed by account address
    accounts: HashMap<RevmAddress, RevmAccountInfo>,
    /// indexed by code hash
    code: HashMap<RevmB256, RevmBytecode>,
    /// indexed by account address and storage slot
    storage: HashMap<RevmAddress, HashMap<RevmU256, RevmU256>>,
    /// indexed by block number
    block_hashes: HashMap<u64, RevmB256>,
    /// stored chain config
    chain_config: ChainConfig,
}

impl ExecutionDB {
    /// Creates a database by executing a block, without performing any validation.
    pub fn from_exec(block: &Block, store: &Store) -> Result<Self, ExecutionDBError> {
        // TODO: perform validation to exit early

        let mut state = evm_state(store.clone(), block.header.parent_hash);
        let mut store_wrapper = StoreWrapper {
            store: store.clone(),
            block_hash: block.header.parent_hash,
        };

        let chain_config = store.get_chain_config()?;

        execute_block(block, &mut state).map_err(Box::new)?;

        let account_updates = get_state_transitions(&mut state);

        let mut accounts = HashMap::new();
        let code = HashMap::new(); // TODO: `code` remains empty for now
        let mut storage = HashMap::new();
        let block_hashes = HashMap::new(); // TODO: `block_hashes` remains empty for now

        for account_update in account_updates {
            let address = RevmAddress::from_slice(account_update.address.as_bytes());
            let account_info = store_wrapper
                .basic(address)?
                .ok_or(ExecutionDBError::NewMissingAccountInfo(address))?;
            accounts.insert(address, account_info);

            let account_storage = account_update
                .added_storage
                .iter()
                .map(|(slot, value)| {
                    let mut value_bytes = [0u8; 32];
                    value.to_big_endian(&mut value_bytes);
                    (
                        RevmU256::from_be_bytes(slot.to_fixed_bytes()),
                        RevmU256::from_be_slice(&value_bytes),
                    )
                })
                .collect();

            storage.insert(address, account_storage);
        }

        Ok(Self {
            accounts,
            code,
            storage,
            block_hashes,
            chain_config,
        })
    }

    pub fn get_chain_config(&self) -> ChainConfig {
        self.chain_config
    }
}

impl DatabaseRef for ExecutionDB {
    /// The database error type.
    type Error = ExecutionDBError;

    /// Get basic account information.
    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        Ok(self.accounts.get(&address).cloned())
    }

    /// Get account code by its hash.
    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.code
            .get(&code_hash)
            .cloned()
            .ok_or(ExecutionDBError::CodeNotFound(code_hash))
    }

    /// Get storage value of address at index.
    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        self.storage
            .get(&address)
            .ok_or(ExecutionDBError::AccountNotFound(address))?
            .get(&index)
            .cloned()
            .ok_or(ExecutionDBError::StorageNotFound(address, index))
    }

    /// Get block hash by block number.
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        self.block_hashes
            .get(&number)
            .cloned()
            .ok_or(ExecutionDBError::BlockHashNotFound(number))
    }
}
