use std::time::Duration;

use ethrex_common::{types::Block, H256};
use ethrex_storage::Store;
use ethrex_vm::{tracing::CallTrace, Evm, EvmEngine, EvmError};

use crate::{error::ChainError, vm::StoreVmDatabase, Blockchain};

impl Blockchain {
    /// Outputs the call trace for the given transaction
    /// May need to re-execute blocks in order to rebuild the transaction's prestate, up to the amount given by `reexec`
    pub async fn trace_transaction_calls(
        &self,
        tx_hash: H256,
        reexec: usize,
        timeout: Duration,
        only_top_call: bool,
        with_log: bool,
    ) -> Result<CallTrace, ChainError> {
        if matches!(self.evm_engine, EvmEngine::LEVM) {
            return Err(ChainError::Custom(
                "Tracing not supported on LEVM".to_string(),
            ));
        }
        // Fetch the transaction's location and the block it is contained in
        let Some((_, block_hash, tx_index)) =
            self.storage.get_transaction_location(tx_hash).await?
        else {
            return Err(ChainError::Custom("Transaction not Found".to_string()));
        };
        let tx_index = tx_index as usize;
        let Some(block) = self.storage.get_block_by_hash(block_hash).await? else {
            return Err(ChainError::Custom("Block not Found".to_string()));
        };
        // Check if we need to re-execute parent blocks
        let mut blocks_to_re_execute = Vec::new();
        fill_missing_state_parents(
            block.header.parent_hash,
            &mut blocks_to_re_execute,
            &self.storage,
            reexec,
        )
        .await?;
        let parent_hash = blocks_to_re_execute
            .last()
            .unwrap_or(&block)
            .header
            .parent_hash;
        // Run parents to rebuild pre-state
        let mut vm = Evm::new(
            self.evm_engine,
            StoreVmDatabase::new(self.storage.clone(), parent_hash),
        );
        for block in blocks_to_re_execute.iter().rev() {
            vm.rerun_block(block, None)?;
        }
        // Run the block until the transaction we want to trace
        vm.rerun_block(&block, Some(tx_index))?;
        // Trace the transaction
        timeout_trace_operation(timeout, move || {
            vm.trace_tx_calls(&block, tx_index, only_top_call, with_log)
        })
        .await
    }
}

/// Fills the `missing_state_parents` vector with all the parent blocks (starting from parent hash) who's state we don't have stored.
/// We might be missing this state due to using batch execute or other methods while syncing the chain
/// If we are not able to find a parent block with state after going through the amount of blocks given by `reexec` an error will be returned
async fn fill_missing_state_parents(
    mut parent_hash: H256,
    missing_state_parents: &mut Vec<Block>,
    store: &Store,
    reexec: usize,
) -> Result<(), ChainError> {
    loop {
        if missing_state_parents.len() > reexec {
            return Err(ChainError::Custom(
                "Exceeded max amount of blocks to re-execute for tracing".to_string(),
            ));
        }
        let Some(parent_block) = store.get_block_by_hash(parent_hash).await? else {
            return Err(ChainError::Custom("Parent Block not Found".to_string()));
        };
        if store.contains_state_node(parent_block.header.state_root)? {
            return Ok(());
        }
        parent_hash = parent_block.header.parent_hash;
        // Add parent to re-execute list
        missing_state_parents.push(parent_block);
    }
}

/// Runs the given evm trace operation, aborting if it takes more than the time given by `tiemout`
async fn timeout_trace_operation<O, T>(timeout: Duration, operation: O) -> Result<T, ChainError>
where
    O: FnOnce() -> Result<T, EvmError> + Send + 'static,
    T: Send + 'static,
{
    Ok(
        tokio::time::timeout(timeout, tokio::task::spawn_blocking(operation))
            .await
            .map_err(|_| ChainError::Custom("Tracing Timeout".to_string()))?
            .map_err(|_| ChainError::Custom("Unexpected Runtime Error".to_string()))??,
    )
}
