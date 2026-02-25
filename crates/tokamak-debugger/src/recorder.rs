//! [`OpcodeRecorder`] implementation that captures [`StepRecord`]s.

use crate::types::{ReplayConfig, StepRecord};
use ethrex_common::{Address, U256};
use ethrex_levm::call_frame::Stack;
use ethrex_levm::debugger_hook::OpcodeRecorder;

/// Records each opcode step into a `Vec<StepRecord>`.
pub struct DebugRecorder {
    pub steps: Vec<StepRecord>,
    config: ReplayConfig,
}

impl DebugRecorder {
    pub fn new(config: ReplayConfig) -> Self {
        Self {
            steps: Vec::new(),
            config,
        }
    }

    fn capture_stack_top(&self, stack: &Stack) -> Vec<U256> {
        let depth = stack.len();
        let n = self.config.stack_top_capture.min(depth);
        let mut top = Vec::with_capacity(n);
        for i in 0..n {
            if let Some(val) = stack.peek(i) {
                top.push(val);
            }
        }
        top
    }
}

impl OpcodeRecorder for DebugRecorder {
    #[allow(clippy::too_many_arguments)]
    fn record_step(
        &mut self,
        opcode: u8,
        pc: usize,
        gas_remaining: i64,
        depth: usize,
        stack: &Stack,
        memory_size: usize,
        code_address: Address,
    ) {
        let step_index = self.steps.len();
        let stack_top = self.capture_stack_top(stack);
        let stack_depth = stack.len();

        self.steps.push(StepRecord {
            step_index,
            pc,
            opcode,
            depth,
            gas_remaining,
            stack_top,
            stack_depth,
            memory_size,
            code_address,
        });
    }
}
