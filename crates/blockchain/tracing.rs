use std::time::Duration;

use ethrex_common::{
    H256,
    tracing::{CallTrace, OpcodeTraceResult, PrestateResult},
    types::{Block, GenericTransaction},
};
use ethrex_storage::Store;
use ethrex_vm::tracing::OpcodeTracerConfig;
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
        // Run the block until the transaction we want to trace
        vm.rerun_block(&block, Some(tx_index))?;
        // Block-absolute log index base: logs emitted by the preceding txs (geth's `logSize`).
        let log_index_base = self.log_index_base(block_hash, tx_index, with_log).await?;
        // Trace the transaction
        timeout_trace_operation(timeout, move || {
            vm.trace_tx_calls(&block, tx_index, only_top_call, with_log, log_index_base)
        })
        .await
    }

    /// Number of logs emitted by the first `tx_count` txs of `block_hash` — the
    /// block-absolute log index base for tracing the tx at that offset (geth's cumulative
    /// `logSize`). Returns 0 when logs aren't collected or there is no preceding tx, so the
    /// receipt lookup is skipped on the common `withLog: false` path.
    async fn log_index_base(
        &self,
        block_hash: H256,
        tx_count: usize,
        with_log: bool,
    ) -> Result<u64, ChainError> {
        if !with_log || tx_count == 0 {
            return Ok(0);
        }
        let receipts = self
            .storage
            .get_receipts_for_block_from_index(&block_hash, 0, Some(tx_count))
            .await?;
        let total = receipts
            .iter()
            .map(|r| u64::try_from(r.logs.len()).unwrap_or(u64::MAX))
            .fold(0u64, u64::saturating_add);
        Ok(total)
    }

    /// Outputs the call trace for each transaction in the block along with the transaction's hash.
    /// The whole block is traced in a single blocking pass (system calls then every tx in order),
    /// so `timeout` bounds the entire block trace rather than each transaction.
    /// May need to re-execute blocks in order to rebuild the block's prestate, up to the amount given by `reexec`.
    /// Returns transaction call traces from oldest to newest.
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
        timeout_trace_operation(timeout, move || {
            vm.trace_block_calls(&block, only_top_call, with_log)
        })
        .await
    }

    /// Outputs the prestate trace for the given transaction.
    /// If `diff_mode` is true, returns both pre and post state; otherwise returns only pre state.
    /// `include_empty` keeps default-state entries in pre (only valid when `diff_mode` is false).
    /// May need to re-execute blocks in order to rebuild the transaction's prestate, up to the amount given by `reexec`.
    pub async fn trace_transaction_prestate(
        &self,
        tx_hash: H256,
        reexec: u32,
        timeout: Duration,
        diff_mode: bool,
        include_empty: bool,
    ) -> Result<PrestateResult, ChainError> {
        let Some((_, block_hash, tx_index)) =
            self.storage.get_transaction_location(tx_hash).await?
        else {
            return Err(ChainError::Custom("Transaction not Found".to_string()));
        };
        let tx_index = tx_index as usize;
        let Some(block) = self.storage.get_block_by_hash(block_hash).await? else {
            return Err(ChainError::Custom("Block not Found".to_string()));
        };
        let mut vm = self
            .rebuild_parent_state(block.header.parent_hash, reexec)
            .await?;
        // Run the block until the transaction we want to trace
        vm.rerun_block(&block, Some(tx_index))?;
        // Trace the transaction
        timeout_trace_operation(timeout, move || {
            vm.trace_tx_prestate(&block, tx_index, diff_mode, include_empty)
        })
        .await
    }

    /// Outputs the prestate trace for each transaction in the block along with the transaction's hash.
    /// If `diff_mode` is true, returns both pre and post state per tx; otherwise returns only pre state.
    /// `include_empty` keeps default-state entries in pre (only valid when `diff_mode` is false).
    /// The whole block is traced in a single blocking pass, so `timeout` bounds the entire trace.
    /// May need to re-execute blocks in order to rebuild the block's prestate, up to the amount given by `reexec`.
    /// Returns prestate traces from oldest to newest transaction.
    pub async fn trace_block_prestate(
        &self,
        block: Block,
        reexec: u32,
        timeout: Duration,
        diff_mode: bool,
        include_empty: bool,
    ) -> Result<Vec<(H256, PrestateResult)>, ChainError> {
        let mut vm = self
            .rebuild_parent_state(block.header.parent_hash, reexec)
            .await?;
        timeout_trace_operation(timeout, move || {
            vm.trace_block_prestate(&block, diff_mode, include_empty)
        })
        .await
    }

    /// Outputs the per-opcode (EIP-3155) trace for the given transaction.
    /// May need to re-execute blocks in order to rebuild the transaction's prestate, up to the amount given by `reexec`.
    pub async fn trace_transaction_opcodes(
        &self,
        tx_hash: H256,
        reexec: u32,
        timeout: Duration,
        cfg: OpcodeTracerConfig,
    ) -> Result<OpcodeTraceResult, ChainError> {
        let Some((_, block_hash, tx_index)) =
            self.storage.get_transaction_location(tx_hash).await?
        else {
            return Err(ChainError::Custom("Transaction not Found".to_string()));
        };
        let tx_index = tx_index as usize;
        let Some(block) = self.storage.get_block_by_hash(block_hash).await? else {
            return Err(ChainError::Custom("Block not Found".to_string()));
        };
        let mut vm = self
            .rebuild_parent_state(block.header.parent_hash, reexec)
            .await?;
        vm.rerun_block(&block, Some(tx_index))?;
        timeout_trace_operation(timeout, move || vm.trace_tx_opcodes(&block, tx_index, cfg)).await
    }

    /// Outputs the opcode (EIP-3155) trace for each transaction in the block along with
    /// the transaction's hash.
    /// The whole block is traced in a single blocking pass, so `timeout` bounds the entire trace.
    /// May need to re-execute blocks in order to rebuild the block's prestate, up to the amount
    /// given by `reexec`.
    /// Returns traces from oldest to newest transaction.
    pub async fn trace_block_opcodes(
        &self,
        block: Block,
        reexec: u32,
        timeout: Duration,
        cfg: OpcodeTracerConfig,
    ) -> Result<Vec<(H256, OpcodeTraceResult)>, ChainError> {
        let mut vm = self
            .rebuild_parent_state(block.header.parent_hash, reexec)
            .await?;
        timeout_trace_operation(timeout, move || vm.trace_block_opcodes(&block, cfg)).await
    }

    /// Traces a synthetic `eth_call`-shaped request (`debug_traceCall`) with the callTracer.
    /// The call runs against `block`'s state: `None` uses the block's committed post-state
    /// (geth's "on top of the block" default), while `Some(i)` rebuilds the state up to (but
    /// excluding) the block's transaction `i`. See [`Self::build_call_trace_vm`] for the
    /// state-sourcing and `reexec` details.
    #[allow(clippy::too_many_arguments)]
    pub async fn trace_call_calls(
        &self,
        block: Block,
        tx_index: Option<usize>,
        transaction: GenericTransaction,
        reexec: u32,
        timeout: Duration,
        only_top_call: bool,
        with_log: bool,
    ) -> Result<CallTrace, ChainError> {
        let mut vm = self.build_call_trace_vm(&block, tx_index, reexec).await?;
        // Log index base = logs from the txs the call runs on top of: those before
        // `tx_index`, or the whole block when tracing on top of it (`None`).
        let preceding_txs = tx_index.unwrap_or(block.body.transactions.len());
        let log_index_base = self
            .log_index_base(block.hash(), preceding_txs, with_log)
            .await?;
        let header = block.header;
        timeout_trace_operation(timeout, move || {
            vm.trace_call_calls(
                &header,
                &transaction,
                only_top_call,
                with_log,
                log_index_base,
            )
        })
        .await
    }

    /// Traces a synthetic `eth_call`-shaped request (`debug_traceCall`) with the prestateTracer.
    /// See [`Self::trace_call_calls`] for the `tx_index`/`reexec` state-rebuild semantics.
    #[allow(clippy::too_many_arguments)]
    pub async fn trace_call_prestate(
        &self,
        block: Block,
        tx_index: Option<usize>,
        transaction: GenericTransaction,
        reexec: u32,
        timeout: Duration,
        diff_mode: bool,
        include_empty: bool,
    ) -> Result<PrestateResult, ChainError> {
        let mut vm = self.build_call_trace_vm(&block, tx_index, reexec).await?;
        let header = block.header;
        timeout_trace_operation(timeout, move || {
            vm.trace_call_prestate(&header, &transaction, diff_mode, include_empty)
        })
        .await
    }

    /// Traces a synthetic `eth_call`-shaped request (`debug_traceCall`) with the opcode
    /// (EIP-3155) tracer. See [`Self::trace_call_calls`] for the `tx_index`/`reexec`
    /// state-rebuild semantics.
    pub async fn trace_call_opcodes(
        &self,
        block: Block,
        tx_index: Option<usize>,
        transaction: GenericTransaction,
        reexec: u32,
        timeout: Duration,
        cfg: OpcodeTracerConfig,
    ) -> Result<OpcodeTraceResult, ChainError> {
        let mut vm = self.build_call_trace_vm(&block, tx_index, reexec).await?;
        let header = block.header;
        timeout_trace_operation(timeout, move || {
            vm.trace_call_opcodes(&header, &transaction, cfg)
        })
        .await
    }

    /// Builds the [`Evm`] a `debug_traceCall` runs against.
    ///
    /// For `tx_index == None` (geth's "on top of the block" default) the block's already
    /// committed post-state is read directly when present, skipping a full block
    /// re-execution. This is the common path (e.g. tracing a call on `latest`) and matches
    /// what `eth_call` does. When a specific `tx_index` is requested, or the block's state
    /// isn't stored (archive/pruned gap), the parent state is rebuilt and the block re-run
    /// up to `tx_index` (processing withdrawals only when the whole block runs).
    async fn build_call_trace_vm(
        &self,
        block: &Block,
        tx_index: Option<usize>,
        reexec: u32,
    ) -> Result<Evm, ChainError> {
        if tx_index.is_none() && self.storage.has_state_root(block.header.state_root)? {
            let vm_db = StoreVmDatabase::new(self.storage.clone(), block.header.clone())?;
            return Ok(self.new_evm(vm_db)?);
        }
        let mut vm = self
            .rebuild_parent_state(block.header.parent_hash, reexec)
            .await?;
        vm.rerun_block(block, tx_index)?;
        Ok(vm)
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
        let parent_header = self
            .storage
            .get_block_header_by_hash(parent_hash)?
            .ok_or(ChainError::ParentNotFound)?;
        let vm_db = StoreVmDatabase::new_with_block_hash_cache(
            self.storage.clone(),
            parent_header,
            block_hash_cache,
        )?;
        let mut vm = self.new_evm(vm_db)?;
        // Run parents to rebuild pre-state
        for block in blocks_to_re_execute.iter().rev() {
            vm.rerun_block(block, None)?;
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
        if store.has_state_root(parent_block.header.state_root)? {
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
