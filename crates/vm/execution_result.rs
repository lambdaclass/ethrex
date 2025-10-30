use bytes::Bytes;
use ethrex_common::{H256, types::Log};
use ethrex_levm::errors::{ExecutionReport as LevmExecutionReport, TxResult};
use revm::{context::result::ExecutionResult as RevmExecutionResult, primitives::Log as RevmLog};

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
                    .iter()
                    .map(|revm_log| {
                        let RevmLog {
                            address,
                            data: log_data,
                        } = revm_log;

                        Log {
                            address: ethrex_common::Address::from_slice(address.0.as_slice()),
                            data: log_data.data.clone().into(),
                            topics: log_data
                                .topics()
                                .iter()
                                .map(|t| H256::from_slice(t.0.as_slice()))
                                .collect(),
                        }
                    })
                    .collect(),
                output: match output {
                    revm::context::result::Output::Call(bytes) => bytes.into(),
                    revm::context::result::Output::Create(bytes, _address) => bytes.into(),
                },
            },
            RevmExecutionResult::Revert { gas_used, output } => ExecutionResult::Revert {
                gas_used,
                output: output.into(),
            },
            RevmExecutionResult::Halt { reason, gas_used } => ExecutionResult::Halt {
                reason: serde_json::to_string(&reason).expect("Failed to serialize reason"),
                gas_used,
            },
        }
    }
}
