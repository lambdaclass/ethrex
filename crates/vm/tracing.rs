use bytes::Bytes;
use ethrex_common::{Address, U256};
use serde::Serialize;

/// Collection of traces of each call frame as defined in geth's `callTracer` output
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#call-tracer
pub type CallTrace = Vec<Call>;

/// Trace of each call frame as defined in geth's `callTracer` output
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#call-tracer
#[derive(Debug)]
pub struct Call {
    /// Type of the Call
    pub r#type: CallType,
    /// Address that initiated the call
    pub from: Address,
    /// Address that received the call
    pub to: Address,
    /// Amount transfered
    pub value: U256,
    /// Gas provided for the call
    pub gas: u64,
    /// Gas used by the call
    pub gas_used: u64,
    /// Call data
    pub input: Bytes,
    /// Return data
    pub output: Bytes,
    /// Error returned if the call failed
    pub error: Option<String>,
    /// Revert reason if the call reverted
    pub revert_reason: Option<String>,
    /// List of nested sub-calls
    pub calls: Box<Vec<Call>>,
}

// CALL, STATICCALL, DELEGATECALL, CREATE, CREATE2, SELFDESTRUCT -> Impl Serialize
#[derive(Serialize, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CallType {
    Call,
    StaticCall,
    DelegateCall,
    Create,
    Create2,
    SelfDestruct,
}
