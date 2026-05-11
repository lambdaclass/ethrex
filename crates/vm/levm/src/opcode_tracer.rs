use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    tracing::{MemoryChunk, OpcodeStep, OpcodeTraceResult},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Configuration for the opcode (EIP-3155) tracer.
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

/// Per-step opcode tracer for EIP-3155 output.
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
    /// handlers before invoking the child frame.  The dispatch loop prefers this value
    /// over the (incorrect) gas-diff that would include forwarded gas.
    pub last_opcode_gas_cost: Option<u64>,
    /// True iff the most recent `pre_step_capture` pushed a new entry. Set to false
    /// when the `limit` cap is reached so that `finalize_step` does not overwrite the
    /// previously retained step.
    pub last_step_captured: bool,
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
            last_step_captured: false,
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
            last_step_captured: false,
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
        storage_kv: Option<(Address, H256, H256)>,
    ) {
        // Enforce limit: stop appending once the cap is reached. The flag prevents
        // `finalize_step` from clobbering the last retained step on later opcodes.
        if self.cfg.limit > 0 && self.logs.len() >= self.cfg.limit {
            self.last_step_captured = false;
            return;
        }

        // Stack: Some(vec) when capture enabled; None when disabled (emits JSON null).
        let stack = if !self.cfg.disable_stack {
            Some(stack_view.to_vec())
        } else {
            None
        };

        // Memory: chunked 32-byte slices when enabled; field omitted otherwise.
        // Emit Some(vec![]) when enabled and memory is empty (EIP-3155 requires
        // the field present whenever enableMemory=true).
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
        let storage = if let Some((_addr, key, value)) = storage_kv {
            let mut m = BTreeMap::new();
            m.insert(key, value);
            Some(m)
        } else {
            None
        };

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

        self.logs.push(log);
        self.last_step_captured = true;
    }

    /// Patches the most-recently-buffered entry with the actual gas cost and any
    /// step-level error string.  Called immediately after the opcode handler returns.
    /// No-op when the most recent `pre_step_capture` did not push (e.g. limit reached).
    pub fn finalize_step(&mut self, gas_cost: u64, error: Option<&str>) {
        if !self.last_step_captured {
            return;
        }
        if let Some(log) = self.logs.last_mut() {
            log.gas_cost = gas_cost;
            log.error = error.map(str::to_owned);
        }
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
