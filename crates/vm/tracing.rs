use bytes::Bytes;
use ethrex_common::{serde_utils, H256};
use ethrex_common::{types::Block, Address, U256};
use serde::Serialize;

use crate::backends::levm::LEVM;
use crate::{backends::revm::REVM, Evm, EvmError};

/// Collection of traces of each call frame as defined in geth's `callTracer` output
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#call-tracer
pub type CallTrace = Vec<Call>;

/// Trace of each call frame as defined in geth's `callTracer` output
/// https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#call-tracer
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
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
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas: u64,
    /// Gas used by the call
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas_used: u64,
    /// Call data
    #[serde(with = "serde_utils::bytes")]
    pub input: Bytes,
    /// Return data
    #[serde(with = "serde_utils::bytes")]
    pub output: Bytes,
    /// Error returned if the call failed
    pub error: Option<String>,
    /// Revert reason if the call reverted
    pub revert_reason: Option<String>,
    /// List of nested sub-calls
    pub calls: Vec<Call>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    /// Logs (if enabled)
    pub logs: Vec<CallLog>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "UPPERCASE")]
pub enum CallType {
    Call,
    CallCode,
    StaticCall,
    DelegateCall,
    Create,
    Create2,
    SelfDestruct,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CallLog {
    pub address: Address,
    pub topics: Vec<H256>,
    #[serde(with = "serde_utils::bytes")]
    pub data: Bytes,
    pub position: u64,
}

impl Evm {
    /// Runs a single tx with the call tracer and outputs its trace
    /// Asumes that the received state already contains changes from previous blocks and other
    /// transactions within its block
    /// Wraps [REVM::trace_tx_calls], does not currenlty have levm support.
    pub fn trace_tx_calls(
        &mut self,
        block: &Block,
        tx_index: usize,
        only_top_call: bool,
        with_log: bool,
    ) -> Result<CallTrace, EvmError> {
        match self {
            Evm::REVM { state } => {
                REVM::trace_tx_calls(block, tx_index, state, only_top_call, with_log)
            }
            Evm::LEVM { db } => LEVM::trace_tx_calls(db, block, tx_index, only_top_call, with_log),
        }
    }

    /// Reruns the given block, saving the changes on the state, doesn't output any results or receipts
    /// If the optional argument `stop_index` is set, the run will stop just before executing the transaction at that index
    /// and won't process the withdrawals afterwards
    /// Wraps [REVM::rerun_block], does not currenlty have levm support.
    pub fn rerun_block(
        &mut self,
        block: &Block,
        stop_index: Option<usize>,
    ) -> Result<(), EvmError> {
        match self {
            Evm::REVM { state } => REVM::rerun_block(block, state, stop_index),
            Evm::LEVM { db } => LEVM::rerun_block(db, block, stop_index),
        }
    }
}
