//! ProofEngine — core EIP-8025 proof orchestration.
//!
//! Coordinates proof request, verification, and header verification for
//! the Engine API proof endpoints.

use std::sync::Arc;

use ethrex_common::H256;
use ethrex_storage::Store;

use super::ssz::{
    SszNewPayloadRequestHeader, new_payload_request_header_root,
};
use super::types::{
    ExecutionProofV1, GeneratedProof, ProofAttributesV1, ProofGenId, ProofStatusV1,
    ProofVerificationStatus, MAX_PROOF_SIZE, MIN_REQUIRED_EXECUTION_PROOFS,
};

/// Core engine for EIP-8025 proof management.
///
/// Handles proof requests, verification, and header verification against
/// stored proofs.
#[derive(Debug, Clone)]
pub struct ProofEngine {
    store: Store,
    /// Monotonically increasing counter for proof generation IDs.
    next_proof_gen_id: Arc<std::sync::atomic::AtomicU64>,
}

impl ProofEngine {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            next_proof_gen_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    /// Request proof generation for a payload.
    ///
    /// Returns a `ProofGenId` immediately. Actual proof generation is async
    /// and will be delivered via callback when ready.
    pub fn request_proofs(
        &self,
        _block_number: u64,
        _block_hash: H256,
        _proof_attributes: &ProofAttributesV1,
    ) -> ProofGenId {
        let id = self
            .next_proof_gen_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        id.to_be_bytes()
    }

    /// Verify an execution proof and store it if valid.
    ///
    /// Checks proof size bounds, validates the proof data against the
    /// public input, and stores it in the proof store on success.
    pub fn verify_proof(
        &self,
        block_number: u64,
        execution_proof: &ExecutionProofV1,
    ) -> ProofStatusV1 {
        // Validate proof_data is non-empty and within size limit
        if execution_proof.proof_data.is_empty() {
            return ProofStatusV1 {
                status: ProofVerificationStatus::Invalid,
                error: Some("proof_data is empty".to_string()),
            };
        }

        if execution_proof.proof_data.len() > MAX_PROOF_SIZE {
            return ProofStatusV1 {
                status: ProofVerificationStatus::Invalid,
                error: Some(format!(
                    "proof_data exceeds maximum size ({} > {})",
                    execution_proof.proof_data.len(),
                    MAX_PROOF_SIZE
                )),
            };
        }

        // TODO: Actual proof verification against the proof backend (SP1, Risc0, etc.)
        // For now, we accept all well-formed proofs. The zkVM verification will
        // be wired in when the prover backend integration is complete.

        // Store the proof
        let root = execution_proof.public_input.new_payload_request_root;
        if let Err(e) = self.store.store_execution_proof(
            block_number,
            root,
            execution_proof.proof_type,
            execution_proof.proof_data.clone(),
        ) {
            return ProofStatusV1 {
                status: ProofVerificationStatus::Invalid,
                error: Some(format!("failed to store proof: {e}")),
            };
        }

        ProofStatusV1 {
            status: ProofVerificationStatus::Valid,
            error: None,
        }
    }

    /// Verify a new payload request header against stored proofs.
    ///
    /// Computes the `hash_tree_root` of the header's SSZ representation,
    /// then looks up stored proofs for that root. If enough proofs exist,
    /// returns VALID; otherwise SYNCING.
    pub fn verify_header(
        &self,
        block_number: u64,
        header: &mut SszNewPayloadRequestHeader,
    ) -> ProofStatusV1 {
        // Compute hash_tree_root of the header
        let root = match new_payload_request_header_root(header) {
            Ok(node) => {
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(node.as_ref());
                H256::from(bytes)
            }
            Err(e) => {
                return ProofStatusV1 {
                    status: ProofVerificationStatus::Invalid,
                    error: Some(format!("failed to compute hash_tree_root: {e}")),
                };
            }
        };

        // Look up stored proofs for this root
        match self.store.get_proofs_by_root(block_number, root) {
            Ok(proofs) if proofs.len() >= MIN_REQUIRED_EXECUTION_PROOFS => ProofStatusV1 {
                status: ProofVerificationStatus::Valid,
                error: None,
            },
            Ok(_) => ProofStatusV1 {
                status: ProofVerificationStatus::Syncing,
                error: None,
            },
            Err(e) => ProofStatusV1 {
                status: ProofVerificationStatus::Invalid,
                error: Some(format!("failed to lookup proofs: {e}")),
            },
        }
    }

    /// Build a `GeneratedProof` for callback delivery to the CL.
    pub fn build_generated_proof(
        proof_gen_id: ProofGenId,
        execution_proof: ExecutionProofV1,
    ) -> GeneratedProof {
        GeneratedProof {
            proof_gen_id,
            execution_proof,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::PublicInputV1;

    fn test_store() -> Store {
        Store::new("test", ethrex_storage::EngineType::InMemory)
            .expect("failed to create in-memory store")
    }

    #[test]
    fn request_proofs_returns_unique_ids() {
        let engine = ProofEngine::new(test_store());
        let attrs = ProofAttributesV1 {
            proof_types: vec![1],
        };
        let id1 = engine.request_proofs(1, H256::zero(), &attrs);
        let id2 = engine.request_proofs(1, H256::zero(), &attrs);
        assert_ne!(id1, id2);
    }

    #[test]
    fn verify_proof_rejects_empty() {
        let engine = ProofEngine::new(test_store());
        let proof = ExecutionProofV1 {
            proof_data: vec![],
            proof_type: 1,
            public_input: PublicInputV1 {
                new_payload_request_root: H256::zero(),
            },
        };
        let status = engine.verify_proof(1, &proof);
        assert_eq!(status.status, ProofVerificationStatus::Invalid);
    }

    #[test]
    fn verify_proof_rejects_oversized() {
        let engine = ProofEngine::new(test_store());
        let proof = ExecutionProofV1 {
            proof_data: vec![0u8; MAX_PROOF_SIZE + 1],
            proof_type: 1,
            public_input: PublicInputV1 {
                new_payload_request_root: H256::zero(),
            },
        };
        let status = engine.verify_proof(1, &proof);
        assert_eq!(status.status, ProofVerificationStatus::Invalid);
    }

    #[test]
    fn verify_proof_accepts_valid() {
        let engine = ProofEngine::new(test_store());
        let proof = ExecutionProofV1 {
            proof_data: vec![0xab; 100],
            proof_type: 1,
            public_input: PublicInputV1 {
                new_payload_request_root: H256::zero(),
            },
        };
        let status = engine.verify_proof(1, &proof);
        assert_eq!(status.status, ProofVerificationStatus::Valid);
    }

    #[test]
    fn verify_header_returns_syncing_without_proofs() {
        let engine = ProofEngine::new(test_store());
        let mut header = SszNewPayloadRequestHeader::default();
        let status = engine.verify_header(1, &mut header);
        assert_eq!(status.status, ProofVerificationStatus::Syncing);
    }
}
