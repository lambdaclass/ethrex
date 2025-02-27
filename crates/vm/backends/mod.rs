mod constants;
pub mod levm;
pub mod revm_b;

use crate::db::evm_state;
use crate::{db::StoreWrapper, errors::EvmError, spec_id, EvmState, SpecId};
use ethrex_common::types::requests::Requests;
use ethrex_common::types::{Block, BlockHeader, Fork, Receipt, Transaction, Withdrawal};
use ethrex_common::H256;
use ethrex_levm::db::CacheDB;
use ethrex_storage::Store;
use ethrex_storage::{error::StoreError, AccountUpdate};
use levm::LEVM;
use revm_b::REVM;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Debug, Default, Clone)]
pub enum EvmImplementation {
    #[default]
    REVM,
    LEVM,
}

#[derive(Debug, Clone)]
pub struct EVM {
    pub evm_impl: EvmImplementation,
    pub storage: Store,
}

impl EVM {
    pub fn new(evm: EvmImplementation, storage: Store) -> Self {
        Self {
            evm_impl: evm,
            storage,
        }
    }

    pub fn default_with_storage(storage: Store) -> Self {
        Self {
            evm_impl: EvmImplementation::REVM,
            storage,
        }
    }

    pub fn execute_block(&self, block: &Block) -> Result<BlockExecutionResult, EvmError> {
        match self.evm_impl {
            EvmImplementation::REVM => {
                let mut state = evm_state(self.storage.clone(), block.header.parent_hash);
                REVM::execute_block(block, &mut state)
            }
            EvmImplementation::LEVM => LEVM::execute_block(block, self.storage.clone()),
        }
    }

    /// Wraps [REVM::execute_tx] and [LEVM::execute_tx].
    /// The output is `(Receipt, u64)` == (transaction_receipt, gas_used).
    #[allow(clippy::too_many_arguments)]
    pub fn execute_tx(
        &self,
        state: &mut EvmState,
        tx: &Transaction,
        block_header: &BlockHeader,
        block_cache: &mut CacheDB,
        remaining_gas: &mut u64,
        sender: Address,
    ) -> Result<(Receipt, u64), EvmError> {
        let chain_config = self.storage.get_chain_config()?;

        match self.evm_impl {
            EvmImplementation::REVM => {
                let execution_result = REVM::execute_tx(
                    tx,
                    block_header,
                    state,
                    spec_id(chain_config, block_header.timestamp),
                    sender,
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
            EvmImplementation::LEVM => {
                let store_wrapper = Arc::new(StoreWrapper {
                    store: self.storage.clone(),
                    block_hash: block_header.parent_hash,
                });

                let execution_report = LEVM::execute_tx(
                    tx,
                    block_header,
                    store_wrapper.clone(),
                    block_cache.clone(),
                    &chain_config,
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
    ) -> Result<(), EvmError> {
        let chain_config = self.storage.get_chain_config()?;
        match self.evm_impl {
            EvmImplementation::REVM => {
                let spec_id = spec_id(&chain_config, block_header.timestamp);
                if block_header.parent_beacon_block_root.is_some() && spec_id >= SpecId::CANCUN {
                    REVM::beacon_root_contract_call(block_header, state)?;
                }

                if spec_id >= SpecId::PRAGUE {
                    REVM::process_block_hash_history(block_header, state)?;
                }
                Ok(())
            }
            EvmImplementation::LEVM => {
                let fork = chain_config.fork(block_header.timestamp);
                let mut new_state = CacheDB::new();
                let store = self.storage.clone();

                if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                    LEVM::beacon_root_contract_call(block_header, &store, &mut new_state)?;
                }

                if fork >= Fork::Prague {
                    LEVM::process_block_hash_history(block_header, &store, &mut new_state)?;
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
    /// WARNING:
    /// [REVM::get_state_transitions] gathers the information from the DB, the functionality of this function
    /// is used in [LEVM::execute_block].
    /// [LEVM::get_state_transitions] gathers the information from a [CacheDB].
    ///
    /// They may have the same name, but they serve for different purposes.
    pub fn get_state_transitions(
        &self,
        state: &mut EvmState,
        parent_hash: H256,
        block_cache: &CacheDB,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        match self.evm_impl {
            EvmImplementation::REVM => Ok(REVM::get_state_transitions(state)),
            EvmImplementation::LEVM => {
                LEVM::get_state_transitions(None, &self.storage, parent_hash, block_cache)
            }
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
        match self.evm_impl {
            EvmImplementation::REVM => REVM::process_withdrawals(state, withdrawals),
            EvmImplementation::LEVM => {
                let parent_hash = block_header.parent_hash;
                let mut new_state = CacheDB::new();
                LEVM::process_withdrawals(&mut new_state, withdrawals, &self.storage, parent_hash)?;
                block_cache.extend(new_state);
                Ok(())
            }
        }
    }

    pub fn extract_requests(
        &self,
        receipts: &[Receipt],
        state: &mut EvmState,
        header: &BlockHeader,
        cache: &mut CacheDB,
    ) -> Result<Vec<Requests>, EvmError> {
        match self.evm_impl {
            EvmImplementation::LEVM => {
                levm::extract_all_requests_levm(receipts, &self.storage, header, cache)
            }
            EvmImplementation::REVM => revm_b::extract_all_requests(receipts, state, header),
        }
    }
}

impl FromStr for EvmImplementation {
    type Err = EvmError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "levm" => Ok(EvmImplementation::LEVM),
            "revm" => Ok(EvmImplementation::REVM),
            _ => Err(EvmError::InvalidEVM(s.to_string())),
        }
    }
}

pub struct BlockExecutionResult {
    pub receipts: Vec<Receipt>,
    pub requests: Vec<Requests>,
    pub account_updates: Vec<AccountUpdate>,
}
