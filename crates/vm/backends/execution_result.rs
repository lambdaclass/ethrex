use bytes::Bytes;
use ethrex_common::Address;
use ethrex_common::{types::Log, H256};
use ethrex_levm::errors::{ExecutionReport as LevmExecutionReport, TxResult};
use revm::primitives::result::Output as RevmOutput;
use revm::primitives::ExecutionResult as RevmExecutionResult;

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

impl From<RevmExecutionResult> for ExecutionResult {
    fn from(val: RevmExecutionResult) -> Self {
        match val {
            RevmExecutionResult::Success {
                reason: _,
                gas_used,
                gas_refunded,
                logs,
                output,
            } => ExecutionResult::Success {
                gas_used,
                gas_refunded,
                logs: logs
                    .into_iter()
                    .map(|log| Log {
                        address: Address::from_slice(log.address.0.as_ref()),
                        topics: log
                            .topics()
                            .iter()
                            .map(|v| H256::from_slice(v.as_slice()))
                            .collect(),
                        data: log.data.data.0,
                    })
                    .collect(),
                output: match output {
                    RevmOutput::Call(bytes) => bytes.0,
                    RevmOutput::Create(bytes, _addr) => bytes.0,
                },
            },
            RevmExecutionResult::Revert { gas_used, output } => ExecutionResult::Revert {
                gas_used,
                output: output.0,
            },
            RevmExecutionResult::Halt { reason, gas_used } => ExecutionResult::Halt {
                reason: format!("{:?}", reason),
                gas_used,
            },
        }
    }
}
impl From<LevmExecutionReport> for ExecutionResult {
    fn from(val: LevmExecutionReport) -> Self {
        match val.result {
            TxResult::Success => ExecutionResult::Success {
                gas_used: val.gas_used - val.gas_refunded,
                gas_refunded: val.gas_refunded,
                logs: val.logs,
                output: val.output,
            },
            TxResult::Revert(_error) => ExecutionResult::Revert {
                gas_used: val.gas_used - val.gas_refunded,
                output: val.output,
            },
        }
    }
}
