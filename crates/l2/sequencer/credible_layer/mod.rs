/// Credible Layer integration with the Phylax Credible Layer.
///
/// This module implements a gRPC client that communicates with the Credible Layer
/// Assertion Enforcer sidecar during block building. Transactions that fail assertion
/// validation are dropped before block inclusion.
///
/// The integration is opt-in via the `--credible-layer-url` CLI flag.
/// When disabled, there is zero overhead.
pub mod client;
pub mod aeges;
pub mod errors;

pub use client::CredibleLayerClient;
pub use aeges::AegesClient;
pub use errors::CredibleLayerError;

/// Generated protobuf/gRPC types for sidecar.proto
pub mod sidecar_proto {
    tonic::include_proto!("sidecar.transport.v1");
}

/// Generated protobuf/gRPC types for aeges.proto
pub mod aeges_proto {
    tonic::include_proto!("aeges.v1");
}
