use ethrex_common::types::ChainConfig;
use ethrex_common::{Address as CoreAddress, H256 as CoreH256};
use revm::context::DBErrorMarker;
use revm::primitives::{
    Address as RevmAddress, B256 as RevmB256, Bytes as RevmBytes, U256 as RevmU256,
};
use revm::state::{AccountInfo as RevmAccountInfo, Bytecode as RevmBytecode};

use crate::db::DynVmDatabase;
use crate::{VmDatabase, errors::EvmError};

/// State used when running the EVM. The state can be represented with a [VmDbWrapper] database
///
/// Encapsulates state behaviour to be agnostic to the evm implementation for crate users.
pub struct EvmState {
    pub inner: revm::database::states::State<DynVmDatabase>,
}

// Needed because revm::database::states::State is not cloneable and we need to
// restore the previous EVM state after executing a transaction in L2 mode whose resulting state diff doesn't fit in a blob.
impl Clone for EvmState {
    fn clone(&self) -> Self {
        let inner = revm::database::states::State::<DynVmDatabase> {
            cache: self.inner.cache.clone(),
            database: self.inner.database.clone(),
            transition_state: self.inner.transition_state.clone(),
            bundle_state: self.inner.bundle_state.clone(),
            use_preloaded_bundle: self.inner.use_preloaded_bundle,
            block_hashes: self.inner.block_hashes.clone(),
        };

        Self { inner }
    }
}

impl EvmState {
    /// Gets the stored chain config
    pub fn chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.inner.database.get_chain_config()
    }
}

/// Builds EvmState from a Store
pub fn evm_state(db: DynVmDatabase) -> EvmState {
    let inner = revm::database::states::State::builder()
        .with_database(db)
        .with_bundle_update()
        .without_state_clear()
        .build();
    EvmState { inner }
}

impl DBErrorMarker for EvmError {}

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
        let code = self.code_by_hash(acc_info.code_hash.0.into())?;

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code: Some(code),
        }))
    }

    fn code_by_hash(&mut self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        let code =
            <dyn VmDatabase>::get_account_code(self.as_ref(), CoreH256::from(code_hash.as_ref()))?;
        Ok(RevmBytecode::new_raw(RevmBytes(code)))
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
        <dyn VmDatabase>::get_block_hash(self.as_ref(), number)
            .map(|hash| RevmB256::from_slice(&hash.0))
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
        let code = self.code_by_hash_ref(acc_info.code_hash.0.into())?;

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code: Some(code),
        }))
    }

    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        let code =
            <dyn VmDatabase>::get_account_code(self.as_ref(), CoreH256::from(code_hash.as_ref()))?;
        Ok(RevmBytecode::new_raw(RevmBytes(code)))
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
        <dyn VmDatabase>::get_block_hash(self.as_ref(), number)
            .map(|hash| RevmB256::from_slice(&hash.0))
    }
}
