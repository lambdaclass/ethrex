use ethrex_common::types::{Block, Transaction};
use ethrex_common::{tracing::CallTrace, types::BlockHeader};
use ethrex_levm::vm::VMType;
use ethrex_levm::{db::gen_db::GeneralizedDatabase, tracing::LevmCallTracer, vm::VM};

use crate::Evm;
use crate::{EvmError, backends::levm::LEVM};

impl LEVM {
    /// Execute all transactions of the block up until a certain transaction specified in `stop_index`.
    /// The goal is to just mutate the state up to that point, without needing to process transaction receipts or requests.
    pub fn rerun_block(
        &mut self,
        block: &Block,
        stop_index: Option<usize>,
    ) -> Result<(), EvmError> {
        self.prepare_block(block)?;

        // Executes transactions and stops when the index matches the stop index.
        for (index, (tx, sender)) in block
            .body
            .get_transactions_with_sender()
            .map_err(|error| EvmError::Transaction(error.to_string()))?
            .into_iter()
            .enumerate()
        {
            if stop_index.is_some_and(|stop| stop == index) {
                break;
            }

            self.execute_tx_aux(tx, sender, &block.header)?;
        }

        // Process withdrawals only if the whole block has been executed.
        if stop_index.is_none() {
            if let Some(withdrawals) = &block.body.withdrawals {
                self.process_withdrawals(withdrawals)?;
            }
        };

        Ok(())
    }

    /// Run transaction with callTracer activated.
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

        let env = self.setup_env(
            tx,
            tx.sender().map_err(|error| {
                EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
            })?,
            &block.header,
        )?;
        let mut vm = VM::new(
            env,
            &mut self.db,
            tx,
            LevmCallTracer::new(only_top_call, with_log),
            self.vm_type,
        )?;

        vm.execute()?;

        let callframe = vm.get_trace_result()?;

        // We only return the top call because a transaction only has one call with subcalls
        Ok(vec![callframe])
    }
}
