//! Core data types for the time-travel debugger.

use bytes::Bytes;
use ethrex_common::{Address, U256};
use ethrex_levm::opcodes::Opcode;
use serde::Serialize;

/// Configuration for replay trace capture.
#[derive(Debug, Clone, Serialize)]
pub struct ReplayConfig {
    /// Number of stack top items to capture per step (default: 8).
    pub stack_top_capture: usize,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            stack_top_capture: 8,
        }
    }
}

/// A single opcode execution step captured during replay.
#[derive(Debug, Clone, Serialize)]
pub struct StepRecord {
    /// Sequential step index (0-based).
    pub step_index: usize,
    /// Program counter before this opcode executed.
    pub pc: usize,
    /// The opcode byte.
    pub opcode: u8,
    /// Call depth (0 = top-level call).
    pub depth: usize,
    /// Gas remaining before this opcode.
    pub gas_remaining: i64,
    /// Top N stack items (index 0 = top of stack).
    pub stack_top: Vec<U256>,
    /// Total number of items on the stack.
    pub stack_depth: usize,
    /// Current memory size in bytes.
    pub memory_size: usize,
    /// Address of the contract being executed.
    pub code_address: Address,
}

impl StepRecord {
    /// Return the human-readable opcode name (e.g. "ADD", "PUSH1").
    pub fn opcode_name(&self) -> String {
        format!("{:?}", Opcode::from(self.opcode))
    }
}

/// Complete execution trace from a transaction replay.
#[derive(Debug, Serialize)]
pub struct ReplayTrace {
    /// All recorded steps.
    pub steps: Vec<StepRecord>,
    /// Configuration used during recording.
    pub config: ReplayConfig,
    /// Total gas used by the transaction.
    pub gas_used: u64,
    /// Whether the transaction succeeded.
    pub success: bool,
    /// Transaction output data.
    pub output: Bytes,
}
