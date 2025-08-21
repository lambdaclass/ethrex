use ethrex_common::{H256, U256, types::Block};
use ethrex_vm::prover_db::PreExecutionState;
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
    pub pre_execution_state: PreExecutionState,
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

/// JSON serializable program input. This struct is forced to serialize into JSON format.
///
// This is necessary because SP1 uses bincode for serialization into zkVM, which does not play well with
// serde attributes like #[serde(skip)], failing to deserialize with an unrelated error message (this is an old bug).
// As a patch we force serialization into JSON first (which is a format that works well with these attributes).
pub struct JSONProgramInput(pub ProgramInput);

impl Default for ProgramInput {
    fn default() -> Self {
        Self {
            blocks: Default::default(),
            pre_execution_state: PreExecutionState::DB(Box::default()),
            elasticity_multiplier: Default::default(),
            #[cfg(feature = "l2")]
            blob_commitment: [0; 48],
            #[cfg(feature = "l2")]
            blob_proof: [0; 48],
        }
    }
}

impl Serialize for JSONProgramInput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut encoded = Vec::new();
        serde_json::to_writer(&mut encoded, &self.0).map_err(serde::ser::Error::custom)?;
        serde_with::Bytes::serialize_as(&encoded, serializer)
    }
}
impl<'de> Deserialize<'de> for JSONProgramInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let encoded: Vec<u8> = serde_with::Bytes::deserialize_as(deserializer)?;
        let decoded: ProgramInput =
            serde_json::from_reader(&encoded[..]).map_err(serde::de::Error::custom)?;
        Ok(JSONProgramInput(decoded))
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
    /// merkle root of all messages in a batch
    pub l1messages_merkle_root: H256,
    #[cfg(feature = "l2")]
    /// hash of all the privileged transactions made in a batch
    pub privileged_transactions_hash: H256,
    #[cfg(feature = "l2")]
    /// blob commitment versioned hash
    pub blob_versioned_hash: H256,
    /// hash of the last block in a batch
    pub last_block_hash: H256,
    /// chain_id of the network
    pub chain_id: U256,
    /// amount of non-privileged transactions
    pub non_privileged_count: U256,
}

impl ProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        [
            self.initial_state_hash.to_fixed_bytes(),
            self.final_state_hash.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.l1messages_merkle_root.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.privileged_transactions_hash.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.blob_versioned_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
            self.chain_id.to_big_endian(),
            self.non_privileged_count.to_big_endian(),
        ]
        .concat()
    }
}
