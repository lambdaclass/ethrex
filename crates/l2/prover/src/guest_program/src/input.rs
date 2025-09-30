use ethrex_common::types::{
    Block, block_execution_witness::ExecutionWitness, fee_config::FeeConfig,
};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[cfg(feature = "l2")]
use ethrex_common::types::blobs_bundle;

/// Private input variables passed into the zkVM execution program.
#[serde_as]
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct ProgramInput {
    /// blocks to execute
    pub blocks: Vec<Block>,
    /// database containing all the data necessary to execute
    pub execution_witness: ExecutionWitness,
    /// value used to calculate base fee
    pub elasticity_multiplier: u64,
    /// Address where collected fees are sent. If None, fees are burned.
    pub fee_config: Option<FeeConfig>,
    #[cfg(feature = "l2")]
    /// KZG commitment to the blob data
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: blobs_bundle::Commitment,
    #[cfg(feature = "l2")]
    /// KZG opening for a challenge over the blob commitment
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: blobs_bundle::Proof,
}

impl Default for ProgramInput {
    fn default() -> Self {
        Self {
            blocks: Default::default(),
            execution_witness: ExecutionWitness::default(),
            elasticity_multiplier: Default::default(),
            fee_config: None,
            #[cfg(feature = "l2")]
            blob_commitment: [0; 48],
            #[cfg(feature = "l2")]
            blob_proof: [0; 48],
        }
    }
}
