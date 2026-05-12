use bytes::Bytes;
use ethrex_common::{
    H256, U256,
    tracing::{MemoryChunk, OpcodeStep, OpcodeTraceResult},
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
#[derive(Debug)]
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
        }
    }

    /// Returns an active tracer with the given config.
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
        // Enforce limit: stop appending once the cap is reached. Clearing the
        // patch index ensures `finalize_step` does not clobber the last retained
        // step on subsequent opcodes.
        if self.cfg.limit > 0 && self.logs.len() >= self.cfg.limit {
            self.last_step_index = None;
            return;
        }

        let log = build_step(
            &self.cfg,
            pc,
            opcode,
            gas,
            /* gas_cost */ 0, // patched in finalize_step
            depth,
            refund,
            stack_view,
            memory_view,
            mem_size,
            return_data,
            storage_kv,
        );

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
    pub fn finalize_step(&mut self, gas_cost: u64, error: Option<&str>) {
        let Some(idx) = self.last_step_index else {
            return;
        };
        if let Some(log) = self.logs.get_mut(idx) {
            log.gas_cost = gas_cost;
            log.error = error.map(str::to_owned);
        }
    }

    /// Pushes a fully-formed synthetic step (used for fused JUMPDEST under JUMP/JUMPI).
    ///
    /// Does **not** update `last_step_index`, so the pending `finalize_step` for the
    /// parent opcode continues to patch the parent's entry. The limit cap is honored
    /// — synthetic pushes are dropped once `cfg.limit` is reached.
    pub fn synthesize_step(&mut self, step: OpcodeStep) {
        if self.cfg.limit > 0 && self.logs.len() >= self.cfg.limit {
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
}

/// Constructs an [`OpcodeStep`] from raw VM state. Shared between the
/// dispatch-loop hook (`pre_step_capture`) and synthetic-step builders
/// (e.g. fused JUMPDEST under JUMP/JUMPI). Callers pass `gas_cost = 0` when
/// they intend to patch it later in `finalize_step`; synthetic steps pass the
/// known cost directly.
#[expect(
    clippy::too_many_arguments,
    reason = "all fields are required per-step state captured from VM"
)]
pub fn build_step(
    cfg: &OpcodeTracerConfig,
    pc: u64,
    opcode: u8,
    gas: u64,
    gas_cost: u64,
    depth: u32,
    refund: u64,
    stack_view: &[U256],
    memory_view: &[u8],
    mem_size: u64,
    return_data: &Bytes,
    storage_kv: Option<(H256, H256)>,
) -> OpcodeStep {
    // Stack: Some(vec) when capture enabled; None when disabled (emits JSON null).
    let stack = if !cfg.disable_stack {
        Some(stack_view.to_vec())
    } else {
        None
    };

    // Memory: chunked 32-byte slices when enabled; field omitted otherwise.
    // When enabled and memory is empty, emit `Some(vec![])` so the field
    // stays present (an empty array signals "captured, just empty").
    let memory = if cfg.enable_memory {
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
    let return_data_field = if cfg.enable_return_data {
        return_data.clone()
    } else {
        Bytes::new()
    };

    OpcodeStep {
        pc,
        op: opcode,
        gas,
        gas_cost,
        mem_size,
        depth,
        return_data: return_data_field,
        refund,
        stack,
        memory,
        storage,
        error: None,
    }
}
