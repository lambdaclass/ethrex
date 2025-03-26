use ethrex_common::{types::BlockHash, Address as CoreAddress, H256 as CoreH256};
use ethrex_storage::{error::StoreError, Store};
use revm::primitives::{
    AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
    Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256,
};

use crate::{ChainConfig, EvmError, ExecutionDB, StoreWrapper};
/// State used when running the EVM. The state can be represented with a [StoreWrapper] database, or
/// with a [ExecutionDB] in case we only want to store the necessary data for some particular
/// execution, for example when proving in L2 mode.
///
/// Encapsulates state behaviour to be agnostic to the evm implementation for crate users.
pub enum EvmState {
    Store(revm::db::State<StoreWrapper>),
    Execution(Box<revm::db::CacheDB<ExecutionDB>>),
}

impl EvmState {
    /// Get a reference to inner `Store` database
    pub fn database(&self) -> Option<&Store> {
        if let EvmState::Store(db) = self {
            Some(&db.database.store)
        } else {
            None
        }
    }

    /// Gets the stored chain config
    pub fn chain_config(&self) -> Result<ChainConfig, EvmError> {
        match self {
            EvmState::Store(db) => db.database.store.get_chain_config().map_err(EvmError::from),
            EvmState::Execution(db) => Ok(db.db.get_chain_config()),
        }
    }
}

/// Builds EvmState from a Store
pub fn evm_state(store: Store, block_hash: BlockHash) -> EvmState {
    EvmState::Store(
        revm::db::State::builder()
            .with_database(StoreWrapper { store, block_hash })
            .with_bundle_update()
            .without_state_clear()
            .build(),
    )
}

impl From<ExecutionDB> for EvmState {
    fn from(value: ExecutionDB) -> Self {
        EvmState::Execution(Box::new(revm::db::CacheDB::new(value)))
    }
}

impl revm::Database for StoreWrapper {
    type Error = StoreError;

    fn basic(&mut self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match self
            .store
            .get_account_info_by_hash(self.block_hash, CoreAddress::from(address.0.as_ref()))?
        {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = self
            .store
            .get_account_code(acc_info.code_hash)?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)));

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code,
        }))
    }

    fn code_by_hash(&mut self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.store
            .get_account_code(CoreH256::from(code_hash.as_ref()))?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)))
            .ok_or_else(|| StoreError::Custom(format!("No code for hash {code_hash}")))
    }

    fn storage(&mut self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(self
            .store
            .get_storage_at_hash(
                self.block_hash,
                CoreAddress::from(address.0.as_ref()),
                CoreH256::from(index.to_be_bytes()),
            )?
            .map(|value| RevmU256::from_limbs(value.0))
            .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash(&mut self, number: u64) -> Result<RevmB256, Self::Error> {
        self.store
            .get_block_header(number)?
            .map(|header| RevmB256::from_slice(&header.compute_block_hash().0))
            .ok_or_else(|| StoreError::Custom(format!("Block {number} not found")))
    }
}

impl revm::DatabaseRef for StoreWrapper {
    type Error = StoreError;

    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match self
            .store
            .get_account_info_by_hash(self.block_hash, CoreAddress::from(address.0.as_ref()))?
        {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = self
            .store
            .get_account_code(acc_info.code_hash)?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)));

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code,
        }))
    }

    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.store
            .get_account_code(CoreH256::from(code_hash.as_ref()))?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)))
            .ok_or_else(|| StoreError::Custom(format!("No code for hash {code_hash}")))
    }

    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(self
            .store
            .get_storage_at_hash(
                self.block_hash,
                CoreAddress::from(address.0.as_ref()),
                CoreH256::from(index.to_be_bytes()),
            )?
            .map(|value| RevmU256::from_limbs(value.0))
            .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        self.store
            .get_block_header(number)?
            .map(|header| RevmB256::from_slice(&header.compute_block_hash().0))
            .ok_or_else(|| StoreError::Custom(format!("Block {number} not found")))
    }
}
