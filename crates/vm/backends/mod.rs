pub mod levm;
use levm::LEVM;

use crate::db::{DynVmDatabase, VmDatabase};
use crate::errors::EvmError;
use crate::execution_result::ExecutionResult;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::requests::Requests;
use ethrex_common::types::{
    AccessList, AccountUpdate, Block, BlockHeader, Fork, GenericTransaction, Receipt, Transaction,
    Withdrawal,
};
use ethrex_common::{Address, types::fee_config::FeeConfig};
use ethrex_crypto::Crypto;
pub use ethrex_levm::call_frame::CallFrameBackup;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
pub use ethrex_levm::db::{CachingDatabase, Database as LevmDatabase};
use ethrex_levm::errors::{ExecutionReport, TxResult};
use ethrex_levm::vm::VMType;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::Sender;
use tracing::instrument;

#[derive(Clone)]
pub struct Evm {
    pub db: GeneralizedDatabase,
    pub vm_type: VMType,
    pub crypto: Arc<dyn Crypto>,
}

impl core::fmt::Debug for Evm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "LEVM",)
    }
}

impl Evm {
    /// Creates a new EVM instance, but with block hash in zero, so if we want to execute a block or transaction we have to set it.
    pub fn new_for_l1(db: impl VmDatabase + 'static, crypto: Arc<dyn Crypto>) -> Self {
        let wrapped_db: DynVmDatabase = Box::new(db);
        Evm {
            db: GeneralizedDatabase::new(Arc::new(wrapped_db)),
            vm_type: VMType::L1,
            crypto,
        }
    }

    pub fn new_for_l2(
        db: impl VmDatabase + 'static,
        fee_config: FeeConfig,
        crypto: Arc<dyn Crypto>,
    ) -> Result<Self, EvmError> {
        let wrapped_db: DynVmDatabase = Box::new(db);

        let evm = Evm {
            db: GeneralizedDatabase::new(Arc::new(wrapped_db)),
            vm_type: VMType::L2(fee_config),
            crypto,
        };

        Ok(evm)
    }

    pub fn new_from_db_for_l1(
        store: Arc<impl LevmDatabase + 'static>,
        crypto: Arc<dyn Crypto>,
    ) -> Self {
        Self::_new_from_db(store, VMType::L1, crypto)
    }

    pub fn new_from_db_for_l2(
        store: Arc<impl LevmDatabase + 'static>,
        fee_config: FeeConfig,
        crypto: Arc<dyn Crypto>,
    ) -> Self {
        Self::_new_from_db(store, VMType::L2(fee_config), crypto)
    }

    fn _new_from_db(
        store: Arc<impl LevmDatabase + 'static>,
        vm_type: VMType,
        crypto: Arc<dyn Crypto>,
    ) -> Self {
        Evm {
            db: GeneralizedDatabase::new(store),
            vm_type,
            crypto,
        }
    }

    /// Execute a block and return the execution result.
    ///
    /// Also records and returns the Block Access List (EIP-7928) for Amsterdam+ forks.
    /// The BAL will be `None` for pre-Amsterdam forks.
    pub fn execute_block(
        &mut self,
        block: &Block,
    ) -> Result<(BlockExecutionResult, Option<BlockAccessList>), EvmError> {
        LEVM::execute_block(block, &mut self.db, self.vm_type, self.crypto.as_ref())
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
        merkleizer: Option<Sender<Vec<AccountUpdate>>>,
        queue_length: &AtomicUsize,
        bal: Option<&BlockAccessList>,
        bal_parallel_exec_enabled: bool,
    ) -> Result<(BlockExecutionResult, Option<BlockAccessList>), EvmError> {
        LEVM::execute_block_pipeline(
            block,
            &mut self.db,
            self.vm_type,
            merkleizer,
            queue_length,
            self.crypto.as_ref(),
            bal,
            bal_parallel_exec_enabled,
        )
    }

    /// Wraps [LEVM::execute_tx].
    /// Updates `remaining_gas` (pre-refund) for block gas accounting and
    /// `cumulative_gas_spent` (post-refund) for receipt cumulative tracking.
    /// Returns (Receipt, gas_spent) where gas_spent is post-refund for block value calculation.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_tx(
        &mut self,
        tx: &Transaction,
        block_header: &BlockHeader,
        cumulative_gas_spent: &mut u64,
        sender: Address,
    ) -> Result<(Receipt, ExecutionReport), EvmError> {
        let execution_report = LEVM::execute_tx(
            tx,
            sender,
            block_header,
            &mut self.db,
            self.vm_type,
            self.crypto.as_ref(),
        )?;

        // Track cumulative post-refund gas for receipt
        *cumulative_gas_spent += execution_report.gas_spent;

        let receipt = Receipt::new(
            tx.tx_type(),
            execution_report.is_success(),
            *cumulative_gas_spent,
            execution_report.logs.clone(),
        );

        Ok((receipt, execution_report))
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
            LEVM::beacon_root_contract_call(
                block_header,
                &mut self.db,
                self.vm_type,
                self.crypto.as_ref(),
            )?;
        }

        if fork >= Fork::Prague {
            LEVM::process_block_hash_history(
                block_header,
                &mut self.db,
                self.vm_type,
                self.crypto.as_ref(),
            )?;
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
        levm::extract_all_requests_levm(
            receipts,
            &mut self.db,
            header,
            self.vm_type,
            self.crypto.as_ref(),
        )
    }

    /// Takes the Block Access List (BAL) from the database if recording was enabled.
    /// Returns `None` if BAL recording was not enabled.
    pub fn take_bal(&mut self) -> Option<BlockAccessList> {
        self.db.take_bal()
    }

    /// Enables BAL (Block Access List) recording for EIP-7928.
    pub fn enable_bal_recording(&mut self) {
        self.db.enable_bal_recording();
    }

    /// Sets the current block access index for BAL recording per EIP-7928 spec (uint32).
    pub fn set_bal_index(&mut self, index: u32) {
        self.db.set_bal_index(index);
    }

    pub fn simulate_tx_from_generic(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
    ) -> Result<ExecutionResult, EvmError> {
        LEVM::simulate_tx_from_generic(tx, header, &mut self.db, self.vm_type, self.crypto.as_ref())
    }

    pub fn create_access_list(
        &mut self,
        tx: &GenericTransaction,
        header: &BlockHeader,
    ) -> Result<(u64, AccessList, Option<String>), EvmError> {
        let result = {
            LEVM::create_access_list(
                tx.clone(),
                header,
                &mut self.db,
                self.vm_type,
                self.crypto.as_ref(),
            )?
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
    /// Block gas used (PRE-REFUND for Amsterdam+ per EIP-7778).
    /// This differs from receipt cumulative_gas_used which is POST-REFUND.
    pub block_gas_used: u64,
    /// Per-tx gas-dimension breakdown. Populated by `execute_block`; left empty by
    /// L2 producer / committer paths that build a `BlockExecutionResult` from
    /// re-derived data. Used by `validate_gas_used` mismatch logging to localize
    /// which tx and which dimension caused the divergence.
    pub tx_gas_breakdowns: Vec<TxGasBreakdown>,
}

/// Per-tx gas-dimension snapshot captured at the block-execution boundary.
/// All fields are pre-refund except `gas_spent` and `gas_refunded` which are
/// the user-pays (post-refund) values.
#[derive(Clone, Debug)]
pub struct TxGasBreakdown {
    pub tx_index: usize,
    pub tx_hash: ethrex_common::H256,
    pub status: TxStatus,
    /// Pre-refund gas used (block-level dimension under EIP-7778).
    pub gas_used: u64,
    /// Post-refund gas paid by the sender.
    pub gas_spent: u64,
    pub gas_refunded: u64,
    /// EIP-8037 state-gas portion of `gas_used` (Amsterdam+); 0 pre-Amsterdam.
    pub state_gas_used: u64,
    /// `gas_used - state_gas_used`. Saturating to avoid underflow on edge cases.
    pub regular_gas_used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxStatus {
    Success,
    Revert,
    Halt,
}

impl core::fmt::Display for TxStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TxStatus::Success => f.write_str("success"),
            TxStatus::Revert => f.write_str("revert"),
            TxStatus::Halt => f.write_str("halt"),
        }
    }
}

impl TxGasBreakdown {
    pub fn from_report(
        tx_index: usize,
        tx_hash: ethrex_common::H256,
        report: &ExecutionReport,
    ) -> Self {
        let status = match &report.result {
            TxResult::Success => TxStatus::Success,
            TxResult::Revert(err) if err.is_revert_opcode() => TxStatus::Revert,
            TxResult::Revert(_) => TxStatus::Halt,
        };
        Self {
            tx_index,
            tx_hash,
            status,
            gas_used: report.gas_used,
            gas_spent: report.gas_spent,
            gas_refunded: report.gas_refunded,
            state_gas_used: report.state_gas_used,
            regular_gas_used: report.gas_used.saturating_sub(report.state_gas_used),
        }
    }
}

/// Emit a structured per-tx gas-dimension dump. Called from the block-validation
/// site when `block_gas_used` disagrees with `header.gas_used`. If `breakdowns` is
/// empty (paths that don't populate it, e.g. L2 producer), a one-liner is logged.
pub fn log_gas_used_mismatch(
    breakdowns: &[TxGasBreakdown],
    block_number: u64,
    actual: u64,
    expected: u64,
) {
    let delta = actual as i128 - expected as i128;
    if breakdowns.is_empty() {
        ::tracing::error!(
            block = block_number,
            actual,
            expected,
            delta,
            "block gas_used mismatch (no per-tx breakdown available on this path)",
        );
        return;
    }
    let sum_regular: u64 = breakdowns.iter().map(|b| b.regular_gas_used).sum();
    let sum_state: u64 = breakdowns.iter().map(|b| b.state_gas_used).sum();
    let sum_refunded: u64 = breakdowns.iter().map(|b| b.gas_refunded).sum();
    ::tracing::error!(
        block = block_number,
        actual,
        expected,
        delta,
        n_txs = breakdowns.len(),
        sum_regular,
        sum_state,
        max_dim = sum_regular.max(sum_state),
        sum_refunded,
        "block gas_used mismatch",
    );
    for b in breakdowns {
        ::tracing::error!(
            tx_idx = b.tx_index,
            tx_hash = %b.tx_hash,
            status = %b.status,
            gas_used = b.gas_used,
            regular = b.regular_gas_used,
            state = b.state_gas_used,
            gas_spent = b.gas_spent,
            gas_refunded = b.gas_refunded,
            "  tx breakdown",
        );
    }
}
