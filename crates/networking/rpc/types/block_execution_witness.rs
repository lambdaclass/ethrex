use std::collections::HashMap;

use bytes::Bytes;
use ethrex_common::{types::BlockHeader, H160, U256};
use serde::{ser::SerializeSeq, Serialize, Serializer};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionWitnessResult {
    #[serde(serialize_with = "serialize_proofs")]
    pub state: Vec<Vec<u8>>,
    #[serde(serialize_with = "serialize_code")]
    pub codes: Vec<Bytes>,
    #[serde(serialize_with = "serialize_keys")]
    pub keys: HashMap<H160, Vec<U256>>,
    pub block_headers: Vec<BlockHeader>,
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

pub fn serialize_code<S>(value: &Vec<Bytes>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq_serializer = serializer.serialize_seq(Some(value.len()))?;
    for code in value {
        seq_serializer.serialize_element(&format!("0x{}", hex::encode(code)))?;
    }
    seq_serializer.end()
}

pub fn serialize_keys<S>(map: &HashMap<H160, Vec<U256>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq_serializer = serializer.serialize_seq(Some(map.len()))?;

    for (address, keys) in map {
        let key = format!("0x{}", hex::encode(address));
        let values: Vec<String> = keys.iter().map(|v| format!("{:#x}", v)).collect();

        let mut obj = serde_json::Map::new();
        obj.insert(
            key,
            serde_json::Value::Array(values.into_iter().map(serde_json::Value::String).collect()),
        );

        seq_serializer.serialize_element(&obj)?;
    }

    seq_serializer.end()
}
