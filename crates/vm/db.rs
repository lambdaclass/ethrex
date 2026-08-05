use crate::EvmError;
use dyn_clone::DynClone;
use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata},
};

pub trait VmDatabase: Send + Sync + DynClone {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError>;
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError>;
    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError>;
    fn get_chain_config(&self) -> Result<ChainConfig, EvmError>;
    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError>;
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError>;

    /// Batch account-state lookup. Default impl loops `get_account_state`.
    /// Backends that can amortize per-key cost (e.g. rocksdb `multi_get_cf` on
    /// the flat key-value table) should override this.
    fn get_account_states_batch(
        &self,
        addresses: &[Address],
    ) -> Result<Vec<Option<AccountState>>, EvmError> {
        addresses
            .iter()
            .map(|a| self.get_account_state(*a))
            .collect()
    }

    /// Batch storage-slot lookup. Default impl loops `get_storage_slot`.
    /// Backends that can amortize per-key cost (e.g. rocksdb `multi_get_cf` on
    /// the flat key-value table) should override this.
    fn get_storage_slots_batch(
        &self,
        keys: &[(Address, H256)],
    ) -> Result<Vec<Option<U256>>, EvmError> {
        keys.iter()
            .map(|&(addr, key)| self.get_storage_slot(addr, key))
            .collect()
    }
}

dyn_clone::clone_trait_object!(VmDatabase);

pub type DynVmDatabase = Box<dyn VmDatabase + Send + Sync + 'static>;
