use bytes::Bytes;
use ethrex_common::{
    H256, U256,
    tracing::{
        MemoryChunk, OpcodeStep, OpcodeTraceResult, StreamingOpts, write_streaming_state_root,
        write_streaming_step, write_streaming_summary,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Configuration for the per-opcode (EIP-3155) tracer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpcodeTracerConfig {
    /// When true, stack values are not included in each step.
    pub disable_stack: bool,
    /// When true, memory contents are included in each step.
    pub enable_memory: bool,
    /// When true, storage diffs at SLOAD/SSTORE steps are not captured.
    pub disable_storage: bool,
    /// When true, return data from the previous sub-call is included.
    pub enable_return_data: bool,
    /// Maximum number of log entries to collect.  0 = unlimited.
    pub limit: usize,
}

/// Per-opcode (EIP-3155) tracer, emitted under the de-facto cross-client
/// `structLogger` wrapper shape.
///
/// Use `LevmOpcodeTracer::disabled()` when tracing is not wanted;
/// the dispatch-loop guard is a single `if self.opcode_tracer.active` branch
/// with no other overhead on the fast path.
pub struct LevmOpcodeTracer {
    /// Whether this tracer is active.
    pub active: bool,
    /// Configuration.
    pub cfg: OpcodeTracerConfig,
    /// Collected per-step entries.
    pub logs: Vec<OpcodeStep>,
    /// Final output bytes (from RETURN / REVERT).
    pub output: Bytes,
    /// Top-level error string, if the transaction reverted.
    pub error: Option<String>,
    /// Gas used by the transaction.
    pub gas_used: u64,
    /// Explicit gas cost written by CALL/CALLCODE/DELEGATECALL/STATICCALL/CREATE/CREATE2
    /// handlers before invoking the child frame, and by `jump()` when JUMP/JUMPI is
    /// fused with JUMPDEST under active tracing.  The dispatch loop prefers this value
    /// over the (incorrect) gas-diff that would include forwarded gas.
    pub last_opcode_gas_cost: Option<u64>,
    /// Index in `logs` of the entry that the next `finalize_step` should patch.
    /// `Some(i)` is set by `pre_step_capture` after a push; `None` after the
    /// `limit` cap is reached (so `finalize_step` is a no-op).  Synthesized
    /// steps (e.g. fused JUMPDEST) push directly without touching this index,
    /// preserving the parent opcode's pending finalize target.
    pub last_step_index: Option<usize>,
    /// When `Some`, each finalized step is written to this sink and the entry is
    /// dropped from `logs` (streaming mode, O(1) peak memory). When `None`, steps
    /// accumulate in `logs` (RPC mode). Setting this makes the tracer non-Clone.
    pub stream: Option<Box<dyn std::io::Write>>,
    /// EIP-3155 emission options for the streaming sink. Mirrors `cfg` polarity-
    /// inverted (enable→disable) at construction.
    pub stream_opts: StreamingOpts,
    /// Counts steps that have been streamed (so cap checks include them).
    pub streamed_count: u64,
    /// Stores the last write error encountered when streaming. Cleared by
    /// `take_stream_error`.
    pub stream_error: Option<std::io::Error>,
}

impl std::fmt::Debug for LevmOpcodeTracer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LevmOpcodeTracer")
            .field("active", &self.active)
            .field("cfg", &self.cfg)
            .field("logs", &self.logs)
            .field("output", &self.output)
            .field("error", &self.error)
            .field("gas_used", &self.gas_used)
            .field("last_opcode_gas_cost", &self.last_opcode_gas_cost)
            .field("last_step_index", &self.last_step_index)
            .field("stream", &self.stream.as_ref().map(|_| "<sink>"))
            .field("stream_opts", &self.stream_opts)
            .field("streamed_count", &self.streamed_count)
            .field("stream_error", &self.stream_error)
            .finish()
    }
}

impl LevmOpcodeTracer {
    /// Returns an inactive tracer.  No allocations; zero overhead on the hot path.
    pub fn disabled() -> Self {
        Self {
            active: false,
            cfg: OpcodeTracerConfig::default(),
            logs: Vec::new(),
            output: Bytes::new(),
            error: None,
            gas_used: 0,
            last_opcode_gas_cost: None,
            last_step_index: None,
            stream: None,
            stream_opts: StreamingOpts::default(),
            streamed_count: 0,
            stream_error: None,
        }
    }

    /// Returns an active tracer with the given config.  Steps accumulate in
    /// `logs` (RPC mode).
    pub fn new(cfg: OpcodeTracerConfig) -> Self {
        Self {
            active: true,
            cfg,
            logs: Vec::new(),
            output: Bytes::new(),
            error: None,
            gas_used: 0,
            last_opcode_gas_cost: None,
            last_step_index: None,
            stream: None,
            stream_opts: StreamingOpts::default(),
            streamed_count: 0,
            stream_error: None,
        }
    }

    /// Returns an active tracer that writes each finalized step directly to
    /// `sink` (streaming mode).  Peak memory is O(1) regardless of trace
    /// length.  The RPC `logs` accumulator is not used.
    pub fn streaming(cfg: OpcodeTracerConfig, sink: Box<dyn std::io::Write>) -> Self {
        let stream_opts = StreamingOpts {
            disable_stack: cfg.disable_stack,
            disable_memory: !cfg.enable_memory,
            disable_storage: cfg.disable_storage,
            disable_return_data: !cfg.enable_return_data,
        };
        Self {
            active: true,
            cfg,
            logs: Vec::new(),
            output: Bytes::new(),
            error: None,
            gas_used: 0,
            last_opcode_gas_cost: None,
            last_step_index: None,
            stream: Some(sink),
            stream_opts,
            streamed_count: 0,
            stream_error: None,
        }
    }

    /// Captures pre-step state, building and buffering an `OpcodeStep` entry.
    ///
    /// Called BEFORE the opcode executes.  `pc` must be the address of the
    /// current opcode (before `advance_pc(1)`).
    ///
    /// `stack_view` must already be bottom-first (caller reverses LEVM's top-first
    /// layout) and empty when `cfg.disable_stack` is true.
    ///
    /// `memory_view` is the live byte slice for the current frame (caller provides
    /// this only when `cfg.enable_memory` is true; otherwise pass `&[]`).
    ///
    /// `storage_kv` is pre-fetched by the caller via `read_storage_for_trace`; it is
    /// `None` for all opcodes except SLOAD/SSTORE (or when storage capture is disabled).
    #[expect(
        clippy::too_many_arguments,
        reason = "all fields are required per-step state from the dispatch-loop hook"
    )]
    #[expect(
        clippy::as_conversions,
        clippy::arithmetic_side_effects,
        reason = "streamed_count fits in usize on supported 64-bit targets; addition bounded by VM step count"
    )]
    pub fn pre_step_capture(
        &mut self,
        pc: u64,
        opcode: u8,
        gas: u64,
        depth: u32,
        refund: u64,
        stack_view: &[U256],
        memory_view: &[u8],
        mem_size: u64,
        return_data: &Bytes,
        storage_kv: Option<(H256, H256)>,
    ) {
        // After a streaming write failure, stop accumulating — the caller is
        // expected to surface `take_stream_error` and abort. Without this guard
        // `logs` would silently grow into RPC-mode behavior on a stream sink.
        if self.stream_error.is_some() {
            self.last_step_index = None;
            return;
        }

        // Enforce limit: stop appending once the cap is reached (counting both
        // buffered and already-streamed steps). Clearing the patch index ensures
        // `finalize_step` does not clobber the last retained step.
        let total = self.streamed_count as usize + self.logs.len();
        if self.cfg.limit > 0 && total >= self.cfg.limit {
            self.last_step_index = None;
            return;
        }

        // Stack: Some(vec) when capture enabled; None when disabled (emits JSON null).
        let stack = if !self.cfg.disable_stack {
            Some(stack_view.to_vec())
        } else {
            None
        };

        // Memory: chunked 32-byte slices when enabled; field omitted otherwise.
        // When enabled and memory is empty, emit `Some(vec![])` so the field
        // stays present (an empty array signals "captured, just empty").
        let memory = if self.cfg.enable_memory {
            if memory_view.is_empty() {
                Some(vec![])
            } else {
                let chunks = memory_view
                    .chunks(32)
                    .map(|c| {
                        let mut arr = [0u8; 32];
                        if let Some(dst) = arr.get_mut(..c.len()) {
                            dst.copy_from_slice(c);
                        }
                        MemoryChunk(arr)
                    })
                    .collect();
                Some(chunks)
            }
        } else {
            None
        };

        // Storage: single-entry map for this step only (no accumulation).
        let storage = storage_kv.map(|(key, value)| {
            let mut m = BTreeMap::new();
            m.insert(key, value);
            m
        });

        // returnData: actual bytes when enabled; empty Bytes otherwise.
        let return_data_field = if self.cfg.enable_return_data {
            return_data.clone()
        } else {
            Bytes::new()
        };

        let log = OpcodeStep {
            pc,
            op: opcode,
            gas,
            gas_cost: 0, // patched in finalize_step
            mem_size,
            depth,
            return_data: return_data_field,
            refund,
            stack,
            memory,
            storage,
            error: None, // patched in finalize_step
        };

        self.last_step_index = Some(self.logs.len());
        self.logs.push(log);
    }

    /// Patches the entry recorded by the most recent `pre_step_capture` with the
    /// actual gas cost and any step-level error string.  Called immediately after
    /// the opcode handler returns.
    ///
    /// No-op when the most recent `pre_step_capture` did not push (limit reached).
    /// Synthesized entries (e.g. fused JUMPDEST) push directly into `logs` without
    /// updating `last_step_index`, so this still patches the correct parent entry.
    ///
    /// In streaming mode, flushes the patched entry AND any synthetic steps that
    /// were appended after it (e.g. fused JUMPDEST) in order, then drops them
    /// from `logs`.
    #[expect(
        clippy::as_conversions,
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        reason = "idx..end range is valid by construction; usize→u64 fits on 64-bit; step count addition bounded by limit"
    )]
    pub fn finalize_step(&mut self, gas_cost: u64, error: Option<&str>) {
        let Some(idx) = self.last_step_index else {
            return;
        };
        if let Some(log) = self.logs.get_mut(idx) {
            log.gas_cost = gas_cost;
            log.error = error.map(str::to_owned);
        }

        // Streaming mode: flush the patched parent step plus any synthetic steps
        // appended after it (e.g. fused JUMPDEST), then drop them from `logs`.
        if self.stream.is_some() {
            let end = self.logs.len();
            for i in idx..end {
                // Safety: we only enter this branch when stream is Some, and we
                // reborrow inside the loop to satisfy the borrow checker.
                if let Some(sink) = self.stream.as_mut() {
                    match write_streaming_step(sink, &self.logs[i], &self.stream_opts) {
                        Ok(()) => {}
                        Err(e) => {
                            self.stream_error = Some(e);
                            self.stream = None;
                            // Truncate whatever we already iterated up to (i entries from idx).
                            let flushed = i - idx;
                            self.streamed_count += flushed as u64;
                            self.logs.truncate(idx);
                            self.last_step_index = None;
                            return;
                        }
                    }
                }
            }
            let flushed = end - idx;
            self.streamed_count += flushed as u64;
            self.logs.truncate(idx);
            self.last_step_index = None;
        }
    }

    /// Pushes a fully-formed synthetic step (used for fused JUMPDEST under JUMP/JUMPI).
    ///
    /// Does **not** update `last_step_index`, so the pending `finalize_step` for the
    /// parent opcode continues to patch the parent's entry. The limit cap is honored
    /// — synthetic pushes are dropped once `cfg.limit` is reached.
    ///
    /// In streaming mode the step is buffered in `logs` exactly like in RPC mode;
    /// `finalize_step` then flushes both the parent and all following synthetic
    /// steps in order, ensuring correct ordering in the output.
    #[expect(
        clippy::as_conversions,
        clippy::arithmetic_side_effects,
        reason = "streamed_count fits in usize on supported 64-bit targets; addition bounded by VM step count"
    )]
    pub fn synthesize_step(&mut self, step: OpcodeStep) {
        // In streaming mode `logs` is truncated after every `finalize_step`, so
        // a `logs.len()`-only check would never fire. Include `streamed_count`
        // to honor the cap across both modes uniformly.
        let total = self.streamed_count as usize + self.logs.len();
        if self.cfg.limit > 0 && total >= self.cfg.limit {
            return;
        }
        self.logs.push(step);
    }

    /// Assembles the final `OpcodeTraceResult` after the transaction finishes.
    pub fn take_result(&mut self) -> OpcodeTraceResult {
        OpcodeTraceResult {
            pass: self.error.is_none(),
            gas_used: self.gas_used,
            output: std::mem::take(&mut self.output),
            steps: std::mem::take(&mut self.logs),
        }
    }

    /// Writes the streaming summary line `{output, gasUsed, error?}` if a sink
    /// is attached and not failed.  Also flushes the underlying writer.
    /// No-op when no sink is attached.
    pub fn flush_summary(
        &mut self,
        output: &[u8],
        gas_used: u64,
        error: Option<&str>,
    ) -> std::io::Result<()> {
        if let Some(sink) = self.stream.as_mut() {
            write_streaming_summary(sink, output, gas_used, error)?;
            sink.flush()?;
        }
        Ok(())
    }

    /// Writes the `{"stateRoot": "0x..."}` line.  Called by the statetest CLI
    /// after `flush_summary` for conventional streaming shape parity.
    /// No-op when no sink is attached.
    pub fn flush_state_root(&mut self, state_root: H256) -> std::io::Result<()> {
        if let Some(sink) = self.stream.as_mut() {
            write_streaming_state_root(sink, state_root)?;
            sink.flush()?;
        }
        Ok(())
    }

    /// Returns the last write error encountered during streaming, clearing the
    /// stored error.  The binary can check this after `vm.execute()` completes.
    pub fn take_stream_error(&mut self) -> Option<std::io::Error> {
        self.stream_error.take()
    }
}
