pub mod levm;
use levm::LEVM;

use crate::db::{DynVmDatabase, VmDatabase};
use crate::errors::EvmError;
use crate::execution_result::ExecutionResult;
use ethrex_common::types::requests::Requests;
use ethrex_common::types::{
    AccessList, AccountUpdate, Block, BlockHeader, Fork, GenericTransaction, Receipt, Transaction,
    Withdrawal,
};
use ethrex_common::{Address, types::fee_config::FeeConfig};
pub use ethrex_levm::call_frame::CallFrameBackup;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
pub use ethrex_levm::db::{CachingDatabase, Database as LevmDatabase, PrewarmHint};
use ethrex_levm::vm::VMType;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::Sender;
use tracing::instrument;

#[derive(Clone)]
pub struct Evm {
    pub db: GeneralizedDatabase,
    pub vm_type: VMType,
}

impl core::fmt::Debug for Evm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "LEVM",)
    }
}

impl Evm {
    /// Creates a new EVM instance, but with block hash in zero, so if we want to execute a block or transaction we have to set it.
    pub fn new_for_l1(db: impl VmDatabase + 'static) -> Self {
        let wrapped_db: DynVmDatabase = Box::new(db);
        Evm {
            db: GeneralizedDatabase::new(Arc::new(wrapped_db)),
            vm_type: VMType::L1,
        }
    }

    pub fn new_for_l2(
        db: impl VmDatabase + 'static,
        fee_config: FeeConfig,
    ) -> Result<Self, EvmError> {
        let wrapped_db: DynVmDatabase = Box::new(db);

        let evm = Evm {
            db: GeneralizedDatabase::new(Arc::new(wrapped_db)),
            vm_type: VMType::L2(fee_config),
        };

        Ok(evm)
    }

    pub fn new_from_db_for_l1(store: Arc<impl LevmDatabase + 'static>) -> Self {
        Self::_new_from_db(store, VMType::L1)
    }

    pub fn new_from_db_for_l2(
        store: Arc<impl LevmDatabase + 'static>,
        fee_config: FeeConfig,
    ) -> Self {
        Self::_new_from_db(store, VMType::L2(fee_config))
    }

    fn _new_from_db(store: Arc<impl LevmDatabase + 'static>, vm_type: VMType) -> Self {
        Evm {
            db: GeneralizedDatabase::new(store),
            vm_type,
        }
    }

    pub fn execute_block(&mut self, block: &Block) -> Result<BlockExecutionResult, EvmError> {
        LEVM::execute_block(block, &mut self.db, self.vm_type)
    }

    #[instrument(
        level = "trace",
        name = "Block execution",
        skip_all,
        fields(namespace = "block_execution")
    )]
    pub fn execute_block_pipeline(
        &mut self,
        block: &Block,
        merkleizer: Sender<Vec<AccountUpdate>>,
        queue_length: &AtomicUsize,
    ) -> Result<BlockExecutionResult, EvmError> {
        LEVM::execute_block_pipeline(block, &mut self.db, self.vm_type, merkleizer, queue_length)
    }

    /// Wraps [LEVM::execute_tx].
    /// The output is `(Receipt, u64)` == (transaction_receipt, gas_used).
    #[allow(clippy::too_many_arguments)]
    pub fn execute_tx(
        &mut self,
        tx: &Transaction,
        block_header: &BlockHeader,
        remaining_gas: &mut u64,
        sender: Address,
    ) -> Result<(Receipt, u64), EvmError> {
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

    pub fn undo_last_tx(&mut self) -> Result<(), EvmError> {
        LEVM::undo_last_tx(&mut self.db)
    }

    /// Wraps [LEVM::beacon_root_contract_call], [LEVM::process_block_hash_history].
    /// This function is used to run/apply all the system contracts to the state.
    pub fn apply_system_calls(&mut self, block_header: &BlockHeader) -> Result<(), EvmError> {
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

    /// Wraps the [LEVM::get_state_transitions] which gathers the information from a [CacheDB].
    /// The output is `Vec<AccountUpdate>`.
    pub fn get_state_transitions(&mut self) -> Result<Vec<AccountUpdate>, EvmError> {
        LEVM::get_state_transitions(&mut self.db)
    }

    /// Wraps [LEVM::process_withdrawals].
    /// Applies the withdrawals to the state or the block_chache if using [LEVM].
    pub fn process_withdrawals(&mut self, withdrawals: &[Withdrawal]) -> Result<(), EvmError> {
        LEVM::process_withdrawals(&mut self.db, withdrawals)
    }

    pub fn extract_requests(
        &mut self,
        receipts: &[Receipt],
        header: &BlockHeader,
    ) -> Result<Vec<Requests>, EvmError> {
        levm::extract_all_requests_levm(receipts, &mut self.db, header, self.vm_type)
    }

    pub fn simulate_tx_from_generic(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
    ) -> Result<ExecutionResult, EvmError> {
        LEVM::simulate_tx_from_generic(tx, header, &mut self.db, self.vm_type)
    }

    pub fn create_access_list(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
    ) -> Result<(u64, AccessList, Option<String>), EvmError> {
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
    /// Block gas used (PRE-REFUND for Amsterdam+ per EIP-7778).
    /// This differs from receipt cumulative_gas_used which is POST-REFUND.
    pub block_gas_used: u64,
}
