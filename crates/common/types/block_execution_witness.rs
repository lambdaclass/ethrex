use core::fmt;
use std::{collections::HashMap, str::FromStr};

use crate::{types::BlockHeader, H160};
use bytes::Bytes;
use hex::FromHexError;
use serde::{
    de::{self, SeqAccess, Visitor},
    ser::SerializeSeq,
    Deserialize, Deserializer, Serialize, Serializer,
};

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionWitnessResult {
    #[serde(
        serialize_with = "serialize_proofs",
        deserialize_with = "deserialize_state"
    )]
    pub state: Vec<Vec<u8>>,
    #[serde(
        serialize_with = "serialize_code",
        deserialize_with = "deserialize_code"
    )]
    pub codes: Vec<Bytes>,
    #[serde(
        serialize_with = "serialize_storage_tries",
        deserialize_with = "deserialize_storage_tries"
    )]
    pub storage_tries: HashMap<H160, Vec<Vec<u8>>>,
    pub block_headers: Vec<BlockHeader>,
    pub parent_block_header: BlockHeader,
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

pub fn serialize_storage_tries<S>(
    map: &HashMap<H160, Vec<Vec<u8>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq_serializer = serializer.serialize_seq(Some(map.len()))?;

    for (address, keys) in map {
        let address_hex = format!("0x{}", hex::encode(address));
        let values_hex: Vec<String> = keys
            .iter()
            .map(|v| format!("0x{}", hex::encode(v)))
            .collect();

        let mut obj = serde_json::Map::new();
        obj.insert(
            address_hex,
            serde_json::Value::Array(
                values_hex
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );

        seq_serializer.serialize_element(&obj)?;
    }

    seq_serializer.end()
}

pub fn deserialize_state<'de, D>(deserializer: D) -> Result<Vec<Vec<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct HexVecVisitor;

    impl<'de> Visitor<'de> for HexVecVisitor {
        type Value = Vec<Vec<u8>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a list of hex-encoded strings")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut out = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                let bytes = decode_hex(s).map_err(de::Error::custom)?;
                out.push(bytes);
            }
            Ok(out)
        }
    }

    deserializer.deserialize_seq(HexVecVisitor)
}

pub fn deserialize_code<'de, D>(deserializer: D) -> Result<Vec<Bytes>, D::Error>
where
    D: Deserializer<'de>,
{
    struct BytesVecVisitor;

    impl<'de> Visitor<'de> for BytesVecVisitor {
        type Value = Vec<Bytes>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a list of hex-encoded strings")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut out = Vec::new();
            while let Some(hex_str) = seq.next_element::<String>()? {
                if let Ok(decoded) = decode_hex(hex_str) {
                    out.push(Bytes::from(decoded));
                }
            }
            Ok(out)
        }
    }

    deserializer.deserialize_seq(BytesVecVisitor)
}

pub fn deserialize_storage_tries<'de, D>(
    deserializer: D,
) -> Result<HashMap<H160, Vec<Vec<u8>>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct KeysVisitor;

    impl<'de> Visitor<'de> for KeysVisitor {
        type Value = HashMap<H160, Vec<Vec<u8>>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str(
                "a list of maps with H160 keys and array of hex-encoded strings as values",
            )
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut map = HashMap::new();

            while let Some(entry) = seq.next_element::<serde_json::Value>()? {
                let obj = entry
                    .as_object()
                    .ok_or_else(|| de::Error::custom("Expected an object in keys array"))?;

                if obj.len() != 1 {
                    return Err(de::Error::custom(
                        "Each object must contain exactly one key",
                    ));
                }

                for (k, v) in obj {
                    let h160 =
                        H160::from_str(k.trim_start_matches("0x")).map_err(de::Error::custom)?;

                    let arr = v
                        .as_array()
                        .ok_or_else(|| de::Error::custom("Expected array as value"))?;

                    let mut vecs = Vec::new();
                    for item in arr {
                        let s = item
                            .as_str()
                            .ok_or_else(|| de::Error::custom("Expected string in array"))?;
                        let bytes =
                            hex::decode(s.trim_start_matches("0x")).map_err(de::Error::custom)?;
                        vecs.push(bytes);
                    }

                    map.insert(h160, vecs);
                }
            }

            Ok(map)
        }
    }

    deserializer.deserialize_seq(KeysVisitor)
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

fn decode_hex(hex: String) -> Result<Vec<u8>, FromHexError> {
    let mut trimmed = hex.trim_start_matches("0x").to_string();
    if trimmed.len() % 2 != 0 {
        trimmed = "0".to_string() + &trimmed;
    }
    hex::decode(trimmed)
}
