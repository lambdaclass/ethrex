use ethrex_common::tracing::CallTrace;
use ethrex_common::types::Block;
use ethrex_levm::{db::gen_db::GeneralizedDatabase, tracing::LevmCallTracer, vm::VM};

use crate::{backends::levm::LEVM, EvmError};

impl LEVM {
    /// Execute all transactions of the block up until a certain transaction specified in `stop_index`.
    /// The goal is to just mutate the state up to that point, without needing to process transaction receipts or requests.
    pub fn rerun_block(
        db: &mut GeneralizedDatabase,
        block: &Block,
        stop_index: Option<usize>,
    ) -> Result<(), EvmError> {
        Self::prepare_block(block, db)?;

        // Executes transactions and stops when the index matches the stop index.
        for (index, (tx, sender)) in block
            .body
            .get_transactions_with_sender()
            .into_iter()
            .enumerate()
        {
            if stop_index.is_some_and(|stop| stop == index) {
                break;
            }

            Self::execute_tx(tx, sender, &block.header, db).map_err(EvmError::from)?;
        }

        // Process withdrawals only if the whole block has been executed.
        if stop_index.is_none() {
            if let Some(withdrawals) = &block.body.withdrawals {
                Self::process_withdrawals(db, withdrawals)?;
            }
        };

        Ok(())
    }

    /// Run transaction with callTracer activated.
    pub fn trace_tx_calls(
        db: &mut GeneralizedDatabase,
        block: &Block,
        tx_index: usize,
        only_top_call: bool,
        with_log: bool,
    ) -> Result<CallTrace, EvmError> {
        //TODO: Just send transaction instead of index (?)
        let tx = block
            .body
            .transactions
            .get(tx_index)
            .ok_or(EvmError::Custom(
                "Missing Transaction for Trace".to_string(),
            ))?;

        let env = Self::setup_env(tx, tx.sender(), &block.header, db)?;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::new(only_top_call, with_log));

        vm.execute()?;

        let callframe = vm
            .tracer
            .callframes
            .pop()
            .ok_or(EvmError::Custom("Could not get trace".to_string()))?;

        // We only return the top call because a transaction only has one call with subcalls...
        Ok(vec![callframe])
    }
}
