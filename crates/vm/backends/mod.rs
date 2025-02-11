pub mod levm;
pub mod revm;

use crate::{errors::EvmError, EvmState};
use ethrex_common::types::{Block, BlockHeader};
use ethrex_storage::AccountUpdate;
use std::str::FromStr;

#[derive(Debug, Clone, Default)]
pub enum EVM {
    #[default]
    REVM,
    LEVM,
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

pub trait IEVM {
    /// The error type for this trait's error. `EvmError` is the default, but it could be modified if needed.
    type Error;

    /// Output for `execute_block`. The default is `(Vec<Receipt>, Vec<AccountUpdate>)`, but it could be modified if needed.
    type BlockExecutionOutput;

    /// Input for `execute_tx`. This must be defined by the implementor. It may vary depending on the backend EVM.
    type TransactionExecutionInput<'a>;

    /// Output for `execute_tx`. This must be defined by the implementor. It may vary depending on the backend EVM.
    type TransactionExecutionResult;

    /// Input for `get_state_transitions`. This must be defined by the implementor. It may vary depending on the backend EVM.
    type GetStateTransitionsInput<'a>;

    /// Executes every transaction of a block returning a list of their receipts executed and a list of accounts that were updated in the execution.
    fn execute_block(
        block: &Block,
        state: &mut EvmState,
    ) -> Result<Self::BlockExecutionOutput, Self::Error>;

    /// Executes a transaction returning its execution result. It may vary depending on the EVM used.
    fn execute_tx(
        input: Self::TransactionExecutionInput<'_>,
    ) -> Result<Self::TransactionExecutionResult, Self::Error>;

    /// Gets the state transitions performed by the execution. Returning an Array of AccounUpdates
    fn get_state_transitions(input: Self::GetStateTransitionsInput<'_>) -> Vec<AccountUpdate>;
}

pub trait SystemContracts {
    /// The error type for this trait's error. `EvmError` is the default, but it could be modified if needed.
    type Error;

    type Evm: IEVM;

    /// Input for `beacon_root_contract_call`. This must be defined by the implementor. It may vary depending on the backend EVM.
    /// Calls the eip4788 beacon block root system call contract
    /// As of the Cancun hard-fork, parent_beacon_block_root needs to be present in the block header.
    type SystemCallInput<'a>;
    fn beacon_root_contract_call(
        block_header: &BlockHeader,
        input: Self::SystemCallInput<'_>,
    ) -> Result<<Self::Evm as IEVM>::TransactionExecutionResult, Self::Error>;
}
