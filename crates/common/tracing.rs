use bytes::Bytes;
use ethereum_types::H256;
use ethereum_types::{Address, U256};
use serde::Serialize;

/// Collection of traces of each call frame as defined in geth's `callTracer` output
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#call-tracer
pub type CallTrace = Vec<TracingCallframe>;

/// Trace of each call frame as defined in geth's `callTracer` output
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#call-tracer
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TracingCallframe {
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
    pub calls: Vec<TracingCallframe>,
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

/// These methods below are implemented for LEVM tracing.
/// If necessary they could be removed and be implemented as functions in levm crate.
impl TracingCallframe {
    pub fn new(
        r#type: CallType,
        from: Address,
        to: Address,
        value: U256,
        gas: u64,
        input: Bytes,
    ) -> Self {
        Self {
            call_type: r#type,
            from,
            to,
            value,
            gas,
            input,
            ..Default::default()
        }
    }

    pub fn process_output(
        &mut self,
        gas_used: u64,
        output: Bytes,
        error: Option<String>,
        revert_reason: Option<String>,
    ) {
        self.gas_used = gas_used;
        self.output = output;
        self.error = error;
        self.revert_reason = revert_reason;
    }

    /// Clear logs of callframe if it reverted and repeat the same with its subcalls.
    pub fn clear_reverted_logs(&mut self) {
        if self.error.is_some() {
            self.logs.clear();
        }
        for subcall in &mut self.calls {
            subcall.clear_reverted_logs();
        }
    }
}
