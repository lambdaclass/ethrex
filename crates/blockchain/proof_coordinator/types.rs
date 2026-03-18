//! Shared types for EIP-8025 proof coordination.
//!
//! These types are used by both the proof coordinator and the RPC handlers.
//! RPC-specific types (request/response structs, SSZ header types) live in
//! `ethrex-rpc::engine::proof_types`.

use bytes::Bytes;
use ethrex_common::H256;
use serde::{Deserialize, Serialize};

/// 8-byte proof generation identifier.
pub type ProofGenId = [u8; 8];

/// Maximum size of a single proof in bytes (300 KiB).
pub const MAX_PROOF_SIZE: usize = 307200;

/// Public input committed to by an execution proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicInputV1 {
    pub new_payload_request_root: H256,
}

/// A single execution proof with its type and public input.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionProofV1 {
    /// Opaque proof bytes (max `MAX_PROOF_SIZE`).
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    pub proof_data: Bytes,
    /// Numeric proof type identifier (QUANTITY encoding in JSON-RPC).
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub proof_type: u64,
    /// The public input this proof commits to.
    pub public_input: PublicInputV1,
}

/// A generated proof paired with its generation identifier.
/// Used by the coordinator to deliver proofs via the callback URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedProof {
    /// The identifier assigned when proof generation was initiated.
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    pub proof_gen_id: Bytes,
    /// The generated proof.
    pub execution_proof: ExecutionProofV1,
}
