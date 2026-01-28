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
}

dyn_clone::clone_trait_object!(VmDatabase);

pub type DynVmDatabase = Box<dyn VmDatabase + Send + Sync + 'static>;
