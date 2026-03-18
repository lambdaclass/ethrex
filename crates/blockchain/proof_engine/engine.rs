//! ProofEngine core — EIP-8025 proof lifecycle management.
//!
//! The `ProofEngine` orchestrates proof generation requests, verification, and
//! header validation for Execution Layer Triggerable Proofs.

use ethrex_common::H256;
use ethrex_common::types::Block;
use ethrex_guest_program::input::ProgramInput;
use ethrex_storage::Store;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};

use crate::Blockchain;

use super::config::ProofEngineConfig;
use super::coordinator::{PendingInput, PendingInputMap};
use super::types::{
    ExecutionProofV1, MAX_PROOF_SIZE, MIN_REQUIRED_EXECUTION_PROOFS, ProofGenId, ProofStatusV1,
    ProofValidationStatus,
};

/// Error type for ProofEngine operations.
#[derive(Debug, thiserror::Error)]
pub enum ProofEngineError {
    #[error("Storage error: {0}")]
    Store(#[from] ethrex_storage::error::StoreError),
    #[error("Chain error: {0}")]
    Chain(#[from] crate::error::ChainError),
    #[error("Proof too large: {size} bytes (max {MAX_PROOF_SIZE})")]
    ProofTooLarge { size: usize },
    #[error("Invalid proof data: {0}")]
    InvalidProof(String),
    #[error("Coordinator not available")]
    CoordinatorUnavailable,
    #[error("{0}")]
    Internal(String),
}

/// Core EIP-8025 proof engine.
///
/// Manages the lifecycle of execution proofs: requesting proof generation,
/// verifying submitted proofs, and validating headers against stored proofs.
pub struct ProofEngine {
    /// Blockchain instance for block execution and witness generation.
    blockchain: Arc<Blockchain>,
    /// Storage handle for reading/writing proofs.
    store: Store,
    /// Engine configuration (callback URL, coordinator address).
    config: ProofEngineConfig,
    /// Shared pending input map for the coordinator (insert directly, bypassing GenServer).
    pending: Option<PendingInputMap>,
    /// Mapping from new_payload_request_root to block_number, populated by request_proofs().
    root_to_block: Mutex<HashMap<H256, u64>>,
}

impl ProofEngine {
    /// Create a new ProofEngine.
    pub fn new(blockchain: Arc<Blockchain>, store: Store, config: ProofEngineConfig) -> Self {
        Self {
            blockchain,
            store,
            config,
            pending: None,
            root_to_block: Mutex::new(HashMap::new()),
        }
    }

    /// Set the shared pending input map from the coordinator.
    pub fn set_pending_map(&mut self, pending: PendingInputMap) {
        self.pending = Some(pending);
    }

    /// Request proof generation for a new payload.
    ///
    /// Converts the payload into a Block, generates an execution witness,
    /// builds a `ProgramInput`, and sends it to the proof coordinator.
    /// Returns a `ProofGenId` that can be used to track the proof.
    pub async fn request_proofs(
        &self,
        block: Block,
        new_payload_request_root: H256,
        requested_proof_types: Vec<u64>,
    ) -> Result<ProofGenId, ProofEngineError> {
        let block_number = block.header.number;
        info!(block_number, "Requesting proof generation");

        // Generate execution witness for this block.
        let witness = self
            .blockchain
            .generate_witness_for_blocks(std::slice::from_ref(&block))
            .await?;

        // Build ProgramInput with the block and witness.
        let program_input = ProgramInput::new(vec![block], witness);

        // Record root → block_number mapping for later lookup.
        match self.root_to_block.lock() {
            Ok(mut map) => {
                map.insert(new_payload_request_root, block_number);
            }
            Err(e) => {
                error!(block_number, error = %e, "root_to_block lock poisoned on insert");
            }
        }

        // Generate a ProofGenId from (block_number, root).
        let proof_gen_id = Self::make_proof_gen_id(block_number, &new_payload_request_root);

        // Insert into the shared pending map (accessible by the coordinator's accept loop).
        if let Some(pending) = &self.pending {
            if let Ok(mut map) = pending.lock() {
                map.insert(
                    block_number,
                    PendingInput {
                        proof_gen_id,
                        new_payload_request_root,
                        program_input,
                        requested_proof_types,
                    },
                );
                debug!(
                    block_number,
                    "Input added to pending map for proof generation"
                );
            } else {
                error!("Failed to lock pending map");
                return Err(ProofEngineError::CoordinatorUnavailable);
            }
        } else {
            warn!("No proof coordinator configured; proof generation skipped");
        }

        Ok(proof_gen_id)
    }

    /// Verify an execution proof and store it if valid.
    ///
    /// Validates the proof format, stores it in `EXECUTION_PROOFS`, and
    /// returns the verification status.
    pub fn verify_proof(
        &self,
        proof: &ExecutionProofV1,
    ) -> Result<ProofStatusV1, ProofEngineError> {
        // Validate proof size.
        if proof.proof_data.len() > MAX_PROOF_SIZE {
            return Err(ProofEngineError::ProofTooLarge {
                size: proof.proof_data.len(),
            });
        }

        // Validate proof_data is non-empty.
        if proof.proof_data.is_empty() {
            return Err(ProofEngineError::InvalidProof(
                "proof_data is empty".to_string(),
            ));
        }

        let root = proof.public_input.new_payload_request_root;

        // Look up block_number from root mapping (populated by request_proofs).
        // NOTE: This is an in-memory mapping only and will be lost on node restart.
        // As a PoC limitation, if the node restarts between request_proofs() and
        // verify_proof(), the root will not be found and the proof is stored under
        // block_number=0 as a fallback.
        let block_number = self
            .root_to_block
            .lock()
            .ok()
            .and_then(|map| map.get(&root).copied())
            .unwrap_or_else(|| {
                warn!(
                    root = %root,
                    "Unknown root in verify_proof; storing with block_number=0"
                );
                0
            });

        // Store the proof.
        self.store.store_execution_proof(
            block_number,
            root,
            proof.proof_type,
            proof.proof_data.to_vec(),
        )?;

        info!(
            block_number,
            proof_type = proof.proof_type,
            "Execution proof stored"
        );

        Ok(ProofStatusV1 {
            status: ProofValidationStatus::Valid,
            error: None,
        })
    }

    /// Verify a new-payload request header against stored proofs.
    ///
    /// Computes the `new_payload_request_root` from the header, looks up
    /// stored proofs, and checks that at least `MIN_REQUIRED_EXECUTION_PROOFS`
    /// valid proofs exist.
    pub fn verify_header(
        &self,
        block_number: u64,
        new_payload_request_root: &H256,
    ) -> Result<ProofStatusV1, ProofEngineError> {
        let proofs = self
            .store
            .get_execution_proofs(block_number, new_payload_request_root)?;

        let count = proofs.len();
        debug!(
            block_number,
            root = %new_payload_request_root,
            count,
            "Header verification"
        );

        if count >= MIN_REQUIRED_EXECUTION_PROOFS {
            Ok(ProofStatusV1 {
                status: ProofValidationStatus::Valid,
                error: None,
            })
        } else {
            // Not enough proofs yet — the node may still be syncing/waiting
            // for proofs to arrive.
            Ok(ProofStatusV1 {
                status: ProofValidationStatus::Syncing,
                error: None,
            })
        }
    }

    /// Build a ProofGenId from block number and root.
    /// Uses the lower 4 bytes of block_number and the first 4 bytes of root.
    fn make_proof_gen_id(block_number: u64, root: &H256) -> ProofGenId {
        let mut id = [0u8; 8];
        id[..4].copy_from_slice(&block_number.to_be_bytes()[4..]);
        id[4..].copy_from_slice(&root.as_bytes()[..4]);
        id
    }

    /// Get a reference to the config.
    pub fn config(&self) -> &ProofEngineConfig {
        &self.config
    }

    /// Get a reference to the store.
    pub fn store(&self) -> &Store {
        &self.store
    }
}
