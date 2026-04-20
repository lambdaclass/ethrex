/// Credible Layer integration with the Phylax Credible Layer.
///
/// This module implements a gRPC client actor that communicates with the Credible Layer
/// Assertion Enforcer sidecar during block building. Transactions that fail assertion
/// validation are dropped before block inclusion.
///
/// The integration is opt-in via the `--credible-layer` CLI flag.
/// When disabled, there is zero overhead.
pub mod client;
pub mod errors;

pub use client::CredibleLayerClient;
pub use errors::CredibleLayerError;

/// Generated protobuf/gRPC types for sidecar.proto
pub mod sidecar_proto {
    tonic::include_proto!("sidecar.transport.v1");
}
