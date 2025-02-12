mod constants;
pub mod levm;
pub mod revm;

use crate::{db::StoreWrapper, errors::EvmError, spec_id, EvmState};
use ethrex_common::types::{Block, BlockHeader, ChainConfig, Receipt, Transaction};
use ethrex_levm::db::CacheDB;
use ethrex_storage::{error::StoreError, AccountUpdate};
use levm::{LevmTransactionExecutionIn, LEVM};
use revm::{RevmTransactionExecutionIn, REVM};
use std::str::FromStr;
use std::sync::Arc;

#[derive(Debug, Default, Clone)]
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

impl EVM {
    /// Wraps [IEVM::execute_block]. The output is `(Vec<Receipt>, Vec<AccountUpdate>)`
    pub fn execute_block(
        &self,
        block: &Block,
        state: &mut EvmState,
    ) -> Result<(Vec<Receipt>, Vec<AccountUpdate>), EvmError> {
        match self {
            EVM::REVM => REVM::execute_block(block, state),
            EVM::LEVM => LEVM::execute_block(block, state),
        }
    }

    pub fn execute_tx(
        &self,
        state: &mut EvmState,
        tx: &Transaction,
        block_header: &BlockHeader,
        block_cache: &mut CacheDB,
        chain_config: &ChainConfig,
        remaining_gas: &mut u64,
    ) -> Result<(Receipt, u64), EvmError> {
        match self {
            EVM::REVM => {
                let input = RevmTransactionExecutionIn::new(
                    tx,
                    block_header,
                    state,
                    spec_id(chain_config, block_header.timestamp),
                );
                let execution_result = REVM::execute_tx(input)?;

                *remaining_gas = remaining_gas.saturating_sub(execution_result.gas_used());

                let receipt = Receipt::new(
                    tx.tx_type(),
                    execution_result.is_success(),
                    block_header.gas_limit - *remaining_gas,
                    execution_result.logs(),
                );

                Ok((receipt, execution_result.gas_used()))
            }
            EVM::LEVM => {
                let store_wrapper = Arc::new(StoreWrapper {
                    store: state.database().unwrap().clone(),
                    block_hash: block_header.parent_hash,
                });

                let input = LevmTransactionExecutionIn::new(
                    tx,
                    block_header,
                    store_wrapper.clone(),
                    block_cache,
                    chain_config,
                );
                let execution_report = LEVM::execute_tx(input)?;

                *remaining_gas = remaining_gas.saturating_sub(execution_report.gas_used);

                let mut new_state = execution_report.new_state.clone();

                // Now original_value is going to be the same as the current_value, for the next transaction.
                // It should have only one value but it is convenient to keep on using our CacheDB structure
                for account in new_state.values_mut() {
                    for storage_slot in account.storage.values_mut() {
                        storage_slot.original_value = storage_slot.current_value;
                    }
                }
                block_cache.extend(new_state);

                let receipt = Receipt::new(
                    tx.tx_type(),
                    execution_report.is_success(),
                    block_header.gas_limit - *remaining_gas,
                    execution_report.logs.clone(),
                );
                Ok((receipt, execution_report.gas_used))
            }
        }
    }

    #[allow(dead_code)]
    fn get_state_transitions() -> Vec<AccountUpdate> {
        todo!()
    }

    #[allow(dead_code)]
    fn process_withdrawals() -> Result<(), StoreError> {
        todo!()
    }
}

pub trait IEVM {
    /// The error type for this trait's error. `EvmError` is the default, but it could be modified if needed.
    type Error;

    /// Output for [IEVM::execute_block]. The default is `(Vec<Receipt>, Vec<AccountUpdate>)`, but it could be modified if needed.
    type BlockExecutionOutput;

    /// Input for [IEVM::execute_tx]. This must be defined by the implementor. It may vary depending on the backend EVM.
    type TransactionExecutionInput<'a>;

    /// Output for [IEVM::execute_tx]. This must be defined by the implementor. It may vary depending on the backend EVM.
    type TransactionExecutionResult;

    /// Input for [IEVM::get_state_transitions]. This must be defined by the implementor. It may vary depending on the backend EVM.
    type GetStateTransitionsInput<'a>;

    /// Input for [IEVM::process_withdrawals]. This must be defined by the implementor. It may vary depending on the backend EVM.
    type ProcessWithdrawalsInput<'a>;

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

    /// Processes a block's withdrawals, updating the account balances in the state
    fn process_withdrawals(input: Self::ProcessWithdrawalsInput<'_>) -> Result<(), StoreError>;
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

    fn process_block_hash_history(
        block_header: &BlockHeader,
        input: Self::SystemCallInput<'_>,
    ) -> Result<<Self::Evm as IEVM>::TransactionExecutionResult, Self::Error>;
}
