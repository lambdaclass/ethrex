use ethrex_common::types::ChainConfig;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_state_backend::StateError;
use serde::{Deserialize, Serialize};

use crate::TrieError;
use ethrex_common::rkyv_utils::VecVecWrapper;

/// Witness data produced by the client and consumed by the guest program
/// inside the zkVM.
///
/// It is essentially an `RpcExecutionWitness` but it also contains `ChainConfig`,
/// and `first_block_number`.
#[derive(
    Default, Serialize, Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive, Clone,
)]
pub struct ExecutionWitness {
    // Contract bytecodes needed for stateless execution.
    #[rkyv(with = VecVecWrapper)]
    pub codes: Vec<Vec<u8>>,
    /// RLP-encoded block headers needed for stateless execution.
    #[rkyv(with = VecVecWrapper)]
    pub block_headers_bytes: Vec<Vec<u8>>,
    /// The block number of the first block
    pub first_block_number: u64,
    // The chain config.
    pub chain_config: ChainConfig,
    /// Serialized trie proof data (RLP-encoded nodes for MPT, backend-specific for others).
    #[rkyv(with = VecVecWrapper)]
    pub state_proof: Vec<Vec<u8>>,
}

/// Error type for guest program state operations.
#[derive(thiserror::Error, Debug)]
pub enum GuestProgramStateError {
    #[error("Failed to rebuild tries: {0}")]
    RebuildTrie(String),
    #[error("Failed to apply account updates {0}")]
    ApplyAccountUpdates(String),
    #[error("DB error: {0}")]
    Database(String),
    #[error("No block headers stored, should at least store parent header")]
    NoBlockHeaders,
    #[error("Parent block header of block {0} was not found")]
    MissingParentHeaderOf(u64),
    #[error("Non-contiguous block headers (there's a gap in the block headers list)")]
    NoncontiguousBlockHeaders,
    #[error("Trie error: {0}")]
    Trie(#[from] TrieError),
    #[error("RLP Decode: {0}")]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Unreachable code reached: {0}")]
    Unreachable(String),
    #[error("Custom error: {0}")]
    Custom(String),
}

impl From<StateError> for GuestProgramStateError {
    fn from(e: StateError) -> Self {
        GuestProgramStateError::Database(e.to_string())
    }
}
