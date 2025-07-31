use ethrex_common::tracing::CallTrace;
use ethrex_common::types::Block;

use crate::backends::levm::LEVM;
use crate::{Evm, EvmError, backends::revm::REVM};

impl Evm {
    /// Runs a single tx with the call tracer and outputs its trace
    /// Asumes that the received state already contains changes from previous blocks and other
    /// transactions within its block
    /// Wraps [REVM::trace_tx_calls], does not currenlty have levm support.
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

        match self {
            Evm::REVM { state } => {
                REVM::trace_tx_calls(&block.header, tx, state, only_top_call, with_log)
            }
            Evm::LEVM { db, vm_type } => {
                LEVM::trace_tx_calls(db, &block.header, tx, only_top_call, with_log, *vm_type)
            }
        }
    }

    /// Reruns the given block, saving the changes on the state, doesn't output any results or receipts
    /// If the optional argument `stop_index` is set, the run will stop just before executing the transaction at that index
    /// and won't process the withdrawals afterwards
    /// Wraps [REVM::rerun_block], does not currenlty have levm support.
    pub fn rerun_block(
        &mut self,
        block: &Block,
        stop_index: Option<usize>,
    ) -> Result<(), EvmError> {
        match self {
            Evm::REVM { state } => REVM::rerun_block(block, state, stop_index),
            Evm::LEVM { db, vm_type } => LEVM::rerun_block(db, block, stop_index, *vm_type),
        }
    }
}
