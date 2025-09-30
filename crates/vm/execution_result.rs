use bytes::Bytes;
use ethrex_common::types::Log;
use ethrex_levm::errors::{ExecutionReport as LevmExecutionReport, TxResult};

#[derive(Debug)]
pub enum ExecutionResult {
    Success {
        gas_used: u64,
        gas_refunded: u64,
        logs: Vec<Log>,
        output: Bytes,
    },
    /// Reverted by `REVERT` opcode
    Revert { gas_used: u64, output: Bytes },
    /// Reverted for other reasons, spends all gas.
    Halt {
        reason: String,
        /// Halting will spend all the gas, which will be equal to gas_limit.
        gas_used: u64,
    },
}

impl ExecutionResult {
    pub fn is_success(&self) -> bool {
        matches!(self, ExecutionResult::Success { .. })
    }
    pub fn gas_used(&self) -> u64 {
        match self {
            ExecutionResult::Success { gas_used, .. } => *gas_used,
            ExecutionResult::Revert { gas_used, .. } => *gas_used,
            ExecutionResult::Halt { gas_used, .. } => *gas_used,
        }
    }
    pub fn logs(&self) -> Vec<Log> {
        match self {
            ExecutionResult::Success { logs, .. } => logs.clone(),
            _ => vec![],
        }
    }
    pub fn gas_refunded(&self) -> u64 {
        match self {
            ExecutionResult::Success { gas_refunded, .. } => *gas_refunded,
            _ => 0,
        }
    }

    pub fn output(&self) -> Bytes {
        match self {
            ExecutionResult::Success { output, .. } => output.clone(),
            ExecutionResult::Revert { output, .. } => output.clone(),
            ExecutionResult::Halt { .. } => Bytes::new(),
        }
    }
}

impl From<LevmExecutionReport> for ExecutionResult {
    fn from(val: LevmExecutionReport) -> Self {
        match val.result {
            TxResult::Success => ExecutionResult::Success {
                gas_used: val.gas_used,
                gas_refunded: val.gas_refunded,
                logs: val.logs,
                output: val.output,
            },
            TxResult::Revert(error) => {
                if error.is_revert_opcode() {
                    ExecutionResult::Revert {
                        gas_used: val.gas_used,
                        output: val.output,
                    }
                } else {
                    ExecutionResult::Halt {
                        reason: error.to_string(),
                        gas_used: val.gas_used,
                    }
                }
            }
        }
    }
}
