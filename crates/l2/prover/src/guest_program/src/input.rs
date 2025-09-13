use ethrex_common::types::{Block, block_execution_witness::ExecutionWitness};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeAs, SerializeAs, serde_as};

#[cfg(feature = "l2")]
use ethrex_common::types::blobs_bundle;

/// Private input variables passed into the zkVM execution program.
#[serde_as]
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct ProgramInput {
    /// blocks to execute
    pub blocks: Vec<Block>,
    /// database containing all the data necessary to execute
    pub db: ExecutionWitness,
    /// value used to calculate base fee
    pub elasticity_multiplier: u64,
    #[cfg(feature = "l2")]
    /// KZG commitment to the blob data
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: blobs_bundle::Commitment,
    #[cfg(feature = "l2")]
    /// KZG opening for a challenge over the blob commitment
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: blobs_bundle::Proof,
}
