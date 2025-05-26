use crate::EvmError;
use bytes::Bytes;
use dyn_clone::DynClone;
use ethrex_common::{
    types::{AccountInfo, ChainConfig},
    Address, H256, U256,
};

pub trait VmDatabase: Send + Sync + DynClone {
    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, EvmError>;
    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError>;
    fn get_block_hash(&self, block_number: u64) -> Result<Option<H256>, EvmError>;
    fn get_chain_config(&self) -> ChainConfig;
    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, EvmError>;
}

dyn_clone::clone_trait_object!(VmDatabase);

pub type DynVmDatabase = Box<dyn VmDatabase + Send + Sync + 'static>;
