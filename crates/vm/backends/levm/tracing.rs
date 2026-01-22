use ethrex_common::types::{Block, Transaction};
use ethrex_common::{tracing::CallTrace, types::BlockHeader, U256};
use ethrex_levm::vm::VMType;
use ethrex_levm::{db::gen_db::GeneralizedDatabase, tracing::LevmCallTracer, vm::VM, EVMConfig};
use ethrex_levm::utils::get_base_fee_per_blob_gas;

use crate::{EvmError, backends::levm::LEVM};

impl LEVM {
    /// Execute all transactions of the block up until a certain transaction specified in `stop_index`.
    /// The goal is to just mutate the state up to that point, without needing to process transaction receipts or requests.
    pub fn rerun_block(
        db: &mut GeneralizedDatabase,
        block: &Block,
        stop_index: Option<usize>,
        vm_type: VMType,
    ) -> Result<(), EvmError> {
        Self::prepare_block(block, db, vm_type)?;

        // Compute base blob fee once for the entire block
        let chain_config = db.store.get_chain_config()?;
        let config = EVMConfig::new_from_chain_config(&chain_config, &block.header);
        let block_excess_blob_gas = block.header.excess_blob_gas.map(U256::from);
        let base_blob_fee_per_gas = get_base_fee_per_blob_gas(block_excess_blob_gas, &config)?;

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

            Self::execute_tx(tx, sender, &block.header, db, vm_type, base_blob_fee_per_gas)?;
        }

        // Process withdrawals only if the whole block has been executed.
        if stop_index.is_none()
            && let Some(withdrawals) = &block.body.withdrawals
        {
            Self::process_withdrawals(db, withdrawals)?;
        };

        Ok(())
    }

    /// Run transaction with callTracer activated.
    pub fn trace_tx_calls(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &Transaction,
        only_top_call: bool,
        with_log: bool,
        vm_type: VMType,
    ) -> Result<CallTrace, EvmError> {
        // Compute base blob fee per gas
        let chain_config = db.store.get_chain_config()?;
        let config = EVMConfig::new_from_chain_config(&chain_config, block_header);
        let block_excess_blob_gas = block_header.excess_blob_gas.map(U256::from);
        let base_blob_fee_per_gas = get_base_fee_per_blob_gas(block_excess_blob_gas, &config)?;

        let env = Self::setup_env(
            tx,
            tx.sender().map_err(|error| {
                EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
            })?,
            block_header,
            db,
            vm_type,
            base_blob_fee_per_gas,
        )?;
        let mut vm = VM::new(
            env,
            db,
            tx,
            LevmCallTracer::new(only_top_call, with_log),
            vm_type,
        )?;

        vm.execute()?;

        let callframe = vm.get_trace_result()?;

        // We only return the top call because a transaction only has one call with subcalls
        Ok(vec![callframe])
    }
}
