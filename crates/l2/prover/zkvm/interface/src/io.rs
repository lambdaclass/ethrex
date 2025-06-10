use ethrex_common::{
    types::{block_execution_witness::ExecutionWitnessResult, Block, BlockHeader},
    H256,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_with::{serde_as, DeserializeAs, SerializeAs};

#[cfg(feature = "l2")]
use ethrex_common::types::blobs_bundle;

/// Private input variables passed into the zkVM execution program.
#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct ProgramInput {
    /// blocks to execute
    #[serde_as(as = "SerdeJSON")]
    pub blocks: Vec<Block>,
    /// header of the previous block
    #[serde_as(as = "SerdeJSON")]
    pub parent_block_header: BlockHeader,
    /// database containing only the data necessary to execute
    pub db: ExecutionWitnessResult,
    /// value used to calculate base fee
    pub elasticity_multiplier: u64,
    #[cfg(feature = "l2")]
    #[serde_as(as = "[_; 48]")]
    /// KZG commitment to the blob data
    pub blob_commitment: blobs_bundle::Commitment,
    #[cfg(feature = "l2")]
    #[serde_as(as = "[_; 48]")]
    /// KZG opening for a challenge over the blob commitment
    pub blob_proof: blobs_bundle::Proof,
}

impl Default for ProgramInput {
    fn default() -> Self {
        Self {
            blocks: Default::default(),
            parent_block_header: Default::default(),
            db: Default::default(),
            elasticity_multiplier: Default::default(),
            #[cfg(feature = "l2")]
            blob_commitment: [0; 48],
            #[cfg(feature = "l2")]
            blob_proof: [0; 48],
        }
    }
}

/// Public output variables exposed by the zkVM execution program. Some of these are part of
/// the program input.
#[derive(Serialize, Deserialize)]
pub struct ProgramOutput {
    /// initial state trie root hash
    pub initial_state_hash: H256,
    /// final state trie root hash
    pub final_state_hash: H256,
    #[cfg(feature = "l2")]
    /// merkle root of all withdrawals in a batch
    pub withdrawals_merkle_root: H256,
    #[cfg(feature = "l2")]
    /// hash of all the deposit logs made in a batch
    pub deposit_logs_hash: H256,
    #[cfg(feature = "l2")]
    /// blob commitment versioned hash
    pub blob_versioned_hash: H256,
    /// hash of the last block in a batch
    pub last_block_hash: H256,
}

impl ProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        [
            self.initial_state_hash.to_fixed_bytes(),
            self.final_state_hash.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.withdrawals_merkle_root.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.deposit_logs_hash.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.blob_versioned_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
        ]
        .concat()
    }
}

/// Used with [serde_with] to encode a fields into JSON before serializing its bytes. This is
/// necessary because a [BlockHeader] isn't compatible with other encoding formats like bincode or RLP.
pub struct SerdeJSON;

impl<T: Serialize> SerializeAs<T> for SerdeJSON {
    fn serialize_as<S>(val: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut encoded = Vec::new();
        serde_json::to_writer(&mut encoded, val).map_err(serde::ser::Error::custom)?;
        serde_with::Bytes::serialize_as(&encoded, serializer)
    }
}

impl<'de, T: DeserializeOwned> DeserializeAs<'de, T> for SerdeJSON {
    fn deserialize_as<D>(deserializer: D) -> Result<T, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let encoded: Vec<u8> = serde_with::Bytes::deserialize_as(deserializer)?;
        serde_json::from_reader(&encoded[..]).map_err(serde::de::Error::custom)
    }
}
