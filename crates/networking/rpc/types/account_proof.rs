use ethrex_core::{serde_utils, Address, H256, U256};
use serde::{ser::SerializeSeq, Serialize, Serializer};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProof {
    #[serde(serialize_with = "serialize_proofs")]
    pub account_proof: Vec<Vec<u8>>,
    pub address: Address,
    pub balance: U256,
    pub code_hash: H256,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub nonce: u64,
    pub storage_hash: H256,
    pub storage_proof: Vec<StorageProof>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageProof {
    pub key: U256,
    #[serde(serialize_with = "serialize_proofs")]
    pub proof: Vec<Vec<u8>>,
    pub value: U256,
}

pub fn serialize_proofs<S>(value: &Vec<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq_serializer = serializer.serialize_seq(Some(value.len()))?;
    for encoded_node in value {
        seq_serializer.serialize_element(&format!("0x{}", hex::encode(encoded_node)))?;
    }
    seq_serializer.end()
}
