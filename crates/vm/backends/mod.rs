pub mod levm;

#[cfg(feature = "revm")]
pub mod revm;
#[cfg(feature = "revm")]
use self::revm::db::{EvmState, evm_state};
#[cfg(feature = "revm")]
use revm::REVM;

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
use std::fmt;
use std::sync::Arc;
use tracing::instrument;

#[derive(Clone)]
pub struct Evm {
    // REVM build
    #[cfg(feature = "revm")]
    pub state: EvmState,

    // non-REVM build
    #[cfg(not(feature = "revm"))]
    pub db: GeneralizedDatabase,

    #[cfg(not(feature = "revm"))]
    pub vm_type: VMType,
}

impl core::fmt::Debug for Evm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // `cfg!` is compile-time; only one branch is kept
        write!(
            f,
            "{}",
            if cfg!(feature = "revm") {
                "REVM"
            } else {
                "LEVM"
            }
        )
    }
}

impl Evm {
    /// Creates a new EVM instance, but with block hash in zero, so if we want to execute a block or transaction we have to set it.
    pub fn new_for_l1(db: impl VmDatabase + 'static) -> Self {
        let wrapped_db: DynVmDatabase = Box::new(db);

        #[cfg(feature = "revm")]
        {
            Evm {
                state: evm_state(wrapped_db),
            }
        }

        #[cfg(not(feature = "revm"))]
        {
            Evm {
                db: GeneralizedDatabase::new(Arc::new(wrapped_db)),
                vm_type: VMType::L1,
            }
        }
    }

    pub fn new_for_l2(db: impl VmDatabase + 'static) -> Result<Self, EvmError> {
        #[cfg(feature = "revm")]
        {
            Err(EvmError::InvalidEVM(
                "REVM is not supported for L2".to_string(),
            ))
        }

        #[cfg(not(feature = "revm"))]
        {
            let wrapped_db: DynVmDatabase = Box::new(db);

            let evm = Evm {
                db: GeneralizedDatabase::new(Arc::new(wrapped_db)),
                vm_type: VMType::L2,
            };

            Ok(evm)
        }
    }

    pub fn new_from_db_for_l1(store: Arc<impl LevmDatabase + 'static>) -> Self {
        Self::_new_from_db(store, VMType::L1)
    }

    pub fn new_from_db_for_l2(store: Arc<impl LevmDatabase + 'static>) -> Self {
        Self::_new_from_db(store, VMType::L2)
    }

    // Only used in non-REVM builds; in REVM builds this constructor is not supported.
    #[cfg(feature = "revm")]
    fn _new_from_db(_store: Arc<impl LevmDatabase + 'static>, _vm_type: VMType) -> Self {
        unreachable!("new_from_db is not supported when built with the `revm` feature")
    }

    #[cfg(not(feature = "revm"))]
    fn _new_from_db(store: Arc<impl LevmDatabase + 'static>, vm_type: VMType) -> Self {
        Evm {
            db: GeneralizedDatabase::new(store),
            vm_type,
        }
    }

    #[instrument(level = "trace", name = "Block execution", skip_all)]
    pub fn execute_block(&mut self, block: &Block) -> Result<BlockExecutionResult, EvmError> {
        #[cfg(feature = "revm")]
        {
            REVM::execute_block(block, &mut self.state)
        }

        #[cfg(not(feature = "revm"))]
        {
            LEVM::execute_block(block, &mut self.db, self.vm_type)
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
        #[cfg(feature = "revm")]
        {
            let chain_config = self.state.chain_config()?;
            let execution_result = REVM::execute_tx(
                tx,
                block_header,
                &mut self.state,
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

        #[cfg(not(feature = "revm"))]
        {
            let execution_report =
                LEVM::execute_tx(tx, sender, block_header, &mut self.db, self.vm_type)?;

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

    pub fn undo_last_tx(&mut self) -> Result<(), EvmError> {
        #[cfg(feature = "revm")]
        {
            Err(EvmError::InvalidEVM(
                "Undoing transaction not supported in REVM".to_string(),
            ))
        }

        #[cfg(not(feature = "revm"))]
        {
            LEVM::undo_last_tx(&mut self.db)
        }
    }

    /// Wraps [REVM::beacon_root_contract_call], [REVM::process_block_hash_history]
    /// and [LEVM::beacon_root_contract_call], [LEVM::process_block_hash_history].
    /// This function is used to run/apply all the system contracts to the state.
    pub fn apply_system_calls(&mut self, block_header: &BlockHeader) -> Result<(), EvmError> {
        #[cfg(feature = "revm")]
        {
            let chain_config = self.state.chain_config()?;
            let spec_id = spec_id(&chain_config, block_header.timestamp);
            if block_header.parent_beacon_block_root.is_some() && spec_id >= SpecId::CANCUN {
                REVM::beacon_root_contract_call(block_header, &mut self.state)?;
            }

            if spec_id >= SpecId::PRAGUE {
                REVM::process_block_hash_history(block_header, &mut self.state)?;
            }

            Ok(())
        }

        #[cfg(not(feature = "revm"))]
        {
            let chain_config = self.db.store.get_chain_config()?;
            let fork = chain_config.fork(block_header.timestamp);

            if block_header.parent_beacon_block_root.is_some() && fork >= Fork::Cancun {
                LEVM::beacon_root_contract_call(block_header, &mut self.db, self.vm_type)?;
            }

            if fork >= Fork::Prague {
                LEVM::process_block_hash_history(block_header, &mut self.db, self.vm_type)?;
            }

            Ok(())
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
        #[cfg(feature = "revm")]
        {
            Ok(REVM::get_state_transitions(&mut self.state))
        }

        #[cfg(not(feature = "revm"))]
        {
            LEVM::get_state_transitions(&mut self.db)
        }
    }

    /// Wraps the [REVM::process_withdrawals] and [LEVM::process_withdrawals].
    /// Applies the withdrawals to the state or the block_chache if using [LEVM].
    pub fn process_withdrawals(&mut self, withdrawals: &[Withdrawal]) -> Result<(), EvmError> {
        #[cfg(feature = "revm")]
        {
            REVM::process_withdrawals(&mut self.state, withdrawals)
        }

        #[cfg(not(feature = "revm"))]
        {
            LEVM::process_withdrawals(&mut self.db, withdrawals)
        }
    }

    pub fn extract_requests(
        &mut self,
        receipts: &[Receipt],
        header: &BlockHeader,
    ) -> Result<Vec<Requests>, EvmError> {
        #[cfg(not(feature = "revm"))]
        {
            levm::extract_all_requests_levm(receipts, &mut self.db, header, self.vm_type)
        }

        #[cfg(feature = "revm")]
        {
            revm::extract_all_requests(receipts, &mut self.state, header)
        }
    }

    pub fn simulate_tx_from_generic(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
        fork: Fork,
    ) -> Result<ExecutionResult, EvmError> {
        #[cfg(feature = "revm")]
        {
            let spec_id = fork_to_spec_id(fork);
            self::revm::helpers::simulate_tx_from_generic(tx, header, &mut self.state, spec_id)
        }

        #[cfg(not(feature = "revm"))]
        {
            LEVM::simulate_tx_from_generic(tx, header, &mut self.db, self.vm_type)
        }
    }

    pub fn create_access_list(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
        fork: Fork,
    ) -> Result<(u64, AccessList, Option<String>), EvmError> {
        #[cfg(feature = "revm")]
        let result = {
            let spec_id = fork_to_spec_id(fork);
            self::revm::helpers::create_access_list(tx, header, &mut self.state, spec_id)?
        };

        #[cfg(not(feature = "revm"))]
        let result = { LEVM::create_access_list(tx.clone(), header, &mut self.db, self.vm_type)? };

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
