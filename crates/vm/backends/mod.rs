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
use levm::LEVM;
use revm::REVM;
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
    /// Wraps [REVM::execute_block] and [LEVM::execute_block].
    /// The output is `(Vec<Receipt>, Vec<AccountUpdate>)`.
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

    /// Wraps [REVM::execute_tx] and [LEVM::execute_tx].
    /// The output is `(Receipt, u64)` == (transaction_receipt, gas_used).
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
                let execution_result = REVM::execute_tx(
                    tx,
                    block_header,
                    state,
                    spec_id(chain_config, block_header.timestamp),
                )?;

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

                let execution_report = LEVM::execute_tx(
                    tx,
                    block_header,
                    store_wrapper.clone(),
                    block_cache.clone(),
                    chain_config,
                )?;

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

    /// Wraps [REVM::beacon_root_contract_call], [REVM::process_block_hash_history]
    /// and [LEVM::beacon_root_contract_call], [LEVM::process_block_hash_history].
    /// This function is used to run/apply all the system contracts to the state.
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
                    REVM::beacon_root_contract_call(block_header, state)?;
                }

                if spec_id >= SpecId::PRAGUE {
                    REVM::process_block_hash_history(block_header, state)?;
                }
                Ok(())
            }
            EVM::LEVM => {
                let fork = chain_config.fork(block_header.timestamp);
                let mut new_state = CacheDB::new();

                if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                    LEVM::beacon_root_contract_call(block_header, state, &mut new_state)?;
                }

                if fork >= Fork::Prague {
                    LEVM::process_block_hash_history(block_header, state, &mut new_state)?;
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

    /// Wraps the [REVM::get_state_transitions] and [LEVM::get_state_transitions].
    /// The output is `Vec<AccountUpdate>`.
    pub fn get_state_transitions(
        &self,
        state: &mut EvmState,
        parent_hash: H256,
        block_cache: &CacheDB,
    ) -> Vec<AccountUpdate> {
        match self {
            EVM::REVM => REVM::get_state_transitions(state),
            EVM::LEVM => LEVM::get_state_transitions(state, parent_hash, block_cache),
        }
    }

    /// Wraps the [REVM::process_withdrawals] and [LEVM::process_withdrawals].
    /// Applies the withdrawals to the state or the block_chache if using [LEVM].
    pub fn process_withdrawals(
        &self,
        withdrawals: &[Withdrawal],
        state: &mut EvmState,
        block_header: &BlockHeader,
        block_cache: &mut CacheDB,
    ) -> Result<(), StoreError> {
        match self {
            EVM::REVM => REVM::process_withdrawals(state, withdrawals),
            EVM::LEVM => {
                let parent_hash = block_header.parent_hash;
                let mut new_state = CacheDB::new();
                LEVM::process_withdrawals(
                    &mut new_state,
                    withdrawals,
                    state.database(),
                    parent_hash,
                )?;
                block_cache.extend(new_state);
                Ok(())
            }
        }
    }
}
