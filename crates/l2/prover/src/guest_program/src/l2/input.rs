use ethrex_common::types::{
    Block, blobs_bundle, block_execution_witness::ExecutionWitness, fee_config::FeeConfig,
};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

/// Input for the L2 stateless validation program.
#[serde_as]
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct ProgramInput {
    /// Blocks to execute.
    pub blocks: Vec<Block>,
    /// Database containing all the data necessary to execute.
    pub execution_witness: ExecutionWitness,
    /// Value used to calculate base fee.
    pub elasticity_multiplier: u64,
    /// Configuration for L2 fees used for each block.
    pub fee_configs: Vec<FeeConfig>,
    /// KZG commitment to the blob data.
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: blobs_bundle::Commitment,
    /// KZG opening for a challenge over the blob commitment.
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: blobs_bundle::Proof,
}

impl Default for ProgramInput {
    fn default() -> Self {
        Self {
            blocks: Default::default(),
            execution_witness: ExecutionWitness::default(),
            elasticity_multiplier: Default::default(),
            fee_configs: Default::default(),
            blob_commitment: [0; 48],
            blob_proof: [0u8; 48],
        }
    }
}

impl ProgramInput {
    /// Creates a new ProgramInput with the given blocks and execution witness.
    /// L2-specific fields are set to default values.
    pub fn new(blocks: Vec<Block>, execution_witness: ExecutionWitness) -> Self {
        Self {
            blocks,
            execution_witness,
            elasticity_multiplier: ethrex_common::types::ELASTICITY_MULTIPLIER,
            ..Default::default()
        }
    }
}
