//! Test utilities for the ethrex-p2p crate.
//!
//! Re-exposes crate-private internals to the integration test crate WITHOUT
//! widening the production public API: this module is compiled only under
//! `#[cfg(any(test, feature = "test-utils"))]`.

use bytes::Bytes;
use ethrex_rlp::error::RLPDecodeError;

/// Shim over the crate-private
/// `rlpx::eth::eth72::transactions::bytes_to_cell_mask`.
pub fn bytes_to_cell_mask(b: &Bytes) -> Result<Option<u128>, RLPDecodeError> {
    crate::rlpx::eth::eth72::transactions::bytes_to_cell_mask(b)
}

/// Shim over the crate-private `network::serve_p2p_requests`: accepts inbound
/// RLPx connections for `context` without starting discovery.
pub async fn serve_p2p_requests(context: crate::network::P2PContext) {
    crate::network::serve_p2p_requests(context).await
}
