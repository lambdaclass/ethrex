use bytes::Bytes;
use ethrex_common::types::{ChainConfig, block_execution_witness::RpcExecutionWitness};

use crate::execution_witness::{ExecutionWitness, GuestProgramStateError};

/// Convert an [`ExecutionWitness`] to [`RpcExecutionWitness`].
pub fn execution_witness_to_rpc(value: ExecutionWitness) -> RpcExecutionWitness {
    RpcExecutionWitness {
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

/// Convert an [`RpcExecutionWitness`] into the internal [`ExecutionWitness`]
/// format, passing serialized trie bytes through directly.
pub fn rpc_witness_to_execution(
    rpc_witness: RpcExecutionWitness,
    chain_config: ChainConfig,
    first_block_number: u64,
) -> Result<ExecutionWitness, GuestProgramStateError> {
    if first_block_number == 0 {
        return Err(GuestProgramStateError::Custom(
            "first_block_number must be > 0 (need parent header)".to_string(),
        ));
    }
    Ok(ExecutionWitness {
        codes: rpc_witness.codes.into_iter().map(|b| b.to_vec()).collect(),
        chain_config,
        first_block_number,
        block_headers_bytes: rpc_witness
            .headers
            .into_iter()
            .map(|b| b.to_vec())
            .collect(),
        state_proof: rpc_witness.state.into_iter().map(|b| b.to_vec()).collect(),
    })
}
