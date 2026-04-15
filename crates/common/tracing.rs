use bytes::Bytes;
use ethereum_types::H256;
use ethereum_types::{Address, U256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Collection of traces of each call frame as defined in geth's `callTracer` output
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#call-tracer
pub type CallTrace = Vec<CallTraceFrame>;

/// Trace of each call frame as defined in geth's `callTracer` output
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#call-tracer
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CallTraceFrame {
    /// Type of the Call
    #[serde(rename = "type")]
    pub call_type: CallType,
    /// Address that initiated the call
    pub from: Address,
    /// Address that received the call
    pub to: Address,
    /// Amount transfered
    pub value: U256,
    /// Gas provided for the call
    #[serde(with = "crate::serde_utils::u64::hex_str")]
    pub gas: u64,
    /// Gas used by the call
    #[serde(with = "crate::serde_utils::u64::hex_str")]
    pub gas_used: u64,
    /// Call data
    #[serde(with = "crate::serde_utils::bytes")]
    pub input: Bytes,
    /// Return data
    #[serde(with = "crate::serde_utils::bytes")]
    pub output: Bytes,
    /// Error returned if the call failed
    pub error: Option<String>,
    /// Revert reason if the call reverted
    pub revert_reason: Option<String>,
    /// List of nested sub-calls
    pub calls: Vec<CallTraceFrame>,
    /// Logs (if enabled)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<CallLog>,
}

#[derive(Serialize, Debug, Default)]
pub enum CallType {
    #[default]
    CALL,
    CALLCODE,
    STATICCALL,
    DELEGATECALL,
    CREATE,
    CREATE2,
    SELFDESTRUCT,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CallLog {
    pub address: Address,
    pub topics: Vec<H256>,
    #[serde(with = "crate::serde_utils::bytes")]
    pub data: Bytes,
    pub position: u64,
}

/// Account state as captured by the prestateTracer.
/// Matches Geth's prestateTracer output format.
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#prestate-tracer
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct PrestateAccountState {
    /// Balance as a hex string (e.g. "0x1a2b3c")
    pub balance: String,
    /// Account nonce
    pub nonce: u64,
    /// Bytecode as a hex string (e.g. "0x6060..."), omitted when empty
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Storage slots as hex key -> hex value map, omitted when empty
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub storage: HashMap<String, String>,
}

/// Per-transaction prestate trace (non-diff mode).
/// Maps account address (hex string) to its state before the transaction.
pub type PrestateTrace = HashMap<String, PrestateAccountState>;

/// Per-transaction prestate trace (diff mode).
/// Contains the pre-tx and post-tx state for all touched accounts.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct PrePostState {
    pub pre: HashMap<String, PrestateAccountState>,
    pub post: HashMap<String, PrestateAccountState>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PrestateAccountState serialization ───────────────────────────────────

    #[test]
    fn account_state_serializes_balance_as_hex_string() {
        let state = PrestateAccountState {
            balance: "0x1a2b3c".to_string(),
            nonce: 1,
            code: None,
            storage: HashMap::new(),
        };

        let json = serde_json::to_value(&state).expect("serialization must succeed");
        assert_eq!(json["balance"], "0x1a2b3c");
        assert_eq!(json["nonce"], 1);
    }

    #[test]
    fn account_state_omits_code_when_none() {
        let state = PrestateAccountState {
            balance: "0x0".to_string(),
            nonce: 0,
            code: None,
            storage: HashMap::new(),
        };

        let json = serde_json::to_value(&state).expect("serialization must succeed");
        assert!(
            json.get("code").is_none(),
            "code field must be omitted when None"
        );
    }

    #[test]
    fn account_state_includes_code_when_present() {
        let code_hex = "0x6080604052".to_string();
        let state = PrestateAccountState {
            balance: "0x0".to_string(),
            nonce: 1,
            code: Some(code_hex.clone()),
            storage: HashMap::new(),
        };

        let json = serde_json::to_value(&state).expect("serialization must succeed");
        assert_eq!(json["code"], code_hex);
    }

    #[test]
    fn account_state_omits_storage_when_empty() {
        let state = PrestateAccountState {
            balance: "0x0".to_string(),
            nonce: 0,
            code: None,
            storage: HashMap::new(),
        };

        let json = serde_json::to_value(&state).expect("serialization must succeed");
        assert!(
            json.get("storage").is_none(),
            "storage field must be omitted when empty"
        );
    }

    #[test]
    fn account_state_includes_storage_slots_when_non_empty() {
        let mut storage = HashMap::new();
        storage.insert(
            "0x0000000000000000000000000000000000000000000000000000000000000001".to_string(),
            "0x0000000000000000000000000000000000000000000000000000000000000064".to_string(),
        );

        let state = PrestateAccountState {
            balance: "0x64".to_string(),
            nonce: 2,
            code: None,
            storage: storage.clone(),
        };

        let json = serde_json::to_value(&state).expect("serialization must succeed");
        assert!(json.get("storage").is_some(), "storage must be present when non-empty");
        let slot = &json["storage"]
            ["0x0000000000000000000000000000000000000000000000000000000000000001"];
        assert_eq!(
            slot,
            "0x0000000000000000000000000000000000000000000000000000000000000064"
        );
    }

    // ── PrestateAccountState deserialization ─────────────────────────────────

    #[test]
    fn account_state_deserializes_from_geth_format() {
        let json = serde_json::json!({
            "balance": "0x3b9aca00",
            "nonce": 5,
            "code": "0x6080",
            "storage": {
                "0x0000000000000000000000000000000000000000000000000000000000000000":
                    "0x0000000000000000000000000000000000000000000000000000000000000001"
            }
        });

        let state: PrestateAccountState =
            serde_json::from_value(json).expect("deserialization must succeed");

        assert_eq!(state.balance, "0x3b9aca00");
        assert_eq!(state.nonce, 5);
        assert_eq!(state.code, Some("0x6080".to_string()));
        assert_eq!(state.storage.len(), 1);
    }

    #[test]
    fn account_state_deserializes_without_optional_code_field() {
        // `code` is `Option` so it may be absent. `storage` must be present (no
        // `#[serde(default)]` on the field), matching how Geth actually emits it.
        let json = serde_json::json!({
            "balance": "0x0",
            "nonce": 0,
            "storage": {}
        });

        let state: PrestateAccountState =
            serde_json::from_value(json).expect("deserialization must succeed");

        assert!(state.code.is_none());
        assert!(state.storage.is_empty());
    }

    // ── PrePostState serialization ────────────────────────────────────────────

    #[test]
    fn pre_post_state_serializes_with_pre_and_post_keys() {
        let mut pre = HashMap::new();
        pre.insert(
            "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
            PrestateAccountState {
                balance: "0x100".to_string(),
                nonce: 1,
                code: None,
                storage: HashMap::new(),
            },
        );

        let mut post = HashMap::new();
        post.insert(
            "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
            PrestateAccountState {
                balance: "0x0".to_string(),
                nonce: 1,
                code: None,
                storage: HashMap::new(),
            },
        );

        let pre_post = PrePostState { pre, post };
        let json = serde_json::to_value(&pre_post).expect("serialization must succeed");

        assert!(json.get("pre").is_some(), "pre key must be present");
        assert!(json.get("post").is_some(), "post key must be present");

        let addr = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        assert_eq!(json["pre"][addr]["balance"], "0x100");
        assert_eq!(json["post"][addr]["balance"], "0x0");
    }

    #[test]
    fn pre_post_state_default_is_empty() {
        let state = PrePostState::default();
        assert!(state.pre.is_empty());
        assert!(state.post.is_empty());
    }

    #[test]
    fn pre_post_state_roundtrips_through_json() {
        let mut storage = HashMap::new();
        storage.insert(
            "0x0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            "0x0000000000000000000000000000000000000000000000000000000000000001"
                .to_string(),
        );

        let mut pre = HashMap::new();
        pre.insert(
            "0x1234567890123456789012345678901234567890".to_string(),
            PrestateAccountState {
                balance: "0xde0b6b3a7640000".to_string(),
                nonce: 3,
                code: Some("0x60006000".to_string()),
                storage,
            },
        );

        let original = PrePostState {
            pre,
            post: HashMap::new(),
        };

        // When serialized, accounts with non-empty storage include the storage
        // field. The `post` map is empty so it serializes to `{}`, which
        // deserializes correctly because HashMap deserialization accepts `{}`.
        let json = serde_json::to_value(&original).expect("serialize must succeed");
        let roundtripped: PrePostState =
            serde_json::from_value(json).expect("deserialize must succeed");

        let addr = "0x1234567890123456789012345678901234567890";
        let account = roundtripped.pre.get(addr).expect("account must be present");
        assert_eq!(account.balance, "0xde0b6b3a7640000");
        assert_eq!(account.nonce, 3);
        assert_eq!(account.code, Some("0x60006000".to_string()));
        assert_eq!(account.storage.len(), 1);
    }

    // ── PrestateTracerConfig deserialization (camelCase) ──────────────────────

    /// This mirrors the `PrestateTracerConfig` struct in `crates/networking/rpc/tracing.rs`.
    /// We test that `diffMode` is correctly deserialized from camelCase JSON.
    #[test]
    fn prestate_tracer_config_diff_mode_deserializes_camel_case() {
        // The RPC sends `{"diffMode": true}` — must deserialize correctly.
        let json = serde_json::json!({"diffMode": true});
        // Simulate what PrestateTracerConfig does: we verify the camelCase key works
        // by deserializing into a local struct that mirrors it.
        #[derive(serde::Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct PrestateTracerConfig {
            #[serde(default)]
            diff_mode: bool,
        }

        let cfg: PrestateTracerConfig =
            serde_json::from_value(json).expect("camelCase deserialization must succeed");
        assert!(cfg.diff_mode);
    }

    #[test]
    fn prestate_tracer_config_defaults_diff_mode_to_false() {
        #[derive(serde::Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct PrestateTracerConfig {
            #[serde(default)]
            diff_mode: bool,
        }

        let cfg: PrestateTracerConfig =
            serde_json::from_value(serde_json::json!({}))
                .expect("empty object must deserialize to defaults");
        assert!(!cfg.diff_mode);
    }
}
