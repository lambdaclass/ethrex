use bytes::Bytes;
use ethrex_core::Address;
use ethrex_core::{types::Log, H256};
use revm::primitives::result::Output as RevmOutput;
use revm::primitives::result::SuccessReason as RevmSuccessReason;
use revm::primitives::ExecutionResult as RevmExecutionResult;

#[derive(Debug)]
pub enum ExecutionResult {
    Success {
        reason: SuccessReason,
        gas_used: u64,
        gas_refunded: u64,
        logs: Vec<Log>,
        output: Output,
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

#[derive(Debug)]
pub enum SuccessReason {
    Stop,
    Return,
    SelfDestruct,
    EofReturnContract,
}

#[derive(Debug)]
pub enum Output {
    Call(Bytes),
    Create(Bytes, Option<Address>),
}

impl From<RevmExecutionResult> for ExecutionResult {
    fn from(val: RevmExecutionResult) -> Self {
        match val {
            RevmExecutionResult::Success {
                reason,
                gas_used,
                gas_refunded,
                logs,
                output,
            } => ExecutionResult::Success {
                reason: match reason {
                    RevmSuccessReason::Stop => SuccessReason::Stop,
                    RevmSuccessReason::Return => SuccessReason::Return,
                    RevmSuccessReason::SelfDestruct => SuccessReason::SelfDestruct,
                    RevmSuccessReason::EofReturnContract => SuccessReason::EofReturnContract,
                },
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
                    RevmOutput::Call(bytes) => Output::Call(bytes.0),
                    RevmOutput::Create(bytes, addr) => Output::Create(
                        bytes.0,
                        addr.map(|addr| Address::from_slice(addr.0.as_ref())),
                    ),
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
            ExecutionResult::Success { output, .. } => match output {
                Output::Call(bytes) => bytes.clone(),
                Output::Create(bytes, _) => bytes.clone(),
            },
            ExecutionResult::Revert { output, .. } => output.clone(),
            ExecutionResult::Halt { .. } => Bytes::new(),
        }
    }
}
