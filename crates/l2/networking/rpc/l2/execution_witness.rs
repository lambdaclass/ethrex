use ethrex_rpc::{RpcErr, debug::execution_witness::ExecutionWitnessRequest};
use serde_json::Value;

use crate::rpc::RpcApiContext;

/// Copy of the L1 handler for execution witness, but
/// fetches fee configs from the rollup store, as they can vary from block to block.
pub async fn handle_execution_witness(
    _request: &ExecutionWitnessRequest,
    _context: RpcApiContext,
) -> Result<Value, RpcErr> {
    // L2 MPT-based witness generation removed; binary trie variant not yet implemented
    Err(RpcErr::Internal(
        "L2 execution witness not supported on binary trie branch".to_string(),
    ))
}
