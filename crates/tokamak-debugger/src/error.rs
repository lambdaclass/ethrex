//! Error types for the time-travel debugger.

use ethrex_levm::errors::VMError;

#[derive(Debug, thiserror::Error)]
pub enum DebuggerError {
    #[error("VM error: {0}")]
    Vm(#[from] VMError),

    #[error("Step {index} out of range (max {max})")]
    StepOutOfRange { index: usize, max: usize },
}
