//! Core EIP-8025 types used by the ProofEngine.
//!
//! These are the internal types used for proof storage and verification.
//! For JSON-RPC serialized types, see `ethrex_rpc::types::proof`.

use ethrex_common::H256;

/// Maximum proof size in bytes (300 KiB).
pub const MAX_PROOF_SIZE: usize = 307_200;

/// Maximum number of execution proofs per payload.
pub const MAX_EXECUTION_PROOFS_PER_PAYLOAD: usize = 4;

/// Minimum required execution proofs for a payload to be considered proven.
pub const MIN_REQUIRED_EXECUTION_PROOFS: usize = 1;

/// 8-byte proof generation identifier.
pub type ProofGenId = [u8; 8];

/// Public input for an execution proof.
///
/// Contains the `hash_tree_root` of the `NewPayloadRequest` SSZ container
/// that the proof attests to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicInputV1 {
    pub new_payload_request_root: H256,
}

/// An execution proof for a single payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProofV1 {
    /// The raw proof bytes (max 300 KiB).
    pub proof_data: Vec<u8>,
    /// Proof system identifier (e.g. 1 = SP1, 2 = Risc0).
    pub proof_type: u64,
    /// The public input committed to by this proof.
    pub public_input: PublicInputV1,
}

/// Attributes describing which proof types to generate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofAttributesV1 {
    pub proof_types: Vec<u64>,
}

/// Status of proof verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofVerificationStatus {
    Valid,
    Invalid,
    Syncing,
    NotSupported,
}

impl ProofVerificationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Valid => "VALID",
            Self::Invalid => "INVALID",
            Self::Syncing => "SYNCING",
            Self::NotSupported => "NOT_SUPPORTED",
        }
    }
}

/// Result of proof verification, returned by the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofStatusV1 {
    pub status: ProofVerificationStatus,
    pub error: Option<String>,
}

/// A generated proof with its identifier.
///
/// This is the body of the callback POST to the CL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedProof {
    pub proof_gen_id: ProofGenId,
    pub execution_proof: ExecutionProofV1,
}
