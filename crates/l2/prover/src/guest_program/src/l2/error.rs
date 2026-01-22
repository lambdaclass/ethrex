use ethrex_common::InvalidBlockError;
use ethrex_common::types::BlobsBundleError;
use ethrex_common::types::block_execution_witness::GuestProgramStateError;
use ethrex_l2_common::privileged_transactions::PrivilegedTransactionError;
use ethrex_vm::EvmError;

/// Errors that can occur during L2 stateless block execution.
#[derive(Debug, thiserror::Error)]
pub enum L2ExecutionError {
    #[error("Block validation error: {0}")]
    BlockValidation(InvalidBlockError),
    #[error("Gas validation error: {0}")]
    GasValidation(InvalidBlockError),
    #[error("Requests root validation error: {0}")]
    RequestsRootValidation(InvalidBlockError),
    #[error("Receipts validation error: {0}")]
    ReceiptsRootValidation(InvalidBlockError),
    #[error("EVM error: {0}")]
    Evm(#[from] EvmError),
    #[error("Privileged transaction calculation error: {0}")]
    PrivilegedTransaction(#[from] PrivilegedTransactionError),
    #[error("Blobs bundle error: {0}")]
    BlobsBundle(#[from] BlobsBundleError),
    #[error("KZG error (proof couldn't be verified): {0}")]
    Kzg(#[from] ethrex_crypto::kzg::KzgError),
    #[error("Invalid KZG blob proof")]
    InvalidBlobProof,
    #[error("FeeConfig not provided for L2 execution")]
    FeeConfigNotFound,
    #[error("Batch has no blocks")]
    EmptyBatch,
    #[error("Execution witness error: {0}")]
    GuestProgramState(#[from] GuestProgramStateError),
    #[error("Invalid initial state trie")]
    InvalidInitialStateTrie,
    #[error("Invalid final state trie")]
    InvalidFinalStateTrie,
    #[error("Invalid hash of block {0} (it's not the parent hash of its successor)")]
    InvalidBlockHash(u64),
    #[error("Failed to calculate privileged transaction hash")]
    InvalidPrivilegedTransaction,
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Failed to convert integer")]
    TryInto(#[from] std::num::TryFromIntError),
}

impl From<crate::common::ExecutionError> for L2ExecutionError {
    fn from(err: crate::common::ExecutionError) -> Self {
        use crate::common::ExecutionError;
        match err {
            ExecutionError::BlockValidation(e) => L2ExecutionError::BlockValidation(e),
            ExecutionError::GasValidation(e) => L2ExecutionError::GasValidation(e),
            ExecutionError::RequestsRootValidation(e) => {
                L2ExecutionError::RequestsRootValidation(e)
            }
            ExecutionError::ReceiptsRootValidation(e) => {
                L2ExecutionError::ReceiptsRootValidation(e)
            }
            ExecutionError::Evm(e) => L2ExecutionError::Evm(e),
            ExecutionError::EmptyBatch => L2ExecutionError::EmptyBatch,
            ExecutionError::GuestProgramState(e) => L2ExecutionError::GuestProgramState(e),
            ExecutionError::InvalidInitialStateTrie => L2ExecutionError::InvalidInitialStateTrie,
            ExecutionError::InvalidFinalStateTrie => L2ExecutionError::InvalidFinalStateTrie,
            ExecutionError::InvalidBlockHash(n) => L2ExecutionError::InvalidBlockHash(n),
            ExecutionError::Internal(s) => L2ExecutionError::Internal(s),
        }
    }
}
