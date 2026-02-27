/// Error type for prover backend operations.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("Initialization error: {0}")]
    Initialization(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Execution error: {0}")]
    Execution(String),

    #[error("Proving error: {0}")]
    Proving(String),

    #[error("Verification error: {0}")]
    Verification(String),

    #[error("Batch proof conversion error: {0}")]
    BatchProofConversion(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

impl BackendError {
    pub fn initialization(e: impl std::fmt::Display) -> Self {
        Self::Initialization(e.to_string())
    }

    pub fn serialization(e: impl std::fmt::Display) -> Self {
        Self::Serialization(e.to_string())
    }

    pub fn execution(e: impl std::fmt::Display) -> Self {
        Self::Execution(e.to_string())
    }

    pub fn proving(e: impl std::fmt::Display) -> Self {
        Self::Proving(e.to_string())
    }

    pub fn verification(e: impl std::fmt::Display) -> Self {
        Self::Verification(e.to_string())
    }

    pub fn batch_proof(e: impl std::fmt::Display) -> Self {
        Self::BatchProofConversion(e.to_string())
    }

    pub fn not_implemented(msg: impl Into<String>) -> Self {
        Self::NotImplemented(msg.into())
    }

    pub fn verify_not_supported() -> Self {
        Self::NotImplemented("Verify not implemented for this backend".to_string())
    }
}
