pub mod levm;
pub mod revm;

use self::revm::db::evm_state;
use crate::db::{DynVmDatabase, VmDatabase};
use crate::errors::EvmError;
use crate::execution_result::ExecutionResult;
use crate::helpers::{SpecId, fork_to_spec_id, spec_id};
use ethrex_common::Address;
use ethrex_common::types::requests::Requests;
use ethrex_common::types::{
    AccessList, AccountUpdate, Block, BlockHeader, Fork, GenericTransaction, Receipt, Transaction,
    Withdrawal,
};
pub use ethrex_levm::call_frame::CallFrameBackup;
use ethrex_levm::db::Database as LevmDatabase;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::vm::VMType;
use levm::LEVM;
use revm::REVM;
use revm::db::EvmState;
use std::fmt;
use std::sync::Arc;
use tracing::instrument;

#[derive(Debug, PartialEq, Clone, Copy, Default)]
pub enum EvmEngine {
    #[default]
    LEVM,
    REVM,
}

impl fmt::Display for EvmEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvmEngine::LEVM => write!(f, "levm"),
            EvmEngine::REVM => write!(f, "revm"),
        }
    }
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

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum Evm {
    REVM {
        state: EvmState,
    },
    LEVM {
        db: GeneralizedDatabase,
        vm_type: VMType,
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
    pub fn new_for_l1(engine: EvmEngine, db: impl VmDatabase + 'static) -> Self {
        let wrapped_db: DynVmDatabase = Box::new(db);

        match engine {
            EvmEngine::REVM => Evm::REVM {
                state: evm_state(wrapped_db),
            },
            EvmEngine::LEVM => Evm::LEVM {
                db: GeneralizedDatabase::new(Arc::new(wrapped_db)),
                vm_type: VMType::L1,
            },
        }
    }

    pub fn new_for_l2(engine: EvmEngine, db: impl VmDatabase + 'static) -> Result<Self, EvmError> {
        if let EvmEngine::REVM = engine {
            return Err(EvmError::InvalidEVM(
                "REVM is not supported for L2".to_string(),
            ));
        }

        let wrapped_db: DynVmDatabase = Box::new(db);

        let evm = Evm::LEVM {
            db: GeneralizedDatabase::new(Arc::new(wrapped_db)),
            vm_type: VMType::L2,
        };

        Ok(evm)
    }

    pub fn new_from_db_for_l1(store: Arc<impl LevmDatabase + 'static>) -> Self {
        Self::_new_from_db(store, VMType::L1)
    }

    pub fn new_from_db_for_l2(store: Arc<impl LevmDatabase + 'static>) -> Self {
        Self::_new_from_db(store, VMType::L2)
    }

    fn _new_from_db(store: Arc<impl LevmDatabase + 'static>, vm_type: VMType) -> Self {
        Evm::LEVM {
            db: GeneralizedDatabase::new(store),
            vm_type,
        }
    }

    #[instrument(level = "trace", name = "Block execution", skip_all)]
    pub fn execute_block(&mut self, block: &Block) -> Result<BlockExecutionResult, EvmError> {
        match self {
            Evm::REVM { state } => REVM::execute_block(block, state),
            Evm::LEVM { db, vm_type } => LEVM::execute_block(block, db, *vm_type),
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
            Evm::LEVM { db, vm_type } => {
                let execution_report = LEVM::execute_tx(tx, sender, block_header, db, *vm_type)?;

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

    pub fn undo_last_tx(&mut self) -> Result<(), EvmError> {
        match self {
            Evm::REVM { .. } => Err(EvmError::InvalidEVM(
                "Undoing transaction not supported in REVM".to_string(),
            )),
            Evm::LEVM { db, .. } => LEVM::undo_last_tx(db),
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
            Evm::LEVM { db, vm_type } => {
                let chain_config = db.store.get_chain_config()?;
                let fork = chain_config.fork(block_header.timestamp);

                if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                    LEVM::beacon_root_contract_call(block_header, db, *vm_type)?;
                }

                if fork >= Fork::Prague {
                    LEVM::process_block_hash_history(block_header, db, *vm_type)?;
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
    pub fn get_state_transitions(&mut self) -> Result<Vec<AccountUpdate>, EvmError> {
        match self {
            Evm::REVM { state } => Ok(REVM::get_state_transitions(state)),
            Evm::LEVM { db, .. } => LEVM::get_state_transitions(db),
        }
    }

    /// Wraps the [REVM::process_withdrawals] and [LEVM::process_withdrawals].
    /// Applies the withdrawals to the state or the block_chache if using [LEVM].
    pub fn process_withdrawals(&mut self, withdrawals: &[Withdrawal]) -> Result<(), EvmError> {
        match self {
            Evm::REVM { state } => REVM::process_withdrawals(state, withdrawals),
            Evm::LEVM { db, .. } => LEVM::process_withdrawals(db, withdrawals),
        }
    }

    pub fn extract_requests(
        &mut self,
        receipts: &[Receipt],
        header: &BlockHeader,
    ) -> Result<Vec<Requests>, EvmError> {
        match self {
            Evm::LEVM { db, vm_type } => {
                levm::extract_all_requests_levm(receipts, db, header, *vm_type)
            }
            Evm::REVM { state } => revm::extract_all_requests(receipts, state, header),
        }
    }

    pub fn simulate_tx_from_generic(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
        fork: Fork,
    ) -> Result<ExecutionResult, EvmError> {
        match self {
            Evm::REVM { state } => {
                let spec_id = fork_to_spec_id(fork);
                self::revm::helpers::simulate_tx_from_generic(tx, header, state, spec_id)
            }
            Evm::LEVM { db, vm_type } => LEVM::simulate_tx_from_generic(tx, header, db, *vm_type),
        }
    }

    pub fn create_access_list(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
        fork: Fork,
    ) -> Result<(u64, AccessList, Option<String>), EvmError> {
        let result = match self {
            Evm::REVM { state } => {
                let spec_id = fork_to_spec_id(fork);
                self::revm::helpers::create_access_list(tx, header, state, spec_id)?
            }

            Evm::LEVM { db, vm_type } => {
                LEVM::create_access_list(tx.clone(), header, db, *vm_type)?
            }
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

#[derive(Clone, Debug)]
pub struct BlockExecutionResult {
    pub receipts: Vec<Receipt>,
    pub requests: Vec<Requests>,
}
