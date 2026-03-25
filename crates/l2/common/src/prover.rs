use ethrex_common::types::{
    Block, blobs_bundle, block_execution_witness::ExecutionWitness, fee_config::FeeConfig,
};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

// Re-export prover types from ethrex-common so existing `ethrex_l2_common::prover::X` paths
// continue to work for all downstream crates.
pub use ethrex_common::types::prover::{
    ProofBytes, ProofData, ProofFormat, ProverOutput, ProverType,
};

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
