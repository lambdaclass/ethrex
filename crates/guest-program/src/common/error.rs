use ethrex_common::InvalidBlockError;
use ethrex_common::types::block_execution_witness::GuestProgramStateError;
use ethrex_vm::EvmError;

/// Errors that can occur during stateless block execution.
///
/// This error type contains variants common to both L1 and L2 execution.
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
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
    #[error("Internal error: {0}")]
    Internal(String),
}
