use ethrex_common::{Address, H256, U256, serde_utils};
use serde::{Serialize, Serializer, ser::SerializeSeq};

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
    /// The requested key in its echo form: 32 bytes when given with 63 or 64
    /// hex digits, otherwise a minimal quantity.
    pub key: String,
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
