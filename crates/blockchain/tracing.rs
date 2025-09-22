use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use ethrex_common::{H256, tracing::CallTrace, types::Block};
use ethrex_storage::Store;
use ethrex_vm::{Evm, EvmError};

use crate::{Blockchain, error::ChainError, vm::StoreVmDatabase};

impl Blockchain {
    /// Outputs the call trace for the given transaction
    /// May need to re-execute blocks in order to rebuild the transaction's prestate, up to the amount given by `reexec`
    pub async fn trace_transaction_calls(
        &self,
        tx_hash: H256,
        reexec: u32,
        timeout: Duration,
        only_top_call: bool,
        with_log: bool,
    ) -> Result<CallTrace, ChainError> {
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
        // Obtain the block's parent state
        let mut vm = self
            .rebuild_parent_state(block.header.parent_hash, reexec)
            .await?;
        let fee_vault = self.fee_vault;
        // Run the block until the transaction we want to trace
        vm.rerun_block(&block, Some(tx_index), fee_vault)?;

        // Trace the transaction
        timeout_trace_operation(timeout, move || {
            vm.trace_tx_calls(&block, tx_index, only_top_call, with_log, fee_vault)
        })
        .await
    }

    /// Outputs the call trace for each transaction in the block along with the transaction's hash
    /// May need to re-execute blocks in order to rebuild the transaction's prestate, up to the amount given by `reexec`
    /// Returns transaction call traces from oldest to newest
    pub async fn trace_block_calls(
        &self,
        // We receive the block instead of its hash/number to support multiple potential endpoints
        block: Block,
        reexec: u32,
        timeout: Duration,
        only_top_call: bool,
        with_log: bool,
    ) -> Result<Vec<(H256, CallTrace)>, ChainError> {
        // Obtain the block's parent state
        let mut vm = self
            .rebuild_parent_state(block.header.parent_hash, reexec)
            .await?;
        let fee_vault = self.fee_vault;
        // Run anything necessary before executing the block's transactions (system calls, etc)
        vm.rerun_block(&block, Some(0), fee_vault)?;
        // Trace each transaction
        // We need to do this in order to pass ownership of block & evm to a blocking process without cloning
        let vm = Arc::new(Mutex::new(vm));
        let block = Arc::new(block);
        let mut call_traces = vec![];
        for index in 0..block.body.transactions.len() {
            // We are cloning the `Arc`s here, not the structs themselves
            let block = block.clone();
            let vm = vm.clone();
            let tx_hash = block.as_ref().body.transactions[index].hash();
            let call_trace = timeout_trace_operation(timeout, move || {
                vm.lock()
                    .map_err(|_| EvmError::Custom("Unexpected Runtime Error".to_string()))?
                    .trace_tx_calls(block.as_ref(), index, only_top_call, with_log, fee_vault)
            })
            .await?;
            call_traces.push((tx_hash, call_trace));
        }
        Ok(call_traces)
    }

    /// Rebuild the parent state for a block given its parent hash, returning an `Evm` instance with all changes cached
    /// Will re-execute all ancestor block's which's state is not stored up to a maximum given by `reexec`
    async fn rebuild_parent_state(
        &self,
        parent_hash: H256,
        reexec: u32,
    ) -> Result<Evm, ChainError> {
        // Check if we need to re-execute parent blocks
        let blocks_to_re_execute =
            get_missing_state_parents(parent_hash, &self.storage, reexec).await?;
        // Base our Evm's state on the newest parent block which's state we have available
        let parent_hash = blocks_to_re_execute
            .last()
            .map(|b| b.header.parent_hash)
            .unwrap_or(parent_hash);
        // Cache block hashes for all parent blocks so we can access them during execution
        let block_hash_cache = blocks_to_re_execute
            .iter()
            .map(|b| (b.header.number, b.hash()))
            .collect();
        let vm_db = StoreVmDatabase::new_with_block_hash_cache(
            self.storage.clone(),
            parent_hash,
            block_hash_cache,
        );
        let mut vm = self.new_evm(vm_db)?;
        // Run parents to rebuild pre-state
        for block in blocks_to_re_execute.iter().rev() {
            vm.rerun_block(block, None, self.fee_vault)?;
        }
        Ok(vm)
    }
}

/// Returns a list of all the parent blocks (starting from parent hash) who's state we don't have stored.
/// The list will be sorted from newer to older
/// We might be missing this state due to using batch execute or other methods while syncing the chain
/// If we are not able to find a parent block with state after going through the amount of blocks given by `reexec` an error will be returned
async fn get_missing_state_parents(
    mut parent_hash: H256,
    store: &Store,
    reexec: u32,
) -> Result<Vec<Block>, ChainError> {
    let mut missing_state_parents = Vec::new();
    loop {
        if missing_state_parents.len() > reexec as usize {
            return Err(ChainError::Custom(
                "Exceeded max amount of blocks to re-execute for tracing".to_string(),
            ));
        }
        let Some(parent_block) = store.get_block_by_hash(parent_hash).await? else {
            return Err(ChainError::Custom("Parent Block not Found".to_string()));
        };
        if store.contains_state_node(parent_block.header.state_root)? {
            break;
        }
        parent_hash = parent_block.header.parent_hash;
        // Add parent to re-execute list
        missing_state_parents.push(parent_block);
    }
    Ok(missing_state_parents)
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
