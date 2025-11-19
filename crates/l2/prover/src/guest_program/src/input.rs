use ethrex_common::types::{Block, block_execution_witness::ExecutionWitness};
use serde_with::serde_as;

/// Private input variables passed into the zkVM execution program.
#[cfg(not(feature = "l2"))]
#[serde_as]
#[derive(
    serde::Serialize, serde::Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive, Default,
)]
pub struct ProgramInput {
    /// Block to execute
    pub block: Block,
    /// database containing all the data necessary to execute
    pub execution_witness: ExecutionWitness,
}

/// Private input variables passed into the zkVM execution program.
#[cfg(feature = "l2")]
#[serde_as]
#[derive(
    serde::Serialize, serde::Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive,
)]
pub struct ProgramInput {
    /// blocks to execute
    pub blocks: Vec<Block>,
    /// database containing all the data necessary to execute
    pub execution_witness: ExecutionWitness,
    /// value used to calculate base fee
    pub elasticity_multiplier: u64,
    /// Configuration for L2 fees used for each block
    pub fee_configs: Vec<ethrex_common::types::fee_config::FeeConfig>,
    /// KZG commitment to the blob data
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: ethrex_common::types::blobs_bundle::Commitment,
    /// KZG opening for a challenge over the blob commitment
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: ethrex_common::types::blobs_bundle::Proof,
}

#[cfg(feature = "l2")]
impl Default for ProgramInput {
    fn default() -> Self {
        Self {
            blocks: Vec::default(),
            execution_witness: ExecutionWitness::default(),
            elasticity_multiplier: u64::default(),
            fee_configs: Vec::default(),
            blob_commitment: [0; 48],
            blob_proof: [0; 48],
        }
    }
}
