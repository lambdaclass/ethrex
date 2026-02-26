use crate::backends::levm::LEVM;
use ethrex_common::tracing::CallTrace;
use ethrex_common::types::{Block, BlockHeader, Transaction};
use ethrex_levm::environment::Environment;

use crate::{Evm, EvmError};

impl Evm {
    /// Build the execution environment for a transaction.
    /// Useful for replaying transactions outside the standard execution path.
    pub fn setup_env_for_tx(
        &self,
        tx: &Transaction,
        block_header: &BlockHeader,
    ) -> Result<Environment, EvmError> {
        let sender = tx
            .sender()
            .map_err(|e| EvmError::Transaction(e.to_string()))?;
        LEVM::setup_env(tx, sender, block_header, &self.db, self.vm_type)
    }

    /// Runs a single tx with the call tracer and outputs its trace.
    /// Assumes that the received state already contains changes from previous blocks and other
    /// transactions within its block.
    /// Wraps LEVM::trace_tx_calls depending on the feature.
    pub fn trace_tx_calls(
        &mut self,
        block: &Block,
        tx_index: usize,
        only_top_call: bool,
        with_log: bool,
    ) -> Result<CallTrace, EvmError> {
        let tx = block
            .body
            .transactions
            .get(tx_index)
            .ok_or(EvmError::Custom(
                "Missing Transaction for Trace".to_string(),
            ))?;

        LEVM::trace_tx_calls(
            &mut self.db,
            &block.header,
            tx,
            only_top_call,
            with_log,
            self.vm_type,
        )
    }

    /// Reruns the given block, saving the changes on the state, doesn't output any results or receipts.
    /// If the optional argument `stop_index` is set, the run will stop just before executing the transaction at that index
    /// and won't process the withdrawals afterwards.
    /// WrapsLEVM::rerun_block depending on the feature.
    pub fn rerun_block(
        &mut self,
        block: &Block,
        stop_index: Option<usize>,
    ) -> Result<(), EvmError> {
        LEVM::rerun_block(&mut self.db, block, stop_index, self.vm_type)
    }
}
