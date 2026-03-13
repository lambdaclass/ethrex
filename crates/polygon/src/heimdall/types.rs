use std::str::FromStr;

use ethereum_types::{Address, H256};
use serde::Deserialize;

/// Wrapper for Heimdall v2 JSON responses.
/// Responses are shaped as: `{"height": "0", "result": { ... }}`
#[derive(Debug, Deserialize)]
pub struct HeimdallResponse<T> {
    pub result: T,
}

/// A Bor span defines the validator set for a range of blocks.
///
/// Supports both Heimdall v1 (flat `validators` array) and v2 (nested
/// `validator_set: {validators: [...]}`) response formats.
#[derive(Debug, Clone)]
pub struct Span {
    pub id: u64,
    pub start_block: u64,
    pub end_block: u64,
    pub selected_producers: Vec<Validator>,
    pub validators: Vec<Validator>,
}

/// v2 wraps validators in a nested object.
#[derive(Deserialize)]
struct ValidatorSetWrapper {
    #[serde(default)]
    validators: Vec<Validator>,
}

/// Intermediate struct for flexible Span deserialization (v1 + v2).
#[derive(Deserialize)]
struct SpanHelper {
    #[serde(deserialize_with = "deserialize_string_u64")]
    id: u64,
    #[serde(deserialize_with = "deserialize_string_u64")]
    start_block: u64,
    #[serde(deserialize_with = "deserialize_string_u64")]
    end_block: u64,
    #[serde(default)]
    selected_producers: Vec<Validator>,
    /// v1 format: flat validators array.
    #[serde(default)]
    validators: Vec<Validator>,
    /// v2 format: nested `validator_set: {validators: [...]}`.
    #[serde(default)]
    validator_set: Option<ValidatorSetWrapper>,
}

impl<'de> Deserialize<'de> for Span {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let helper = SpanHelper::deserialize(deserializer)?;
        // v2 nests validators under validator_set; v1 has them flat.
        let validators = if let Some(vs) = helper.validator_set {
            vs.validators
        } else {
            helper.validators
        };
        Ok(Span {
            id: helper.id,
            start_block: helper.start_block,
            end_block: helper.end_block,
            selected_producers: helper.selected_producers,
            validators,
        })
    }
}

/// A Bor validator in a span.
///
/// Accepts both v1 field name (`"ID"`) and v2 (`"val_id"`, `"id"`).
#[derive(Debug, Clone, Deserialize)]
pub struct Validator {
    #[serde(
        rename = "ID",
        alias = "val_id",
        alias = "id",
        deserialize_with = "deserialize_string_u64"
    )]
    pub id: u64,
    pub signer: Address,
    #[serde(deserialize_with = "deserialize_string_u64")]
    pub voting_power: u64,
    #[serde(deserialize_with = "deserialize_string_i64")]
    pub proposer_priority: i64,
}

/// A Heimdall state sync event record.
#[derive(Debug, Clone, Deserialize)]
pub struct EventRecord {
    #[serde(deserialize_with = "deserialize_string_u64")]
    pub id: u64,
    pub contract: Address,
    pub data: String,
    pub tx_hash: H256,
    #[serde(deserialize_with = "deserialize_string_u64")]
    pub log_index: u64,
    pub bor_chain_id: String,
    pub record_time: String,
}

/// A Heimdall milestone.
#[derive(Debug, Clone, Deserialize)]
pub struct Milestone {
    #[serde(
        rename = "ID",
        alias = "id",
        default,
        deserialize_with = "deserialize_string_u64"
    )]
    pub id: u64,
    #[serde(deserialize_with = "deserialize_string_u64")]
    pub start_block: u64,
    #[serde(deserialize_with = "deserialize_string_u64")]
    pub end_block: u64,
    #[serde(deserialize_with = "deserialize_hash_flexible")]
    pub hash: H256,
}

/// A Heimdall checkpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct Checkpoint {
    #[serde(
        rename = "ID",
        alias = "id",
        default,
        deserialize_with = "deserialize_string_u64"
    )]
    pub id: u64,
    #[serde(deserialize_with = "deserialize_string_u64")]
    pub start_block: u64,
    #[serde(deserialize_with = "deserialize_string_u64")]
    pub end_block: u64,
    #[serde(deserialize_with = "deserialize_hash_flexible")]
    pub root_hash: H256,
}

/// Basic Heimdall node status.
#[derive(Debug, Clone, Deserialize)]
pub struct HeimdallStatus {
    pub latest_block_height: String,
    #[serde(default)]
    pub catching_up: bool,
}

/// Response for count endpoints (checkpoints/count, milestones/count).
#[derive(Debug, Clone, Deserialize)]
pub struct CountResult {
    #[serde(deserialize_with = "deserialize_string_u64")]
    pub count: u64,
}

// ---- Deserialization helpers for string-encoded numbers ----

/// Deserializes a value that may be either a number or a string-encoded number.
fn deserialize_string_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNum {
        Str(String),
        Num(u64),
    }

    match StringOrNum::deserialize(deserializer)? {
        StringOrNum::Str(s) => s.parse().map_err(serde::de::Error::custom),
        StringOrNum::Num(n) => Ok(n),
    }
}

/// Deserializes a value that may be either a number or a string-encoded signed number.
fn deserialize_string_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNum {
        Str(String),
        Num(i64),
    }

    match StringOrNum::deserialize(deserializer)? {
        StringOrNum::Str(s) => s.parse().map_err(serde::de::Error::custom),
        StringOrNum::Num(n) => Ok(n),
    }
}

/// Deserializes an H256 hash from either hex ("0x...") or base64 encoding.
///
/// Heimdall v1 returns hashes as hex strings, v2 returns base64-encoded bytes.
fn deserialize_hash_flexible<'de, D>(deserializer: D) -> Result<H256, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = String::deserialize(deserializer)?;
    // v1 hex format: "0x..."
    if s.starts_with("0x") || s.starts_with("0X") {
        return H256::from_str(&s).map_err(serde::de::Error::custom);
    }
    // v2 base64 format
    let bytes = base64_decode(&s)
        .ok_or_else(|| serde::de::Error::custom(format!("invalid base64: {s}")))?;
    if bytes.len() != 32 {
        return Err(serde::de::Error::custom(format!(
            "expected 32 bytes for H256, got {}",
            bytes.len()
        )));
    }
    Ok(H256::from_slice(&bytes))
}

/// Decode a standard base64 string (alphabet A-Z a-z 0-9 +/ with = padding).
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn char_val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let input = input.trim_end_matches('=');
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for &byte in input.as_bytes() {
        buf = (buf << 6) | char_val(byte)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }

    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_span_with_string_numbers() {
        let json = r#"{
            "id": "500",
            "start_block": "1000",
            "end_block": "1999",
            "selected_producers": [
                {
                    "ID": "1",
                    "signer": "0x0000000000000000000000000000000000000001",
                    "voting_power": "100",
                    "proposer_priority": "-5"
                }
            ],
            "validators": []
        }"#;

        let span: Span = serde_json::from_str(json).expect("should parse span");
        assert_eq!(span.id, 500);
        assert_eq!(span.start_block, 1000);
        assert_eq!(span.end_block, 1999);
        assert_eq!(span.selected_producers.len(), 1);
        assert_eq!(span.selected_producers[0].id, 1);
        assert_eq!(span.selected_producers[0].voting_power, 100);
        assert_eq!(span.selected_producers[0].proposer_priority, -5);
    }

    #[test]
    fn deserialize_span_with_numeric_numbers() {
        let json = r#"{
            "id": 500,
            "start_block": 1000,
            "end_block": 1999,
            "selected_producers": [],
            "validators": []
        }"#;

        let span: Span = serde_json::from_str(json).expect("should parse span");
        assert_eq!(span.id, 500);
    }

    #[test]
    fn deserialize_heimdall_response_wrapper() {
        let json = r#"{
            "height": "0",
            "result": {
                "id": "100",
                "start_block": "5000",
                "end_block": "5999",
                "selected_producers": [],
                "validators": []
            }
        }"#;

        let resp: HeimdallResponse<Span> =
            serde_json::from_str(json).expect("should parse wrapped response");
        assert_eq!(resp.result.id, 100);
        assert_eq!(resp.result.start_block, 5000);
    }

    #[test]
    fn deserialize_event_record() {
        let json = r#"{
            "id": "42",
            "contract": "0x0000000000000000000000000000000000001001",
            "data": "0xabcdef",
            "tx_hash": "0x0000000000000000000000000000000000000000000000000000000000001234",
            "log_index": "3",
            "bor_chain_id": "137",
            "record_time": "2023-11-15T10:30:00Z"
        }"#;

        let record: EventRecord = serde_json::from_str(json).expect("should parse event record");
        assert_eq!(record.id, 42);
        assert_eq!(record.log_index, 3);
        assert_eq!(record.bor_chain_id, "137");
    }

    #[test]
    fn deserialize_milestone() {
        let json = r#"{
            "ID": "10",
            "start_block": "50000",
            "end_block": "50100",
            "hash": "0x0000000000000000000000000000000000000000000000000000000000005678"
        }"#;

        let milestone: Milestone = serde_json::from_str(json).expect("should parse milestone");
        assert_eq!(milestone.id, 10);
        assert_eq!(milestone.start_block, 50000);
        assert_eq!(milestone.end_block, 50100);
    }

    #[test]
    fn deserialize_checkpoint() {
        let json = r#"{
            "ID": "7",
            "start_block": "100000",
            "end_block": "100999",
            "root_hash": "0x0000000000000000000000000000000000000000000000000000000000009abc"
        }"#;

        let cp: Checkpoint = serde_json::from_str(json).expect("should parse checkpoint");
        assert_eq!(cp.id, 7);
        assert_eq!(cp.start_block, 100000);
        assert_eq!(cp.end_block, 100999);
    }

    #[test]
    fn deserialize_status() {
        let json = r#"{
            "latest_block_height": "12345678",
            "catching_up": false
        }"#;

        let status: HeimdallStatus = serde_json::from_str(json).expect("should parse status");
        assert_eq!(status.latest_block_height, "12345678");
        assert!(!status.catching_up);
    }

    #[test]
    fn deserialize_count_result() {
        let json = r#"{
            "height": "0",
            "result": {
                "count": "42"
            }
        }"#;

        let resp: HeimdallResponse<CountResult> =
            serde_json::from_str(json).expect("should parse count response");
        assert_eq!(resp.result.count, 42);
    }

    #[test]
    fn deserialize_count_result_numeric() {
        let json = r#"{
            "height": "0",
            "result": {
                "count": 99
            }
        }"#;

        let resp: HeimdallResponse<CountResult> =
            serde_json::from_str(json).expect("should parse numeric count");
        assert_eq!(resp.result.count, 99);
    }

    #[test]
    fn deserialize_event_record_list_response() {
        let json = r#"{
            "height": "0",
            "result": [
                {
                    "id": "1",
                    "contract": "0x0000000000000000000000000000000000001001",
                    "data": "0xaa",
                    "tx_hash": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "log_index": "0",
                    "bor_chain_id": "137",
                    "record_time": "2023-01-01T00:00:00Z"
                },
                {
                    "id": "2",
                    "contract": "0x0000000000000000000000000000000000001001",
                    "data": "0xbb",
                    "tx_hash": "0x0000000000000000000000000000000000000000000000000000000000000002",
                    "log_index": "1",
                    "bor_chain_id": "137",
                    "record_time": "2023-01-01T00:01:00Z"
                }
            ]
        }"#;

        let resp: HeimdallResponse<Vec<EventRecord>> =
            serde_json::from_str(json).expect("should parse event list");
        assert_eq!(resp.result.len(), 2);
        assert_eq!(resp.result[0].id, 1);
        assert_eq!(resp.result[1].id, 2);
    }

    // ---- v2 format tests ----

    #[test]
    fn deserialize_span_v2_nested_validator_set() {
        let json = r#"{
            "id": 500,
            "start_block": 1000,
            "end_block": 1999,
            "bor_chain_id": "80002",
            "selected_producers": [
                {
                    "val_id": 1,
                    "signer": "0x0000000000000000000000000000000000000001",
                    "voting_power": 100,
                    "proposer_priority": -5
                }
            ],
            "validator_set": {
                "validators": [
                    {
                        "val_id": 1,
                        "signer": "0x0000000000000000000000000000000000000001",
                        "voting_power": 100,
                        "proposer_priority": -5
                    },
                    {
                        "val_id": 2,
                        "signer": "0x0000000000000000000000000000000000000002",
                        "voting_power": 200,
                        "proposer_priority": 10
                    }
                ]
            }
        }"#;

        let span: Span = serde_json::from_str(json).expect("should parse v2 span");
        assert_eq!(span.id, 500);
        assert_eq!(span.start_block, 1000);
        assert_eq!(span.end_block, 1999);
        assert_eq!(span.selected_producers.len(), 1);
        assert_eq!(span.selected_producers[0].id, 1);
        // Validators come from the nested validator_set
        assert_eq!(span.validators.len(), 2);
        assert_eq!(span.validators[0].id, 1);
        assert_eq!(span.validators[1].id, 2);
        assert_eq!(span.validators[1].voting_power, 200);
    }

    #[test]
    fn deserialize_validator_v2_val_id() {
        let json = r#"{
            "val_id": 42,
            "signer": "0x0000000000000000000000000000000000000001",
            "voting_power": 100,
            "proposer_priority": 0
        }"#;

        let v: Validator = serde_json::from_str(json).expect("should parse v2 validator");
        assert_eq!(v.id, 42);
        assert_eq!(v.voting_power, 100);
    }

    #[test]
    fn deserialize_milestone_v2_lowercase_id() {
        let json = r#"{
            "id": 10,
            "start_block": 50000,
            "end_block": 50100,
            "hash": "0x0000000000000000000000000000000000000000000000000000000000005678"
        }"#;

        let m: Milestone = serde_json::from_str(json).expect("should parse v2 milestone");
        assert_eq!(m.id, 10);
    }

    #[test]
    fn deserialize_checkpoint_v2_lowercase_id() {
        let json = r#"{
            "id": 7,
            "start_block": 100000,
            "end_block": 100999,
            "root_hash": "0x0000000000000000000000000000000000000000000000000000000000009abc"
        }"#;

        let cp: Checkpoint = serde_json::from_str(json).expect("should parse v2 checkpoint");
        assert_eq!(cp.id, 7);
    }

    #[test]
    fn deserialize_milestone_v2_base64_hash() {
        // Real v2 response: hash is base64-encoded 32 bytes
        let json = r#"{
            "start_block": "35143908",
            "end_block": "35143908",
            "hash": "T3QafVHKkzIoQ4WTNYAeGkt6DXEUVSJ3NYFlCOpCDro=",
            "bor_chain_id": "80002",
            "milestone_id": "512664989449fba33e",
            "timestamp": "1773415349"
        }"#;

        let m: Milestone =
            serde_json::from_str(json).expect("should parse v2 milestone with base64 hash");
        // id defaults to 0 when missing
        assert_eq!(m.id, 0);
        assert_eq!(m.start_block, 35_143_908);
        assert_eq!(m.end_block, 35_143_908);
        // Hash should be decoded from base64
        assert_ne!(m.hash, H256::zero());
    }

    #[test]
    fn deserialize_checkpoint_v2_base64_root_hash() {
        let json = r#"{
            "id": "36791",
            "start_block": "35142259",
            "end_block": "35142770",
            "root_hash": "YnGmEw42hYIDA8EaAY/3qpnKiLaAz41q/ZHADBkegtE=",
            "bor_chain_id": "80002",
            "timestamp": "1773414172"
        }"#;

        let cp: Checkpoint =
            serde_json::from_str(json).expect("should parse v2 checkpoint with base64 root_hash");
        assert_eq!(cp.id, 36_791);
        assert_eq!(cp.start_block, 35_142_259);
        assert_ne!(cp.root_hash, H256::zero());
    }

    #[test]
    fn base64_decode_roundtrip() {
        // "AAAA..." (32 zero bytes in base64)
        let b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let bytes = base64_decode(b64).expect("valid base64");
        assert_eq!(bytes.len(), 32);
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn base64_decode_real_hash() {
        let b64 = "T3QafVHKkzIoQ4WTNYAeGkt6DXEUVSJ3NYFlCOpCDro=";
        let bytes = base64_decode(b64).expect("valid base64");
        assert_eq!(bytes.len(), 32);
    }
}
