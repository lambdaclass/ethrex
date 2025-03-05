mod constants;
pub mod levm;
pub mod revm_b;

use crate::db::evm_state;
use crate::{db::StoreWrapper, errors::EvmError, spec_id, EvmState, SpecId};
use ethrex_common::types::requests::Requests;
use ethrex_common::types::{Block, BlockHeader, Fork, Receipt, Transaction, Withdrawal};
use ethrex_common::{Address, H256};
use ethrex_levm::db::CacheDB;
use ethrex_storage::Store;
use ethrex_storage::{error::StoreError, AccountUpdate};
use levm::LEVM;
use revm_b::REVM;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Default)]
pub enum EvmEngine {
    #[default]
    REVM,
    LEVM,
}

// Allow conversion from string for backward compatibility
impl TryFrom<String> for EvmEngine {
    type Error = EvmError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "revm" => Ok(EvmEngine::REVM),
            "levm" => Ok(EvmEngine::LEVM),
            _ => Err(EvmError::InvalidEVM(s)),
        }
    }
}

pub enum Evm {
    REVM {
        state: EvmState,
    },
    LEVM {
        store_wrapper: StoreWrapper,
        block_cache: CacheDB,
    },
}

impl std::fmt::Debug for Evm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Evm::REVM { .. } => write!(f, "REVM"),
            Evm::LEVM { .. } => {
                write!(f, "LEVM")
            }
        }
    }
}

impl Evm {
    /// Creates a new EVM instance, but with block hash in zero, so if we want to execute a block or transaction we have to set it.
    pub fn new(engine: EvmEngine, store: Store, parent_hash: H256) -> Self {
        match engine {
            EvmEngine::REVM => Evm::REVM {
                state: evm_state(store.clone(), parent_hash),
            },
            EvmEngine::LEVM => Evm::LEVM {
                store_wrapper: StoreWrapper {
                    store: store.clone(),
                    block_hash: parent_hash,
                },
                block_cache: CacheDB::new(),
            },
        }
    }

    pub fn default(store: Store, parent_hash: H256) -> Self {
        Self::new(EvmEngine::default(), store, parent_hash)
    }

    pub fn execute_block(&mut self, block: &Block) -> Result<BlockExecutionResult, EvmError> {
        match self {
            Evm::REVM { state } => {
                let mut state =
                    evm_state(state.database().unwrap().clone(), block.header.parent_hash);
                REVM::execute_block(block, &mut state)
            }
            Evm::LEVM { store_wrapper, .. } => {
                LEVM::execute_block(block, store_wrapper.store.clone())
            }
        }
    }

    /// Wraps [REVM::execute_tx] and [LEVM::execute_tx].
    /// The output is `(Receipt, u64)` == (transaction_receipt, gas_used).
    #[allow(clippy::too_many_arguments)]
    pub fn execute_tx(
        &mut self,
        tx: &Transaction,
        block_header: &BlockHeader,
        remaining_gas: &mut u64,
        sender: Address,
    ) -> Result<(Receipt, u64), EvmError> {
        match self {
            Evm::REVM { state } => {
                let chain_config = state.chain_config()?;
                let execution_result = REVM::execute_tx(
                    tx,
                    block_header,
                    state,
                    spec_id(&chain_config, block_header.timestamp),
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
            Evm::LEVM {
                store_wrapper,
                block_cache,
            } => {
                store_wrapper.block_hash = block_header.parent_hash; // JIC
                let store = store_wrapper.store.clone();
                let chain_config = store.get_chain_config()?;

                let execution_report = LEVM::execute_tx(
                    tx,
                    block_header,
                    Arc::new(store_wrapper.clone()),
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
    pub fn apply_system_calls(&mut self, block_header: &BlockHeader) -> Result<(), EvmError> {
        match self {
            Evm::REVM { state } => {
                let chain_config = state.chain_config()?;
                let spec_id = spec_id(&chain_config, block_header.timestamp);
                if block_header.parent_beacon_block_root.is_some() && spec_id >= SpecId::CANCUN {
                    REVM::beacon_root_contract_call(block_header, state)?;
                }

                if spec_id >= SpecId::PRAGUE {
                    REVM::process_block_hash_history(block_header, state)?;
                }
                Ok(())
            }
            Evm::LEVM {
                store_wrapper,
                block_cache,
            } => {
                let store = store_wrapper.store.clone();
                let chain_config = store.get_chain_config()?;
                let fork = chain_config.fork(block_header.timestamp);
                let mut new_state = CacheDB::new();

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
        &mut self,
        parent_hash: H256,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        match self {
            Evm::REVM { state } => Ok(REVM::get_state_transitions(state)),
            Evm::LEVM {
                store_wrapper,
                block_cache,
            } => {
                store_wrapper.block_hash = parent_hash;
                LEVM::get_state_transitions(None, store_wrapper, block_cache)
            }
        }
    }

    /// Wraps the [REVM::process_withdrawals] and [LEVM::process_withdrawals].
    /// Applies the withdrawals to the state or the block_chache if using [LEVM].
    pub fn process_withdrawals(
        &mut self,
        withdrawals: &[Withdrawal],
        block_header: &BlockHeader,
    ) -> Result<(), StoreError> {
        match self {
            Evm::REVM { state } => REVM::process_withdrawals(state, withdrawals),
            Evm::LEVM {
                store_wrapper,
                block_cache,
            } => {
                let parent_hash = block_header.parent_hash;
                let mut new_state = CacheDB::new();
                LEVM::process_withdrawals(
                    &mut new_state,
                    withdrawals,
                    &store_wrapper.store,
                    parent_hash,
                )?;
                block_cache.extend(new_state);
                Ok(())
            }
        }
    }

    pub fn extract_requests(
        &mut self,
        receipts: &[Receipt],
        header: &BlockHeader,
    ) -> Result<Vec<Requests>, EvmError> {
        match self {
            Evm::LEVM {
                store_wrapper,
                block_cache,
            } => {
                levm::extract_all_requests_levm(receipts, &store_wrapper.store, header, block_cache)
            }
            Evm::REVM { state } => revm_b::extract_all_requests(receipts, state, header),
        }
    }
}

pub struct BlockExecutionResult {
    pub receipts: Vec<Receipt>,
    pub requests: Vec<Requests>,
    pub account_updates: Vec<AccountUpdate>,
}
