mod blobs;
mod error;
mod input;
mod messages;
mod output;
mod program;

pub use error::L2ExecutionError;
pub use input::ProgramInput;
pub use output::ProgramOutput;
pub use program::execution_program;

// Re-export common types for convenience
pub use ethrex_guest_common::{BlockExecutionResult, ExecutionError, execute_blocks};
