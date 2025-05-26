use ethrex_common::{types::ChainConfig, Address as CoreAddress, H256 as CoreH256};
use revm::{
    primitives::{
        AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
        Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256,
    },
    DatabaseRef,
};

use crate::{db::DynVmDatabase, prover_db::ProverDB};
use crate::{
    errors::{EvmError, ProverDBError},
    VmDatabase,
};

/// State used when running the EVM. The state can be represented with a [VmDbWrapper] database, or
/// with a [ProverDB] in case we only want to store the necessary data for some particular
/// execution, for example when proving in L2 mode.
///
/// Encapsulates state behaviour to be agnostic to the evm implementation for crate users.
pub enum EvmState {
    Store(revm::db::State<DynVmDatabase>),
    Execution(Box<revm::db::CacheDB<ProverDB>>),
}

// Needed because revm::db::State is not cloneable and we need to
// restore the previous EVM state after executing a transaction in L2 mode whose resulting state diff doesn't fit in a blob.
impl Clone for EvmState {
    fn clone(&self) -> Self {
        match self {
            EvmState::Store(state) => EvmState::Store(revm::db::State::<DynVmDatabase> {
                cache: state.cache.clone(),
                database: state.database.clone(),
                transition_state: state.transition_state.clone(),
                bundle_state: state.bundle_state.clone(),
                use_preloaded_bundle: state.use_preloaded_bundle,
                block_hashes: state.block_hashes.clone(),
            }),
            EvmState::Execution(execution) => {
                EvmState::Execution(Box::new(Into::<revm::db::CacheDB<ProverDB>>::into(
                    *execution.clone(),
                )))
            }
        }
    }
}

impl EvmState {
    /// Gets the stored chain config
    pub fn chain_config(&self) -> Result<ChainConfig, EvmError> {
        match self {
            EvmState::Store(db) => Ok(db.database.get_chain_config()),
            EvmState::Execution(db) => Ok(db.db.get_chain_config()),
        }
    }
}

/// Builds EvmState from a Store
pub fn evm_state(db: DynVmDatabase) -> EvmState {
    EvmState::Store(
        revm::db::State::builder()
            .with_database(db)
            .with_bundle_update()
            .without_state_clear()
            .build(),
    )
}

impl From<ProverDB> for EvmState {
    fn from(value: ProverDB) -> Self {
        EvmState::Execution(Box::new(revm::db::CacheDB::new(value)))
    }
}

impl DatabaseRef for ProverDB {
    /// The database error type.
    type Error = ProverDBError;

    /// Get basic account information.
    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let Some(account_info) = self.accounts.get(&CoreAddress::from(address.0.as_ref())) else {
            return Ok(None);
        };

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(account_info.balance.0),
            nonce: account_info.nonce,
            code_hash: RevmB256::from_slice(&account_info.code_hash.0),
            code: None,
        }))
    }

    /// Get account code by its hash.
    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.code
            .get(&CoreH256::from(code_hash.as_ref()))
            .map(|b| RevmBytecode::new_raw(RevmBytes(b.clone())))
            .ok_or(ProverDBError::CodeNotFound(code_hash))
    }

    /// Get storage value of address at index.
    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        self.storage
            .get(&CoreAddress::from(address.0.as_ref()))
            .ok_or(ProverDBError::AccountNotFound(address))?
            .get(&CoreH256::from(index.to_be_bytes()))
            .map(|v| RevmU256::from_limbs(v.0))
            .ok_or(ProverDBError::StorageValueNotFound(address, index))
    }

    /// Get block hash by block number.
    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        self.block_hashes
            .get(&number)
            .map(|h| RevmB256::from_slice(&h.0))
            .ok_or(ProverDBError::BlockHashNotFound(number))
    }
}

impl revm::Database for DynVmDatabase {
    type Error = EvmError;

    fn basic(&mut self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match <dyn VmDatabase>::get_account_info(
            self.as_ref(),
            CoreAddress::from(address.0.as_ref()),
        )? {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = <dyn VmDatabase>::get_account_code(self.as_ref(), acc_info.code_hash)?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)));

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code,
        }))
    }

    fn code_by_hash(&mut self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        <dyn VmDatabase>::get_account_code(self.as_ref(), CoreH256::from(code_hash.as_ref()))?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)))
            .ok_or_else(|| EvmError::DB(format!("No code for hash {code_hash}")))
    }

    fn storage(&mut self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(<dyn VmDatabase>::get_storage_slot(
            self.as_ref(),
            CoreAddress::from(address.0.as_ref()),
            CoreH256::from(index.to_be_bytes()),
        )?
        .map(|value| RevmU256::from_limbs(value.0))
        .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash(&mut self, number: u64) -> Result<RevmB256, Self::Error> {
        <dyn VmDatabase>::get_block_hash(self.as_ref(), number)?
            .map(|hash| RevmB256::from_slice(&hash.0))
            .ok_or_else(|| EvmError::DB(format!("Block {number} not found")))
    }
}

impl revm::DatabaseRef for DynVmDatabase {
    type Error = EvmError;

    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match <dyn VmDatabase>::get_account_info(
            self.as_ref(),
            CoreAddress::from(address.0.as_ref()),
        )? {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = <dyn VmDatabase>::get_account_code(self.as_ref(), acc_info.code_hash)?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)));

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code,
        }))
    }

    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        <dyn VmDatabase>::get_account_code(self.as_ref(), CoreH256::from(code_hash.as_ref()))?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)))
            .ok_or_else(|| EvmError::DB(format!("No code for hash {code_hash}")))
    }

    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(<dyn VmDatabase>::get_storage_slot(
            self.as_ref(),
            CoreAddress::from(address.0.as_ref()),
            CoreH256::from(index.to_be_bytes()),
        )?
        .map(|value| RevmU256::from_limbs(value.0))
        .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        <dyn VmDatabase>::get_block_hash(self.as_ref(), number)?
            .map(|hash| RevmB256::from_slice(&hash.0))
            .ok_or_else(|| EvmError::DB(format!("Block {number} not found")))
    }
}
