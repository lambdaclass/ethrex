use bytes::Bytes;
use ethereum_types::H256;
use ethereum_types::{Address, U256};
use serde::Serialize;
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

/// Per-account state entry emitted by the prestateTracer.
///
/// `balance` is `Option<U256>`: `None` means "field absent from output",
/// `Some(0)` still serializes (lets diff post emit a balance that became zero).
#[derive(Debug, Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PrestateAccountState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<U256>,
    #[serde(default, skip_serializing_if = "is_zero_nonce")]
    pub nonce: u64,
    #[serde(
        default,
        skip_serializing_if = "Bytes::is_empty",
        with = "crate::serde_utils::bytes"
    )]
    pub code: Bytes,
    #[serde(default, skip_serializing_if = "H256::is_zero")]
    pub code_hash: H256,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub storage: HashMap<H256, H256>,
}

impl PrestateAccountState {
    /// True when the entry conveys no information (no balance set, no nonce, no code,
    /// no code hash, no storage). Used to drop entries that would serialize to `{}`.
    /// A `Some(0)` balance is treated as no balance for emptiness purposes — it carries
    /// no diff information beyond what the absence of the field would imply.
    pub fn is_empty(&self) -> bool {
        self.balance.unwrap_or_default().is_zero()
            && self.nonce == 0
            && self.code.is_empty()
            && self.code_hash.is_zero()
            && self.storage.is_empty()
    }
}

/// Per-transaction prestate trace (non-diff mode).
/// Maps account address to its state before the transaction.
pub type PrestateTrace = HashMap<Address, PrestateAccountState>;

/// Result of a prestateTracer execution — either a plain prestate map or a diff.
#[derive(Debug, Clone)]
pub enum PrestateResult {
    /// Non-diff mode: map of address → pre-tx account state.
    Prestate(PrestateTrace),
    /// Diff mode: pre-tx and post-tx state for all touched accounts.
    Diff(PrePostState),
}

/// Per-transaction prestate trace (diff mode).
/// Contains the pre-tx and post-tx state for all touched accounts.
#[derive(Debug, Serialize, Default, Clone)]
pub struct PrePostState {
    pub pre: HashMap<Address, PrestateAccountState>,
    pub post: HashMap<Address, PrestateAccountState>,
}

fn is_zero_nonce(n: &u64) -> bool {
    *n == 0
}
