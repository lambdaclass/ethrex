//! EIP-8025 JSON-RPC types for the Engine API proof endpoints.
//!
//! These types handle camelCase JSON serialization matching the Engine API spec.
//! For internal proof types, see `ethrex_blockchain::proof_engine::types`.

use serde::{Deserialize, Serialize};

use ethrex_common::{serde_utils, Address, Bloom, H256};

/// Maximum proof size in bytes (300 KiB).
pub const MAX_PROOF_SIZE: usize = 307_200;

/// Maximum number of execution proofs per payload.
pub const MAX_EXECUTION_PROOFS_PER_PAYLOAD: usize = 4;

/// Minimum required execution proofs for a payload to be considered proven.
pub const MIN_REQUIRED_EXECUTION_PROOFS: usize = 1;

/// Public input for an execution proof (JSON-RPC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicInputV1 {
    pub new_payload_request_root: H256,
}

/// An execution proof for a single payload (JSON-RPC).
///
/// `proofType` is encoded as QUANTITY (hex u64) in the Engine API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionProofV1 {
    #[serde(with = "serde_utils::bytes")]
    pub proof_data: bytes::Bytes,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub proof_type: u64,
    pub public_input: PublicInputV1,
}

/// Proof generation attributes (JSON-RPC).
///
/// Specifies which proof types the CL wants the EL to generate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofAttributesV1 {
    #[serde(with = "quantity_vec")]
    pub proof_types: Vec<u64>,
}

/// Status of proof verification (JSON-RPC response).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofStatusV1 {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ProofStatusV1 {
    pub fn valid() -> Self {
        Self {
            status: "VALID".to_string(),
            error: None,
        }
    }

    pub fn invalid(error: String) -> Self {
        Self {
            status: "INVALID".to_string(),
            error: Some(error),
        }
    }

    pub fn syncing() -> Self {
        Self {
            status: "SYNCING".to_string(),
            error: None,
        }
    }

    pub fn not_supported() -> Self {
        Self {
            status: "NOT_SUPPORTED".to_string(),
            error: None,
        }
    }
}

/// 8-byte proof generation identifier, hex-encoded in JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofGenId(pub [u8; 8]);

impl Serialize for ProofGenId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let hex = format!("0x{}", hex::encode(self.0));
        serializer.serialize_str(&hex)
    }
}

impl<'de> Deserialize<'de> for ProofGenId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 8 {
            return Err(serde::de::Error::custom(format!(
                "expected 8 bytes for ProofGenId, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes);
        Ok(ProofGenId(arr))
    }
}

/// A generated proof with its identifier (callback POST body).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedProof {
    pub proof_gen_id: ProofGenId,
    pub execution_proof: ExecutionProofV1,
}

/// Execution payload header for EIP-8025 (JSON-RPC).
///
/// Contains roots instead of full transaction/withdrawal lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPayloadHeaderV1 {
    pub parent_hash: H256,
    pub fee_recipient: Address,
    pub state_root: H256,
    pub receipts_root: H256,
    pub logs_bloom: Bloom,
    pub prev_randao: H256,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub block_number: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas_limit: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas_used: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub timestamp: u64,
    #[serde(with = "serde_utils::bytes")]
    pub extra_data: bytes::Bytes,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub base_fee_per_gas: u64,
    pub block_hash: H256,
    pub transactions_root: H256,
    pub withdrawals_root: H256,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub blob_gas_used: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub excess_blob_gas: u64,
}

/// New payload request header for EIP-8025 verification (JSON-RPC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPayloadRequestHeaderV1 {
    pub execution_payload_header: ExecutionPayloadHeaderV1,
    pub versioned_hashes: Vec<H256>,
    pub parent_beacon_block_root: H256,
    pub execution_requests: Vec<bytes::Bytes>,
}

/// EIP-8025 specific JSON-RPC error codes.
pub mod error_codes {
    /// The proof is invalid.
    pub const INVALID_PROOF: i64 = -39001;
    /// The header is invalid.
    pub const INVALID_HEADER: i64 = -39002;
    /// The payload is invalid.
    pub const INVALID_PAYLOAD: i64 = -39003;
    /// The proof system / data is unavailable.
    pub const UNAVAILABLE: i64 = -39004;
}

/// Serde helper for Vec<u64> where each element is QUANTITY-encoded.
mod quantity_vec {
    use serde::{Deserialize, Deserializer, Serializer, ser::SerializeSeq};

    pub fn serialize<S>(values: &[u64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(values.len()))?;
        for v in values {
            seq.serialize_element(&format!("0x{v:x}"))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let strings = Vec::<String>::deserialize(deserializer)?;
        strings
            .into_iter()
            .map(|s| {
                let s = s.strip_prefix("0x").unwrap_or(&s);
                u64::from_str_radix(s, 16).map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_status_serialization() {
        let status = ProofStatusV1::valid();
        let json = serde_json::to_string(&status).expect("serialize");
        assert!(json.contains("\"status\":\"VALID\""));
        assert!(!json.contains("error"));
    }

    #[test]
    fn proof_status_invalid_serialization() {
        let status = ProofStatusV1::invalid("bad proof".to_string());
        let json = serde_json::to_string(&status).expect("serialize");
        assert!(json.contains("\"status\":\"INVALID\""));
        assert!(json.contains("\"error\":\"bad proof\""));
    }

    #[test]
    fn proof_gen_id_round_trip() {
        let id = ProofGenId([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"0x0102030405060708\"");
        let deserialized: ProofGenId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, deserialized);
    }

    #[test]
    fn proof_attributes_quantity_encoding() {
        let attrs = ProofAttributesV1 {
            proof_types: vec![1, 2, 255],
        };
        let json = serde_json::to_string(&attrs).expect("serialize");
        assert!(json.contains("\"0x1\""));
        assert!(json.contains("\"0x2\""));
        assert!(json.contains("\"0xff\""));

        let deserialized: ProofAttributesV1 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(attrs, deserialized);
    }

    #[test]
    fn execution_proof_camel_case() {
        let proof = ExecutionProofV1 {
            proof_data: bytes::Bytes::from(vec![0xab, 0xcd]),
            proof_type: 1,
            public_input: PublicInputV1 {
                new_payload_request_root: H256::zero(),
            },
        };
        let json = serde_json::to_string(&proof).expect("serialize");
        assert!(json.contains("\"proofData\""));
        assert!(json.contains("\"proofType\""));
        assert!(json.contains("\"publicInput\""));
        assert!(json.contains("\"newPayloadRequestRoot\""));
    }
}
