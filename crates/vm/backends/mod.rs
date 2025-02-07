pub mod levm;
pub mod revm;

use crate::{errors::EvmError, EvmState};
use ethrex_core::types::Block;
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
    type TransactionExecutionInput;

    /// Output for `execute_tx`. This must be defined by the implementor. It may vary depending on the backend EVM.
    type TransactionExecutionResult;

    /// Input for `get_state_transitions`. This must be defined by the implementor. It may vary depending on the backend EVM.
    type GetStateTransitionsInput;

    /// Executes every transaction of a block returning a list of their receipts executed and a list of accounts that were updated in the execution.
    fn execute_block(
        block: &Block,
        state: &mut EvmState,
    ) -> Result<Self::BlockExecutionOutput, Self::Error>;

    /// Executes a transaction returning its execution result. It may vary depending on the EVM used.
    fn execute_tx(
        input: Self::TransactionExecutionInput,
    ) -> Result<Self::TransactionExecutionResult, Self::Error>;

    fn get_state_transitions(input: Self::GetStateTransitionsInput) -> Vec<AccountUpdate>;
}
