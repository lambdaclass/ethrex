//! ProofEngine core — EIP-8025 proof lifecycle management.
//!
//! The `ProofEngine` orchestrates proof generation requests, verification, and
//! header validation for Execution Layer Triggerable Proofs.

use bytes::Bytes;
use ethrex_common::types::Block;
use ethrex_common::H256;
use ethrex_storage::Store;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tracing::{debug, error, info, warn};

use crate::Blockchain;

use super::config::ProofEngineConfig;
use super::coordinator::{CoordCastMsg, L1ProofCoordinator};
use super::types::{
    ExecutionProofV1, ProofGenId, ProofStatusV1, ProofValidationStatus,
    MAX_PROOF_SIZE, MIN_REQUIRED_EXECUTION_PROOFS,
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
    #[error("Callback delivery failed: {0}")]
    CallbackFailed(String),
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
    /// Handle to the L1 ProofCoordinator GenServer (if started).
    coordinator: Option<TokioMutex<spawned_concurrency::tasks::GenServerHandle<L1ProofCoordinator>>>,
    /// HTTP client for callback delivery.
    http_client: reqwest::Client,
}

impl ProofEngine {
    /// Create a new ProofEngine.
    pub fn new(
        blockchain: Arc<Blockchain>,
        store: Store,
        config: ProofEngineConfig,
    ) -> Self {
        Self {
            blockchain,
            store,
            config,
            coordinator: None,
            http_client: reqwest::Client::new(),
        }
    }

    /// Set the coordinator handle after the coordinator has been started.
    pub fn set_coordinator(
        &mut self,
        handle: spawned_concurrency::tasks::GenServerHandle<L1ProofCoordinator>,
    ) {
        self.coordinator = Some(TokioMutex::new(handle));
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
    ) -> Result<ProofGenId, ProofEngineError> {
        let block_number = block.header.number;
        info!(block_number, "Requesting proof generation");

        // Generate execution witness for this block.
        let witness = self
            .blockchain
            .generate_witness_for_blocks(std::slice::from_ref(&block))
            .await?;

        // Build ProgramInput (EIP-8025 variant with SSZ NewPayloadRequest).
        // NOTE: The actual SSZ NewPayloadRequest construction from the block
        // will be wired in Phase 5 when the guest program is integrated.
        // For now, we use the L1 ProgramInput with blocks + witness.

        // Generate a ProofGenId from (block_number, root).
        let proof_gen_id = Self::make_proof_gen_id(block_number, &new_payload_request_root);

        // Send to coordinator if available.
        if let Some(coord_mutex) = &self.coordinator {
            let mut coord = coord_mutex.lock().await;
            if let Err(e) = coord
                .cast(CoordCastMsg::NewInput {
                    block_number,
                    proof_gen_id,
                    new_payload_request_root,
                    witness: Box::new(witness),
                })
                .await
            {
                error!("Failed to send input to coordinator: {e}");
                return Err(ProofEngineError::CoordinatorUnavailable);
            }
            debug!(block_number, "Input sent to proof coordinator");
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
        block_number: u64,
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

        // TODO: When a prover backend is integrated, actually verify the
        // proof cryptographically here. For now we accept all well-formed
        // proofs and store them (the coordinator will have verified via the
        // prover backend before submitting).

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
            "Header verification: found {count} proofs"
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

    /// Deliver a generated proof to the configured callback URL.
    pub async fn deliver_proof(
        &self,
        proof_gen_id: ProofGenId,
        proof: ExecutionProofV1,
    ) -> Result<(), ProofEngineError> {
        let Some(callback_url) = &self.config.callback_url else {
            debug!("No callback URL configured; skipping proof delivery");
            return Ok(());
        };

        let body = super::types::GeneratedProof {
            proof_gen_id: Bytes::copy_from_slice(&proof_gen_id),
            execution_proof: proof,
        };

        let resp = self
            .http_client
            .post(callback_url.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|e| ProofEngineError::CallbackFailed(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ProofEngineError::CallbackFailed(format!(
                "HTTP {}",
                resp.status()
            )));
        }

        info!("Proof delivered to callback URL");
        Ok(())
    }

    /// Build a ProofGenId from block number and root.
    /// Uses the first 4 bytes of block_number and first 4 bytes of root.
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
