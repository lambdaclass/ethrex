//! Test utilities for the ethrex-p2p crate.
//!
//! Re-exposes crate-private internals to the integration test crate WITHOUT
//! widening the production public API: this module is compiled only under
//! `#[cfg(any(test, feature = "test-utils"))]`.

use bytes::Bytes;
use ethrex_common::types::ForkId;
use ethrex_rlp::error::RLPDecodeError;
use secp256k1::SecretKey;

use crate::types::{Node, NodeError, NodeRecord};

/// Shim over the crate-private
/// `rlpx::eth::eth72::transactions::bytes_to_cell_mask`.
pub fn bytes_to_cell_mask(b: &Bytes) -> Result<Option<u128>, RLPDecodeError> {
    crate::rlpx::eth::eth72::transactions::bytes_to_cell_mask(b)
}

/// Shim over the crate-private `network::local_node_record`.
pub fn local_node_record(
    node: &Node,
    fork_id: Option<ForkId>,
    signer: &SecretKey,
) -> Result<NodeRecord, NodeError> {
    crate::network::local_node_record(node, fork_id, signer)
}
