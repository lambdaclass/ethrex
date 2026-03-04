mod error;
mod execution;

pub mod app_execution;
pub mod app_state;
pub mod app_types;
pub mod handlers;
pub mod incremental_mpt;
#[cfg(feature = "l2")]
pub mod input_converter;

pub use error::ExecutionError;
pub use execution::{BatchExecutionResult, execute_blocks};
