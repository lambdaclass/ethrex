use ethrex_common::{
    H256,
    types::{BlobsBundleError, BlockHash},
};
use ethrex_rlp::error::RLPDecodeError;
use ethrex_storage::error::StoreError;
use ethrex_trie::TrieError;
use ethrex_vm::EvmError;

// Re-export InvalidBlockError from ethrex-common for backwards compatibility
pub use ethrex_common::InvalidBlockError;

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("Invalid Block: {0}")]
    InvalidBlock(#[from] InvalidBlockError),
    #[error("Parent block not found")]
    ParentNotFound,
    //TODO: If a block with block_number greater than latest plus one is received
    //maybe we are missing data and should wait for syncing
    #[error("The post-state of the parent-block.")]
    ParentStateNotFound,
    #[error("DB error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Trie error: {0}")]
    TrieError(#[from] TrieError),
    #[error("RLP decode error: {0}")]
    RLPDecodeError(#[from] RLPDecodeError),
    #[error("EVM error: {0}")]
    EvmError(EvmError),
    #[error("Invalid Transaction: {0}")]
    InvalidTransaction(String),
    #[error("Failed to generate witness: {0}")]
    WitnessGeneration(String),
    #[error("{0}")]
    Custom(String),
    #[error("Unknown Payload")]
    UnknownPayload,
}

impl From<EvmError> for ChainError {
    fn from(value: EvmError) -> Self {
        match value {
            EvmError::Transaction(err) => {
                ChainError::InvalidBlock(InvalidBlockError::InvalidTransaction(err))
            }
            EvmError::InvalidDepositRequest => ChainError::InvalidBlock(
                InvalidBlockError::InvalidTransaction("Invalid deposit request layout".to_string()),
            ),
            other_errors => ChainError::EvmError(other_errors),
        }
    }
}

#[cfg(feature = "metrics")]
impl ChainError {
    pub fn to_metric(&self) -> &str {
        match self {
            ChainError::InvalidBlock(_) => "invalid_block",
            ChainError::ParentNotFound => "parent_not_found",
            ChainError::ParentStateNotFound => "parent_state_not_found",
            ChainError::StoreError(_) => "store_error",
            ChainError::TrieError(_) => "trie_error",
            ChainError::RLPDecodeError(_) => "rlp_decode_error",
            ChainError::EvmError(_) => "evm_error",
            ChainError::InvalidTransaction(_) => "invalid_transaction",
            ChainError::WitnessGeneration(_) => "witness_generation",
            ChainError::Custom(_) => "custom_error",
            ChainError::UnknownPayload => "unknown_payload",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MempoolError {
    #[error("No block header")]
    NoBlockHeaderError,
    #[error("DB error: {0}")]
    StoreError(#[from] StoreError),
    #[error("BlobsBundle error: {0}")]
    BlobsBundleError(#[from] BlobsBundleError),
    #[error("Transaction max init code size exceeded")]
    TxMaxInitCodeSizeError,
    #[error("Transaction max data size exceeded")]
    TxMaxDataSizeError,
    #[error("Transaction gas limit exceeded")]
    TxGasLimitExceededError,
    #[error(
        "Transaction gas limit exceeds maximum. Transaction hash: {0}, transaction gas limit: {1}"
    )]
    TxMaxGasLimitExceededError(H256, u64),
    #[error("Transaction priority fee above gas fee")]
    TxTipAboveFeeCapError,
    #[error("Transaction intrinsic gas overflow")]
    TxGasOverflowError,
    #[error("Transaction intrinsic gas cost above gas limit")]
    TxIntrinsicGasCostAboveLimitError,
    #[error("Transaction blob base fee too low")]
    TxBlobBaseFeeTooLowError,
    #[error("Blob transaction submited without blobs bundle")]
    BlobTxNoBlobsBundle,
    #[error("Nonce for account too low")]
    NonceTooLow,
    #[error("Nonce already used")]
    InvalidNonce,
    #[error("Transaction chain id mismatch, expected chain id: {0}")]
    InvalidChainId(u64),
    #[error("Account does not have enough balance to cover the tx cost")]
    NotEnoughBalance,
    #[error("Transaction gas fields are invalid")]
    InvalidTxGasvalues,
    #[error("Invalid pooled TxType, expected: {0}")]
    InvalidPooledTxType(u8),
    #[error("Invalid pooled transaction size, differs from expected")]
    InvalidPooledTxSize,
    #[error("Requested pooled transaction was not received")]
    RequestedPooledTxNotFound,
    #[error("Transaction sender is invalid {0}")]
    InvalidTxSender(#[from] ethrex_common::EcdsaError),
    #[error("Attempted to replace a pooled transaction with an underpriced transaction")]
    UnderpricedReplacement,
}

#[derive(Debug)]
pub enum ForkChoiceElement {
    Head,
    Safe,
    Finalized,
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidForkChoice {
    #[error("DB error: {0}")]
    StoreError(#[from] StoreError),
    #[error("The node has not finished syncing.")]
    Syncing,
    #[error("Head hash value is invalid.")]
    InvalidHeadHash,
    #[error("New head block is already canonical. Skipping update.")]
    NewHeadAlreadyCanonical,
    #[error("A fork choice element ({:?}) was not found, but an ancestor was, so it's not a sync problem.", ._0)]
    ElementNotFound(ForkChoiceElement),
    #[error("Pre merge block can't be a fork choice update.")]
    PreMergeBlock,
    #[error("Safe, finalized and head blocks are not in the correct order.")]
    Unordered,
    #[error("The following blocks are not connected between each other: {:?}, {:?}", ._0, ._1)]
    Disconnected(ForkChoiceElement, ForkChoiceElement),
    #[error("Requested head is an invalid block.")]
    InvalidHead,
    #[error("Previously rejected block.")]
    InvalidAncestor(BlockHash),
    #[error("Cannot find link between Head and the canonical chain")]
    UnlinkedHead,

    // TODO(#5564): handle arbitrary reorgs
    #[error("State root of the new head is not reachable from the database")]
    StateNotReachable,
}
