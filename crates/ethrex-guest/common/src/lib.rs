mod error;
mod execution;

pub use error::ExecutionError;
pub use execution::{BatchExecutionResult, execute_blocks};

/// Report cycles used in a code block when running inside SP1 zkVM.
///
/// When the feature "sp1-cycles" is enabled, it will print start and end cycle
/// tracking messages that are compatible with SP1's cycle tracking system.
pub fn report_cycles<T, E>(_label: &str, block: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    #[cfg(feature = "sp1-cycles")]
    println!("cycle-tracker-report-start: {_label}");
    let result = block();
    #[cfg(feature = "sp1-cycles")]
    println!("cycle-tracker-report-end: {_label}");
    result
}
