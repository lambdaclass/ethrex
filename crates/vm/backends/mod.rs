pub mod levm;
pub mod revm;

use self::revm::db::evm_state;
use crate::execution_result::ExecutionResult;
use crate::helpers::{fork_to_spec_id, spec_id, SpecId};
use crate::ExecutionDB;
use crate::{db::StoreWrapper, errors::EvmError};
use ethrex_common::types::requests::Requests;
use ethrex_common::types::{
    AccessList, Block, BlockHeader, Fork, GenericTransaction, Receipt, Transaction, Withdrawal,
};
use ethrex_common::{Address, H256};
use ethrex_levm::db::CacheDB;
use ethrex_levm::vm::GeneralizedDatabase;
use ethrex_storage::Store;
use ethrex_storage::{error::StoreError, AccountUpdate};
use levm::LEVM;
use revm::db::EvmState;
use revm::REVM;
use std::sync::Arc;

#[derive(Debug, PartialEq, Clone, Copy, Default)]
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
    REVM { state: EvmState },
    LEVM { db: GeneralizedDatabase },
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
                db: GeneralizedDatabase::new(
                    Arc::new(StoreWrapper {
                        store: store.clone(),
                        block_hash: parent_hash,
                    }),
                    CacheDB::new(),
                ),
            },
        }
    }

    pub fn from_execution_db(db: ExecutionDB) -> Self {
        Evm::LEVM {
            db: GeneralizedDatabase::new(Arc::new(db), CacheDB::new()),
        }
    }

    pub fn default(store: Store, parent_hash: H256) -> Self {
        Self::new(EvmEngine::default(), store, parent_hash)
    }

    pub fn execute_block(&mut self, block: &Block) -> Result<BlockExecutionResult, EvmError> {
        match self {
            Evm::REVM { state } => {
                let mut state = evm_state(
                    state
                        .database()
                        .ok_or(EvmError::Custom(
                            "Failed to fetch database from EVM State".to_owned(),
                        ))?
                        .clone(),
                    block.header.parent_hash,
                );
                REVM::execute_block(block, &mut state)
            }
            Evm::LEVM { db } => LEVM::execute_block(block, db),
        }
    }

    pub fn execute_block_without_clearing_state(
        &mut self,
        block: &Block,
    ) -> Result<BlockExecutionResult, EvmError> {
        match self {
            Evm::REVM { state } => REVM::execute_block(block, state),
            Evm::LEVM { .. } => {
                // TODO(#2218): LEVM does not support a way  persist the state between block executions
                todo!();
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
            Evm::LEVM { db } => {
                let execution_report = LEVM::execute_tx(tx, sender, block_header, db)?;

                *remaining_gas = remaining_gas.saturating_sub(execution_report.gas_used);

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
            Evm::LEVM { db } => {
                let chain_config = db.store.get_chain_config();
                let fork = chain_config.fork(block_header.timestamp);

                if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                    LEVM::beacon_root_contract_call(block_header, db)?;
                }

                if fork >= Fork::Prague {
                    LEVM::process_block_hash_history(block_header, db)?;
                }

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
        // parent_hash: H256,
        fork: Fork,
    ) -> Result<Vec<AccountUpdate>, EvmError> {
        match self {
            Evm::REVM { state } => Ok(REVM::get_state_transitions(state)),
            Evm::LEVM { db } => LEVM::get_state_transitions(db, fork),
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
            Evm::LEVM { db } => {
                LEVM::process_withdrawals(db, withdrawals, block_header.parent_hash)?;
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
            Evm::LEVM { db } => levm::extract_all_requests_levm(receipts, db, header),
            Evm::REVM { state } => revm::extract_all_requests(receipts, state, header),
        }
    }

    pub fn simulate_tx_from_generic(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
        fork: Fork,
    ) -> Result<ExecutionResult, EvmError> {
        let spec_id = fork_to_spec_id(fork);

        match self {
            Evm::REVM { state } => {
                self::revm::helpers::simulate_tx_from_generic(tx, header, state, spec_id)
            }
            Evm::LEVM { db } => LEVM::simulate_tx_from_generic(tx, header, db),
        }
    }

    pub fn create_access_list(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
        fork: Fork,
    ) -> Result<(u64, AccessList, Option<String>), EvmError> {
        let spec_id = fork_to_spec_id(fork);

        let result = match self {
            Evm::REVM { state } => {
                self::revm::helpers::create_access_list(tx, header, state, spec_id)?
            }

            Evm::LEVM { db } => LEVM::create_access_list(tx.clone(), header, db)?,
        };
        match result {
            (
                ExecutionResult::Success {
                    gas_used,
                    gas_refunded: _,
                    logs: _,
                    output: _,
                },
                access_list,
            ) => Ok((gas_used, access_list, None)),
            (
                ExecutionResult::Revert {
                    gas_used,
                    output: _,
                },
                access_list,
            ) => Ok((
                gas_used,
                access_list,
                Some("Transaction Reverted".to_string()),
            )),
            (ExecutionResult::Halt { reason, gas_used }, access_list) => {
                Ok((gas_used, access_list, Some(reason)))
            }
        }
    }
}

#[derive(Clone)]
pub struct BlockExecutionResult {
    pub receipts: Vec<Receipt>,
    pub requests: Vec<Requests>,
    pub account_updates: Vec<AccountUpdate>,
}
