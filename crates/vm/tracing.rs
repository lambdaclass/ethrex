use bytes::Bytes;
use ethrex_common::{types::Block, Address, U256};
use serde::Serialize;

use crate::{
    backends::revm::{db::EvmState, REVM},
    Evm, EvmError,
};

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

impl Evm {
    /// Executes the block until a given tx is reached, then generates the call trace for the tx
    /// Wraps [REVM::trace_tx_calls], does not currenlty have levm support.
    pub fn trace_tx_calls(
        &mut self,
        block: &Block,
        tx_index: usize,
    ) -> Result<CallTrace, EvmError> {
        match self {
            Evm::REVM { state } => REVM::trace_tx_calls(block, tx_index, state),
            Evm::LEVM { db: _ } => {
                // Tracing is not implemented for levm
                return Err(EvmError::Custom(
                    "Transaction Tracing not supported for LEVM".to_string(),
                ));
            }
        }
    }

    /// Reruns the given block, saving the changes on the state, doesn't output any results or receipts
    /// Wraps [REVM::rerun_block], does not currenlty have levm support.
    pub fn rerun_block(&mut self, block: &Block) -> Result<(), EvmError> {
        match self {
            Evm::REVM { state } => REVM::rerun_block(block, state),
            Evm::LEVM { db: _ } => {
                // Tracing is not implemented for levm
                return Err(EvmError::Custom(
                    "Block Rerun not supported for LEVM".to_string(),
                ));
            }
        }
    }
}
