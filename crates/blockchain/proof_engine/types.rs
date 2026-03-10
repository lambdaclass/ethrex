//! Engine API types for EIP-8025 (Execution Layer Triggerable Proofs).

use bytes::Bytes;
use ethrex_common::H256;
use serde::{Deserialize, Serialize};

/// 8-byte proof generation identifier.
pub type ProofGenId = [u8; 8];

/// Maximum size of a single proof in bytes (300 KiB).
pub const MAX_PROOF_SIZE: usize = 307200;
/// Maximum execution proofs that can be attached to a payload.
pub const MAX_EXECUTION_PROOFS_PER_PAYLOAD: usize = 4;
/// Minimum required execution proofs per payload.
pub const MIN_REQUIRED_EXECUTION_PROOFS: usize = 1;

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

/// Proof types a prover is willing to generate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofAttributesV1 {
    /// Requested proof type identifiers.
    pub proof_types: Vec<u64>,
}

/// Status of a proof verification or generation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofStatusV1 {
    pub status: ProofValidationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Proof validation status values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProofValidationStatus {
    Valid,
    Invalid,
    Syncing,
    NotSupported,
}

/// A generated proof paired with its generation identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedProof {
    /// The identifier assigned when proof generation was initiated.
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    pub proof_gen_id: Bytes,
    /// The generated proof.
    pub execution_proof: ExecutionProofV1,
}

/// Headerized execution payload for JSON-RPC transport (17 fields matching
/// CL `ExecutionPayloadHeader`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPayloadHeaderV1 {
    pub parent_hash: H256,
    pub fee_recipient: ethrex_common::Address,
    pub state_root: H256,
    pub receipts_root: H256,
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    pub logs_bloom: Bytes,
    pub prev_randao: H256,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub block_number: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub gas_limit: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub gas_used: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub timestamp: u64,
    #[serde(with = "ethrex_common::serde_utils::bytes")]
    pub extra_data: Bytes,
    pub base_fee_per_gas: H256,
    pub block_hash: H256,
    pub transactions_root: H256,
    pub withdrawals_root: H256,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub blob_gas_used: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub excess_blob_gas: u64,
    pub deposit_requests_root: H256,
    pub withdrawal_requests_root: H256,
    pub consolidation_requests_root: H256,
}

/// Headerized new-payload request for JSON-RPC transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPayloadRequestHeaderV1 {
    pub execution_payload_header: ExecutionPayloadHeaderV1,
    pub versioned_hashes: Vec<H256>,
    pub parent_beacon_block_root: H256,
    #[serde(with = "ethrex_common::serde_utils::bytes::vec")]
    pub execution_requests: Vec<Bytes>,
}
