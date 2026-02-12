use std::time::Duration;
use tracing::info;

/// Per-phase timing breakdown for one block's journey through engine_newPayload.
///
/// All durations are wall-clock time. Phases that run concurrently
/// (warmer/executor/merkleizer) are measured independently.
#[derive(Debug, Clone, Default)]
pub struct BlockTimings {
    // ── RPC layer (measured in RPC handler thread) ──
    /// JSON param parsing + deserialization
    pub rpc_parse: Duration,
    /// into_block() + compute_transactions_root + compute_withdrawals_root + validate_block_hash
    pub rpc_block_construction: Duration,
    /// Time block waited in unbounded channel before executor picked it up
    pub channel_handoff: Duration,

    // ── Validation ──
    /// validate_block() header checks
    pub validate: Duration,

    // ── Parallel execution scope ──
    /// Warmer thread wall-clock (rayon par_iter warmup)
    pub warmer: Duration,
    /// Executor thread wall-clock (sequential TX execution)
    pub executor: Duration,
    /// Merkleizer wall-clock (dispatch to shards + collect)
    pub merkleizer: Duration,
    /// Merkle time overlapping with executor
    pub merkle_concurrent: Duration,
    /// Merkle time after executor finished (drain)
    pub merkle_drain: Duration,
    /// Max pending items in executor→merkleizer mpsc queue
    pub merkle_queue_high_water: usize,

    // ── Storage ──
    /// store_block total (trie layer wait + RocksDB WriteBatch)
    pub store: Duration,

    // ── Block metadata (for correlation) ──
    pub block_number: u64,
    pub gas_used: u64,
    pub gas_limit: u64,
    pub tx_count: usize,

    // ── Derived ──
    /// Pipeline total: validate + execution scope + store
    pub pipeline_total: Duration,
    /// Full e2e: rpc_parse + block_construction + handoff + pipeline_total
    pub e2e_total: Duration,
}

impl BlockTimings {
    /// Emit the structured `[FLIGHT]` log line for this block.
    pub fn emit_flight_log(&self) {
        let pipeline_ms = self.pipeline_total.as_millis();
        if pipeline_ms == 0 {
            return;
        }

        let e2e_ms = self.e2e_total.as_millis();
        let rpc_parse_ms = self.rpc_parse.as_millis();
        let block_build_ms = self.rpc_block_construction.as_millis();
        let handoff_ms = self.channel_handoff.as_millis();
        let validate_ms = self.validate.as_millis();
        let exec_ms = self.executor.as_millis();
        let merkle_ms = self.merkle_concurrent.as_millis() + self.merkle_drain.as_millis();
        let merkle_concurrent_ms = self.merkle_concurrent.as_millis();
        let merkle_drain_ms = self.merkle_drain.as_millis();
        let store_ms = self.store.as_millis();
        let warmer_ms = self.warmer.as_millis();

        info!(
            "[FLIGHT] block={} gas={} txs={} e2e={}ms rpc_parse={}ms block_build={}ms handoff={}ms pipeline={}ms validate={}ms exec={}ms merkle={}ms(concurrent={}ms drain={}ms) store={}ms warmer={}ms queue_peak={}",
            self.block_number,
            self.gas_used,
            self.tx_count,
            e2e_ms,
            rpc_parse_ms,
            block_build_ms,
            handoff_ms,
            pipeline_ms,
            validate_ms,
            exec_ms,
            merkle_ms,
            merkle_concurrent_ms,
            merkle_drain_ms,
            store_ms,
            warmer_ms,
            self.merkle_queue_high_water,
        );
    }
}
