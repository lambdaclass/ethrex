mod constants;
pub mod levm;
pub mod revm;

use crate::{db::StoreWrapper, errors::EvmError, spec_id, EvmState, SpecId};
use ethrex_common::types::{
    Block, BlockHeader, ChainConfig, Fork, Receipt, Transaction, Withdrawal,
};
use ethrex_common::H256;
use ethrex_levm::db::CacheDB;
use ethrex_storage::{error::StoreError, AccountUpdate};
use levm::{
    LevmGetStateTransitionsIn, LevmProcessWithdrawalsIn, LevmSystemCallIn,
    LevmTransactionExecutionIn, LEVM,
};
use revm::{
    RevmGetStateTransitionsIn, RevmProcessWithdrawalsIn, RevmSystemCallIn,
    RevmTransactionExecutionIn, REVM,
};
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
    /// Wraps [IEVM::execute_block]. The output is `(Vec<Receipt>, Vec<AccountUpdate>)`.
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

    /// Wraps [IEVM::execute_tx]. The output is `(Receipt, u64)` == (transaction_receipt, gas_used).
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

    /// Wraps the [SystemContracts] trait. This function is used to run/apply all the system contracts to the state.
    pub fn apply_system_calls(
        &self,
        state: &mut EvmState,
        block_header: &BlockHeader,
        block_cache: &mut CacheDB,
        chain_config: &ChainConfig,
    ) -> Result<(), EvmError> {
        match self {
            EVM::REVM => {
                let spec_id = spec_id(chain_config, block_header.timestamp);
                if block_header.parent_beacon_block_root.is_some() && spec_id >= SpecId::CANCUN {
                    REVM::beacon_root_contract_call(
                        block_header,
                        RevmSystemCallIn::new(state, spec_id),
                    )?;
                }

                if spec_id >= SpecId::PRAGUE {
                    REVM::process_block_hash_history(
                        block_header,
                        RevmSystemCallIn::new(state, spec_id),
                    )?;
                }
                Ok(())
            }
            EVM::LEVM => {
                let store_wrapper = Arc::new(StoreWrapper {
                    store: state.database().unwrap().clone(),
                    block_hash: block_header.parent_hash,
                });

                let fork = chain_config.fork(block_header.timestamp);
                let mut new_state = CacheDB::new();

                if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                    let report = LEVM::beacon_root_contract_call(
                        block_header,
                        LevmSystemCallIn::new(store_wrapper.clone(), chain_config),
                    )?;

                    new_state.extend(report.new_state);
                }

                if fork >= Fork::Prague {
                    let report = LEVM::process_block_hash_history(
                        block_header,
                        LevmSystemCallIn::new(store_wrapper.clone(), chain_config),
                    )?;

                    new_state.extend(report.new_state);
                }

                // Now original_value is going to be the same as the current_value, for the next transaction.
                // It should have only one value but it is convenient to keep on using our CacheDB structure
                for account in new_state.values_mut() {
                    for storage_slot in account.storage.values_mut() {
                        storage_slot.original_value = storage_slot.current_value;
                    }
                }

                block_cache.extend(new_state);
                Ok(())
            }
        }
    }

    /// Wraps the [IEVM::get_state_transitions]. The output is `Vec<AccountUpdate>`.
    pub fn get_state_transitions(
        &self,
        state: &mut EvmState,
        parent_hash: H256,
        block_cache: &CacheDB,
    ) -> Vec<AccountUpdate> {
        match self {
            EVM::REVM => REVM::get_state_transitions(RevmGetStateTransitionsIn::new(state)),
            EVM::LEVM => LEVM::get_state_transitions(LevmGetStateTransitionsIn::new(
                state,
                parent_hash,
                block_cache,
            )),
        }
    }

    /// Wraps the [IEVM::process_withdrawals]. Applies the withdrawals to the state or the block_chache if using [LEVM].
    pub fn process_withdrawals(
        &self,
        withdrawals: &[Withdrawal],
        state: &mut EvmState,
        block_header: &BlockHeader,
        block_cache: &mut CacheDB,
    ) -> Result<(), StoreError> {
        match self {
            EVM::REVM => {
                REVM::process_withdrawals(RevmProcessWithdrawalsIn::new(state, withdrawals))
            }
            EVM::LEVM => {
                let parent_hash = block_header.parent_hash;
                let mut new_state = CacheDB::new();
                LEVM::process_withdrawals(LevmProcessWithdrawalsIn::new(
                    &mut new_state,
                    withdrawals,
                    state.database(),
                    parent_hash,
                ))?;
                block_cache.extend(new_state);
                Ok(())
            }
        }
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
