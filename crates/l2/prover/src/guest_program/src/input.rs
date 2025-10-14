use ethrex_common::types::{Block, block_execution_witness::ExecutionWitness};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[cfg(feature = "l2")]
use ethrex_common::types::fee_config::FeeConfig;
/// Private input variables passed into the zkVM execution program.
#[cfg(feature = "l2")]
#[serde_as]
#[allow(clippy::derivable_impls)]
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct ProgramInput {
    /// blocks to execute
    pub blocks: Vec<Block>,
    /// database containing all the data necessary to execute
    pub execution_witness: ExecutionWitness,
    /// value used to calculate base fee
    pub elasticity_multiplier: u64,
    /// Configuration for L2 fees
    pub fee_config: Option<FeeConfig>,
    /// KZG commitment to the blob data
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: ethrex_common::types::blobs_bundle::Commitment,
    /// KZG opening for a challenge over the blob commitment
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: ethrex_common::types::blobs_bundle::Proof,
}

/// Private input variables passed into the zkVM execution program.
#[cfg(not(feature = "l2"))]
#[serde_as]
#[derive(Default, Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct ProgramInput {
    /// blocks to execute
    pub blocks: Vec<Block>,
    /// database containing all the data necessary to execute
    pub execution_witness: ExecutionWitness,
    /// value used to calculate base fee
    pub elasticity_multiplier: u64,
}

#[cfg(feature = "l2")]
impl Default for ProgramInput {
    fn default() -> Self {
        Self {
            blocks: Default::default(),
            execution_witness: ExecutionWitness::default(),
            elasticity_multiplier: Default::default(),
            fee_config: None,
            blob_commitment: [0; 48],
            blob_proof: [0; 48],
        }
    }
}
