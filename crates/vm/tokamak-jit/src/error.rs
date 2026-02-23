//! Error types for the JIT compilation crate.

/// Errors that can occur during JIT compilation or execution.
#[derive(Debug, thiserror::Error)]
pub enum JitError {
    /// LLVM/revmc compilation failed.
    #[error("compilation failed: {0}")]
    CompilationFailed(String),

    /// State adapter conversion error (LEVM ↔ revmc type mismatch).
    #[error("adapter error: {0}")]
    AdapterError(String),

    /// JIT result diverged from interpreter result in validation mode.
    #[error("validation mismatch: {reason}")]
    ValidationMismatch {
        /// Description of the mismatch.
        reason: String,
    },

    /// LLVM backend initialization error.
    #[error("LLVM error: {0}")]
    LlvmError(String),

    /// Bytecode exceeds maximum size for JIT compilation.
    #[error("bytecode too large: {size} bytes (max {max})")]
    BytecodeTooLarge {
        /// Actual bytecode size.
        size: usize,
        /// Maximum allowed size.
        max: usize,
    },
}
