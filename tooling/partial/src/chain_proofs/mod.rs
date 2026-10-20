//! EIP-8440 execution chain proofs on top of the light client.
//!
//! A chain proof attests that every payload from its origin beacon block to its head beacon
//! block executed validly and is bound to the beacon chain. A node that starts from a weak
//! subjectivity checkpoint and sees a verified proof whose origin is at or before that
//! checkpoint has established valid execution up to the proof's head without executing
//! anything. This module verifies proofs through a [`ProofEngine`], maps their heads to
//! execution blocks with the [`ProofTracker`], and fetches them from a beacon node.

pub mod engine;
pub mod tracker;
pub mod types;

use std::sync::Arc;

use ethrex_common::H256;
use tracing::{debug, info, warn};

use self::engine::{ProofEngine, Verdict};
use self::tracker::{AcceptedProof, ProofRejected, ProofTracker};
use self::types::{ExecutionProofsResponse, SignedExecutionProof};
use crate::light_client::beacon_api::{BeaconApi, BeaconApiError};

/// How the node uses chain proofs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProofMode {
    /// Chain proofs are not used.
    #[default]
    Off,
    /// Proofs are fetched and verified; the node reports which heads are proven but serves
    /// the light client's head.
    Observe,
    /// The node serves state only at execution blocks with a verified chain proof reaching
    /// its checkpoint.
    Require,
}

impl std::str::FromStr for ProofMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "observe" => Ok(Self::Observe),
            "require" => Ok(Self::Require),
            other => Err(format!(
                "unknown proof mode {other:?}; expected off, observe or require"
            )),
        }
    }
}

/// Why a proof was not taken.
#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    /// The engine said the proof is invalid.
    #[error("proof engine rejected the proof for head {0:#x}")]
    Invalid(H256),
    /// The engine could not answer.
    #[error(transparent)]
    Engine(#[from] engine::EngineError),
    /// The tracker refused it.
    #[error(transparent)]
    Rejected(#[from] ProofRejected),
    /// The beacon API failed.
    #[error(transparent)]
    Api(#[from] BeaconApiError),
    /// The proof data is empty.
    #[error("proof for head {0:#x} carries no data")]
    Empty(H256),
}

/// Verifies and records chain proofs.
pub struct ChainProofs {
    engine: Arc<dyn ProofEngine>,
    tracker: ProofTracker,
    chain_id: u64,
}

impl ChainProofs {
    /// A verifier over `engine` for the chain `chain_id`, tracking from the checkpoint.
    pub fn new(engine: Arc<dyn ProofEngine>, tracker: ProofTracker, chain_id: u64) -> Self {
        Self {
            engine,
            tracker,
            chain_id,
        }
    }

    /// The tracker.
    pub const fn tracker(&self) -> &ProofTracker {
        &self.tracker
    }

    /// The tracker, mutably: the follower records every head it sees.
    pub const fn tracker_mut(&mut self) -> &mut ProofTracker {
        &mut self.tracker
    }

    /// The engine's name.
    pub fn engine_name(&self) -> &'static str {
        self.engine.name()
    }

    /// Verifies `proof` and records it. `origin_slot` is the origin's slot when the caller
    /// learned it from a beacon header; `None` lets the tracker use a seen block.
    pub async fn submit(
        &mut self,
        proof: &SignedExecutionProof,
        origin_slot: Option<u64>,
    ) -> Result<AcceptedProof, ProofError> {
        let head = H256(proof.message.head_block_root);
        let origin = H256(proof.message.origin_block_root);
        if proof.message.proof_data.as_ref().is_empty() {
            return Err(ProofError::Empty(head));
        }
        if !self.tracker.wants(head, proof.message.proof_type) {
            return Err(ProofRejected::UnknownHead(head).into());
        }
        let public_input = proof.message.public_input(self.chain_id);
        match self.engine.verify(&proof.message, &public_input).await? {
            Verdict::Valid => {}
            Verdict::Invalid => return Err(ProofError::Invalid(head)),
        }
        let accepted = self
            .tracker
            .accept(head, origin, origin_slot, proof.message.proof_type)?;
        info!(
            head = %head,
            origin = %origin,
            proof_type = proof.message.proof_type,
            payload = ?accepted.execution_block_hash,
            "execution chain proof accepted"
        );
        Ok(accepted)
    }

    /// Asks `api` for proofs of the unproven beacon blocks seen so far and submits them.
    /// Origins the tracker does not know are resolved to a slot through a header fetched by
    /// root, whose root is checked. Returns how many proofs were accepted.
    pub async fn fetch_from(&mut self, api: &BeaconApi, limit: usize) -> usize {
        let mut accepted = 0;
        for head in self.tracker.unproven_heads(limit) {
            let path = format!("/eth/v1/beacon/execution_proofs/{head:#x}");
            let response: ExecutionProofsResponse = match api.get_json(&path).await {
                Ok(response) => response,
                Err(BeaconApiError::Status { status: 404, .. }) => continue,
                Err(err) => {
                    debug!(%err, %head, "no proofs from the beacon node");
                    continue;
                }
            };
            for json in response.data {
                let proof = match SignedExecutionProof::try_from(json) {
                    Ok(proof) => proof,
                    Err(err) => {
                        warn!(%err, %head, "malformed proof from the beacon node");
                        continue;
                    }
                };
                let origin = H256(proof.message.origin_block_root);
                let origin_slot = match self.tracker.slot_of(origin) {
                    Some(slot) => Some(slot),
                    None => self.origin_slot(api, origin).await,
                };
                match self.submit(&proof, origin_slot).await {
                    Ok(_) => accepted += 1,
                    Err(err) => debug!(%err, %head, "proof not taken"),
                }
            }
        }
        accepted
    }

    /// The slot of the beacon block `root`, from a header the beacon node serves, accepted
    /// only if the header hashes to `root`.
    async fn origin_slot(&self, api: &BeaconApi, root: H256) -> Option<u64> {
        let header = api.header(&format!("{root:#x}")).await.ok()?;
        let message = header.header.message;
        if message.root() != root {
            warn!(%root, "beacon node served a header that does not hash to the requested root");
            return None;
        }
        Some(message.slot)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::engine::MockProofEngine;
    use super::types::{ExecutionProof, MockProof, PublicInput};
    use super::*;
    use crate::light_client::LcHead;

    fn head(slot: u64, parent: u8, me: u8, exec: u8) -> LcHead {
        LcHead {
            slot,
            beacon_root: H256::repeat_byte(me),
            parent_root: H256::repeat_byte(parent),
            execution_block_hash: H256::repeat_byte(exec),
        }
    }

    fn mock_proof(origin: H256, head: H256, chain_id: u64) -> SignedExecutionProof {
        let input = PublicInput::new(origin, head, chain_id);
        SignedExecutionProof {
            message: ExecutionProof {
                proof_data: MockProof::claiming(&input)
                    .unwrap()
                    .encode()
                    .try_into()
                    .unwrap(),
                proof_type: 3,
                origin_block_root: origin.0,
                head_block_root: head.0,
            },
            validator_index: 0,
            signature: [0u8; 96],
        }
    }

    #[tokio::test]
    async fn a_valid_proof_marks_the_committed_payload_proven() {
        let checkpoint = head(100, 0, 1, 0xa0);
        let mut proofs =
            ChainProofs::new(Arc::new(MockProofEngine), ProofTracker::new(checkpoint), 1);
        proofs.tracker_mut().observe(head(101, 1, 2, 0xa0));
        proofs.tracker_mut().observe(head(102, 2, 3, 0xa1));
        let proof = mock_proof(checkpoint.beacon_root, H256::repeat_byte(2), 1);
        let accepted = proofs.submit(&proof, None).await.unwrap();
        assert_eq!(accepted.execution_block_hash, Some(H256::repeat_byte(0xa1)));
        assert!(proofs.tracker().is_proven(H256::repeat_byte(0xa1)));

        // A proof for the wrong chain id is for another public input.
        let wrong_chain = mock_proof(checkpoint.beacon_root, H256::repeat_byte(3), 2);
        assert!(matches!(
            proofs.submit(&wrong_chain, None).await,
            Err(ProofError::Invalid(_))
        ));
        // A proof whose origin is after the checkpoint is verified but not accepted.
        let recent = mock_proof(H256::repeat_byte(2), H256::repeat_byte(3), 1);
        assert!(matches!(
            proofs.submit(&recent, None).await,
            Err(ProofError::Rejected(ProofRejected::OriginTooRecent { .. }))
        ));
        assert_eq!(proofs.tracker().proof_count(), 1);
    }

    /// What ethlambda's beacon node serves on `GET /eth/v1/beacon/execution_proofs/{root}`
    /// for a mock proof its validator client produced, captured from ethlambda's own
    /// wire test (`the_validator_client_can_prove_the_head_with_a_mock_proof`). The two
    /// implementations share no code, so this pins that the containers, the JSON, the
    /// public input and the mock proof bytes agree end to end.
    #[tokio::test]
    async fn a_proof_served_by_ethlambda_is_accepted() {
        const DEPOSIT_CHAIN_ID: u64 = 1;
        let path = format!(
            "{}/testdata/ethlambda/execution_proofs.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let response: super::types::ExecutionProofsResponse =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(response.data.len(), 1);
        let proof = SignedExecutionProof::try_from(response.data[0].clone()).unwrap();
        assert_eq!(proof.validator_index, 4);
        assert_eq!(proof.message.proof_type, 1);
        assert!(
            proof.message.is_origin(),
            "the first proof starts a recursion"
        );

        // The mock proof's public values are the public input this node computes.
        let engine = MockProofEngine;
        let input = proof.message.public_input(DEPOSIT_CHAIN_ID);
        assert_eq!(
            engine.verify(&proof.message, &input).await.unwrap(),
            super::engine::Verdict::Valid
        );
        let other_chain = proof.message.public_input(DEPOSIT_CHAIN_ID + 1);
        assert_eq!(
            engine.verify(&proof.message, &other_chain).await.unwrap(),
            super::engine::Verdict::Invalid
        );

        // A node whose checkpoint is the proof's origin takes it.
        let origin = H256(proof.message.origin_block_root);
        let checkpoint = LcHead {
            slot: 32,
            beacon_root: origin,
            parent_root: H256::repeat_byte(0x10),
            execution_block_hash: H256::repeat_byte(0xa0),
        };
        let mut proofs = ChainProofs::new(
            Arc::new(MockProofEngine),
            ProofTracker::new(checkpoint),
            DEPOSIT_CHAIN_ID,
        );
        proofs.submit(&proof, None).await.unwrap();
        assert_eq!(proofs.tracker().proof_count(), 1);
    }

    #[test]
    fn proof_mode_parses() {
        assert_eq!("off".parse::<ProofMode>().unwrap(), ProofMode::Off);
        assert_eq!("Observe".parse::<ProofMode>().unwrap(), ProofMode::Observe);
        assert_eq!("REQUIRE".parse::<ProofMode>().unwrap(), ProofMode::Require);
        assert!("strict".parse::<ProofMode>().is_err());
    }
}
