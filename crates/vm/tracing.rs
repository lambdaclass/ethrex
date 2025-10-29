use ethrex_common::tracing::CallTrace;
use ethrex_common::types::Block;

#[cfg(not(feature = "revm"))]
use crate::backends::levm::LEVM;
#[cfg(feature = "revm")]
use crate::backends::revm::REVM;

use crate::{Evm, EvmError};

impl Evm {
    /// Runs a single tx with the call tracer and outputs its trace.
    /// Assumes that the received state already contains changes from previous blocks and other
    /// transactions within its block.
    /// Wraps REVM::trace_tx_calls or LEVM::trace_tx_calls depending on the feature.
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

        #[cfg(feature = "revm")]
        {
            REVM::trace_tx_calls(&block.header, tx, &mut self.state, only_top_call, with_log)
        }

        #[cfg(not(feature = "revm"))]
        {
            LEVM::trace_tx_calls(
                &mut self.db,
                &block.header,
                tx,
                only_top_call,
                with_log,
                self.vm_type,
            )
        }
    }

    /// Reruns the given block, saving the changes on the state, doesn't output any results or receipts.
    /// If the optional argument `stop_index` is set, the run will stop just before executing the transaction at that index
    /// and won't process the withdrawals afterwards.
    /// Wraps REVM::rerun_block or LEVM::rerun_block depending on the feature.
    pub fn rerun_block(
        &mut self,
        block: &Block,
        stop_index: Option<usize>,
    ) -> Result<(), EvmError> {
        #[cfg(feature = "revm")]
        {
            REVM::rerun_block(block, &mut self.state, stop_index)
        }

        #[cfg(not(feature = "revm"))]
        {
            LEVM::rerun_block(&mut self.db, block, stop_index, self.vm_type)
        }
    }
}
