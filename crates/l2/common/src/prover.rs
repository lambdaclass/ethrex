use ethrex_common::types::{
    Block, blobs_bundle, block_execution_witness::ExecutionWitness, fee_config::FeeConfig,
};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::calldata::Value;

// Re-export prover types from ethrex-common so existing `ethrex_l2_common::prover::X` paths
// continue to work for all downstream crates.
pub use ethrex_common::types::prover::{ProofBytes, ProofData, ProofFormat, ProverType};

/// Returns empty calldata for a prover type, used as a placeholder when
/// no real proof is available yet. Matches the `OnChainProposer.sol` verify() signature.
pub fn empty_calldata(prover_type: ProverType) -> Vec<Value> {
    match prover_type {
        ProverType::Exec => unimplemented!("Exec prover doesn't generate calldata"),
        _ => vec![Value::Bytes(vec![].into())],
    }
}

/// Returns the on-chain getter name for checking whether this proof type
/// is required by the OnChainProposer contract, or `None` for types that
/// don't have an on-chain verifier.
pub fn verifier_getter(prover_type: ProverType) -> Option<&'static str> {
    match prover_type {
        ProverType::RISC0 => Some("REQUIRE_RISC0_PROOF()"),
        ProverType::SP1 => Some("REQUIRE_SP1_PROOF()"),
        ProverType::TDX => Some("REQUIRE_TDX_PROOF()"),
        ProverType::Exec => None,
    }
}

#[serde_as]
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct ProverInputData {
    pub blocks: Vec<Block>,
    pub execution_witness: ExecutionWitness,
    pub elasticity_multiplier: u64,
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: blobs_bundle::Commitment,
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: blobs_bundle::Proof,
    pub fee_configs: Vec<FeeConfig>,
}

/// Contains the proof data recently created by the prover.
/// It can be either a `ProofCalldata` ready to be sent to the on-chain verifier or a `ProofBytes`
/// to be sent to Aligned.
#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub enum BatchProof {
    ProofCalldata(ProofCalldata),
    ProofBytes(ProofBytes),
}

impl BatchProof {
    pub fn prover_type(&self) -> ProverType {
        match self {
            BatchProof::ProofCalldata(proof) => proof.prover_type,
            BatchProof::ProofBytes(proof) => proof.prover_type,
        }
    }

    pub fn calldata(&self) -> Vec<Value> {
        match self {
            BatchProof::ProofCalldata(proof) => proof.calldata.clone(),
            BatchProof::ProofBytes(proof) => {
                // For TDX proofs stored as ProofBytes, the `proof` field contains
                // the signature that was previously in ProofCalldata.calldata.
                // For zkVM proofs this returns the raw proof bytes as calldata.
                if proof.proof.is_empty() {
                    vec![]
                } else {
                    vec![Value::Bytes(proof.proof.clone().into())]
                }
            }
        }
    }

    pub fn compressed(&self) -> Option<Vec<u8>> {
        match self {
            BatchProof::ProofCalldata(_) => None,
            BatchProof::ProofBytes(proof) => Some(proof.proof.clone()),
        }
    }

    pub fn public_values(&self) -> Vec<u8> {
        match self {
            BatchProof::ProofCalldata(_) => vec![],
            BatchProof::ProofBytes(proof_bytes) => proof_bytes.public_values.clone(),
        }
    }
}

/// Contains the data ready to be sent to the on-chain verifiers.
#[derive(PartialEq, Serialize, Deserialize, Clone, Debug)]
pub struct ProofCalldata {
    pub prover_type: ProverType,
    pub calldata: Vec<Value>,
}
