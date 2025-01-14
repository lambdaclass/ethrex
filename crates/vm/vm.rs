pub mod db;
pub mod errors;
pub mod execution_db;
mod execution_result;
#[cfg(feature = "levm")]
pub mod levm;
#[cfg(feature = "l2")]
mod mods;
pub mod revm;

use db::StoreWrapper;
use ethrex_core::types::{Block, ChainConfig, Receipt};
use ethrex_storage::{AccountUpdate, Store};
use execution_db::ExecutionDB;
use std::str::FromStr;

// Export needed types
pub use errors::EvmError;
pub use execution_result::*;

pub type BlockExecutionOutput = (Vec<Receipt>, Vec<AccountUpdate>);

pub const WITHDRAWAL_MAGIC_DATA: &[u8] = b"burn";
pub const DEPOSIT_MAGIC_DATA: &[u8] = b"mint";

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

impl From<ExecutionDB> for EvmState {
    fn from(value: ExecutionDB) -> Self {
        EvmState::Execution(Box::new(revm::db::CacheDB::new(value)))
    }
}

#[derive(Debug, Clone)]
pub enum EVM {
    LEVM,
    REVM,
}

impl FromStr for EVM {
    type Err = EvmError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "levm" => Ok(EVM::LEVM),
            "revm" => Ok(EVM::REVM),
            _ => Err(EvmError::InvalidEVM(s.to_string())),
        }
    }
}

pub fn execute_block(
    block: &Block,
    state: &mut EvmState,
    evm: &EVM,
) -> Result<BlockExecutionOutput, EvmError> {
    match evm {
        EVM::LEVM => {
            cfg_if::cfg_if! {
                if #[cfg(feature = "levm")] {
                    levm::execute_block(block, state)
                } else {
                    Err(EvmError::InvalidEVM("Using EVM::LEVM but levm feature is not enabled".to_owned()))
                }
            }
        }
        EVM::REVM => revm::execute_block(block, state),
    }
}
