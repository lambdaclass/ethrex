use bytes::Bytes;

use crate::rkyv_utils::VecVecWrapper;
use crate::serde_utils;
use crate::types::ChainConfig;
use serde::{Deserialize, Serialize};

/// Witness data produced by the client and consumed by the guest program
/// inside the zkVM.
///
/// It is essentially an [`RpcExecutionWitness`] plus [`ChainConfig`] and
/// `first_block_number`.
#[derive(
    Default, Serialize, Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive, Clone,
)]
pub struct ExecutionWitness {
    /// Contract bytecodes needed for stateless execution.
    #[rkyv(with = VecVecWrapper)]
    pub codes: Vec<Vec<u8>>,
    /// RLP-encoded block headers needed for stateless execution.
    #[rkyv(with = VecVecWrapper)]
    pub block_headers_bytes: Vec<Vec<u8>>,
    /// The block number of the first block.
    pub first_block_number: u64,
    /// The chain config.
    pub chain_config: ChainConfig,
    /// Serialized trie proof data (RLP-encoded nodes for MPT, backend-specific for others).
    #[rkyv(with = VecVecWrapper)]
    pub state_proof: Vec<Vec<u8>>,
}

/// RPC-friendly representation of an execution witness.
///
/// This is the format returned by the `debug_executionWitness` RPC method.
/// The trie nodes are pre-serialized to avoid expensive traversal on every RPC request.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RpcExecutionWitness {
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub state: Vec<Bytes>,
    #[serde(
        default,
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub keys: Vec<Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub codes: Vec<Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub headers: Vec<Bytes>,
}

impl From<ExecutionWitness> for RpcExecutionWitness {
    fn from(value: ExecutionWitness) -> Self {
        Self {
            state: value.state_proof.into_iter().map(Bytes::from).collect(),
            keys: Vec::new(),
            codes: value.codes.into_iter().map(Bytes::from).collect(),
            headers: value
                .block_headers_bytes
                .into_iter()
                .map(Bytes::from)
                .collect(),
        }
    }
}

impl RpcExecutionWitness {
    /// Convert an RPC execution witness into the internal [`ExecutionWitness`]
    /// format, passing serialized trie bytes through directly.
    pub fn into_execution_witness(
        self,
        chain_config: ChainConfig,
        first_block_number: u64,
    ) -> Result<ExecutionWitness, ExecutionWitnessConversionError> {
        if first_block_number == 0 {
            return Err(ExecutionWitnessConversionError::FirstBlockNumberZero);
        }
        Ok(ExecutionWitness {
            codes: self.codes.into_iter().map(|b| b.to_vec()).collect(),
            chain_config,
            first_block_number,
            block_headers_bytes: self.headers.into_iter().map(|b| b.to_vec()).collect(),
            state_proof: self.state.into_iter().map(|b| b.to_vec()).collect(),
        })
    }
}

/// Error returned by [`RpcExecutionWitness::into_execution_witness`] when the
/// input is malformed.
#[derive(thiserror::Error, Debug)]
pub enum ExecutionWitnessConversionError {
    #[error("first_block_number must be > 0 (need parent header)")]
    FirstBlockNumberZero,
}
