use std::time::Duration;

use bytes::Bytes;
use ethrex_common::{Address, H256, U256};
use ethrex_common::{
    serde_utils,
    tracing::{CallTraceFrame, CallType, PrestateResult, StructLoggerEmit, StructLoggerResult},
};
use ethrex_vm::tracing::OpcodeTracerConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{rpc::RpcHandler, types::block_identifier::BlockIdentifier, utils::RpcErr};

/// Default max amount of blocks to re-excute if it is not given
const DEFAULT_REEXEC: u32 = 128;
/// Default max amount of time to spend tracing a transaction (doesn't take into account state rebuild time)
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct TraceTransactionRequest {
    tx_hash: H256,
    trace_config: TraceConfig,
}

pub struct TraceBlockByNumberRequest {
    block: BlockIdentifier,
    trace_config: TraceConfig,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TraceConfig {
    #[serde(default)]
    tracer: TracerType,
    // This differs for each different tracer so we will parse it afterwards when we know the type
    #[serde(default)]
    tracer_config: Option<Value>,
    #[serde(default, with = "serde_utils::duration::opt")]
    timeout: Option<Duration>,
    #[serde(default)]
    reexec: Option<u32>,
}

/// The tracer variant to use for a debug trace request.
///
/// **Divergence from geth**: geth's default (when no `tracer` field is provided) is the
/// per-opcode tracer. ethrex keeps `CallTracer` as the default for compatibility with
/// Blockscout-style clients that rely on the no-tracer-specified → callTracer behaviour.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
// The wire-format names (`callTracer`, `prestateTracer`, `opcodeTracer`) are
// fixed by client convention; variants must keep the `Tracer` suffix to
// serialize correctly via `rename_all = "camelCase"`.
#[allow(clippy::enum_variant_names)]
enum TracerType {
    #[default]
    CallTracer,
    PrestateTracer,
    /// Per-opcode tracer emitting EIP-3155 step content under the de-facto
    /// `structLogger` wrapper shape (`{failed, gas, returnValue, structLogs}`).
    /// Selected via `"tracer": "opcodeTracer"`.
    OpcodeTracer,
    /// Flat call tracer matching geth's built-in `flatCallTracer`: a flat array
    /// of call frames with `traceAddress` and `subtraces`, following the
    /// Parity/OpenEthereum shape plus geth's `creationMethod` field on
    /// `create` actions. Selected via `"tracer": "flatCallTracer"`.
    FlatCallTracer,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CallTracerConfig {
    #[serde(default)]
    only_top_call: bool,
    #[serde(default)]
    with_log: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PrestateTracerConfig {
    #[serde(default)]
    diff_mode: bool,
    #[serde(default)]
    include_empty: bool,
}

impl PrestateTracerConfig {
    fn validate(&self) -> Result<(), RpcErr> {
        if self.diff_mode && self.include_empty {
            return Err(RpcErr::BadParams(
                "cannot use diffMode with includeEmpty".to_string(),
            ));
        }
        Ok(())
    }
}

type BlockTrace<TxTrace> = Vec<BlockTraceComponent<TxTrace>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BlockTraceComponent<TxTrace: Serialize> {
    tx_hash: H256,
    result: TxTrace,
}

impl<TxTrace: Serialize> From<(H256, TxTrace)> for BlockTraceComponent<TxTrace> {
    fn from(value: (H256, TxTrace)) -> Self {
        BlockTraceComponent {
            tx_hash: value.0,
            result: value.1,
        }
    }
}

impl RpcHandler for TraceTransactionRequest {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 && params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 1 or 2 params".to_owned()));
        };
        let trace_config = if params.len() == 2 {
            serde_json::from_value(params[1].clone())?
        } else {
            TraceConfig::default()
        };

        Ok(TraceTransactionRequest {
            tx_hash: serde_json::from_value(params[0].clone())?,
            trace_config,
        })
    }

    async fn handle(
        &self,
        context: crate::rpc::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        let reexec = self.trace_config.reexec.unwrap_or(DEFAULT_REEXEC);
        let timeout = self.trace_config.timeout.unwrap_or(DEFAULT_TIMEOUT);
        match self.trace_config.tracer {
            TracerType::CallTracer => {
                // Parse tracer config now that we know the type
                let config = if let Some(value) = &self.trace_config.tracer_config {
                    serde_json::from_value(value.clone())?
                } else {
                    CallTracerConfig::default()
                };
                let call_trace = context
                    .blockchain
                    .trace_transaction_calls(
                        self.tx_hash,
                        reexec,
                        timeout,
                        config.only_top_call,
                        config.with_log,
                    )
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                // Geth returns a single CallTraceFrame object, not an array.
                // Blockscout expects this format for internal transaction indexing.
                let top_frame = call_trace
                    .into_iter()
                    .next()
                    .ok_or(RpcErr::Internal("Empty call trace".to_string()))?;
                Ok(serde_json::to_value(top_frame)?)
            }
            TracerType::PrestateTracer => {
                let config: PrestateTracerConfig =
                    if let Some(value) = &self.trace_config.tracer_config {
                        serde_json::from_value(value.clone())?
                    } else {
                        PrestateTracerConfig::default()
                    };
                config.validate()?;
                let result = context
                    .blockchain
                    .trace_transaction_prestate(
                        self.tx_hash,
                        reexec,
                        timeout,
                        config.diff_mode,
                        config.include_empty,
                    )
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                match result {
                    PrestateResult::Prestate(trace) => Ok(serde_json::to_value(trace)?),
                    PrestateResult::Diff(diff) => Ok(serde_json::to_value(diff)?),
                }
            }
            TracerType::OpcodeTracer => {
                let cfg: OpcodeTracerConfig = self
                    .trace_config
                    .tracer_config
                    .as_ref()
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()?
                    .unwrap_or_default();
                let emit = StructLoggerEmit {
                    mem_size: cfg.enable_memory,
                    return_data: cfg.enable_return_data,
                    refund: false,
                };
                let result = context
                    .blockchain
                    .trace_transaction_opcodes(self.tx_hash, reexec, timeout, cfg)
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                // `debug_traceTransaction` returns the geth-RPC structLogger shape.
                Ok(serde_json::to_value(StructLoggerResult {
                    result: &result,
                    emit,
                })?)
            }
            TracerType::FlatCallTracer => {
                let call_trace = context
                    .blockchain
                    .trace_transaction_calls(
                        self.tx_hash,
                        reexec,
                        timeout,
                        false, // need all subcalls
                        false,
                    )
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                let top_frame = call_trace
                    .into_iter()
                    .next()
                    .ok_or(RpcErr::Internal("Empty call trace".to_string()))?;
                let flat_frames = flatten_call_trace(&top_frame);
                Ok(serde_json::to_value(flat_frames)?)
            }
        }
    }
}

impl RpcHandler for TraceBlockByNumberRequest {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 && params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 1 or 2 params".to_owned()));
        };
        let trace_config = if params.len() == 2 {
            serde_json::from_value(params[1].clone())?
        } else {
            TraceConfig::default()
        };

        let block = BlockIdentifier::parse(params[0].clone(), 0)?;

        Ok(TraceBlockByNumberRequest {
            block,
            trace_config,
        })
    }

    async fn handle(
        &self,
        context: crate::rpc::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        let block_number = self
            .block
            .resolve_block_number(&context.storage)
            .await?
            .ok_or(RpcErr::Internal("Block not Found".to_string()))?;
        let block = context
            .storage
            .get_block_by_number(block_number)
            .await?
            .ok_or(RpcErr::Internal("Block not Found".to_string()))?;
        let reexec = self.trace_config.reexec.unwrap_or(DEFAULT_REEXEC);
        let timeout = self.trace_config.timeout.unwrap_or(DEFAULT_TIMEOUT);
        match self.trace_config.tracer {
            TracerType::CallTracer => {
                // Parse tracer config now that we know the type
                let config = if let Some(value) = &self.trace_config.tracer_config {
                    serde_json::from_value(value.clone())?
                } else {
                    CallTracerConfig::default()
                };
                let call_traces = context
                    .blockchain
                    .trace_block_calls(
                        block,
                        reexec,
                        timeout,
                        config.only_top_call,
                        config.with_log,
                    )
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                // Unwrap each CallTrace (Vec<CallTraceFrame>) to a single
                // CallTraceFrame to match geth's callTracer response format.
                let block_trace: BlockTrace<CallTraceFrame> = call_traces
                    .into_iter()
                    .map(|(hash, trace)| {
                        let frame = trace
                            .into_iter()
                            .next()
                            .ok_or_else(|| RpcErr::Internal("Empty call trace".to_string()))?;
                        Ok((hash, frame).into())
                    })
                    .collect::<Result<_, RpcErr>>()?;
                Ok(serde_json::to_value(block_trace)?)
            }
            TracerType::PrestateTracer => {
                let config: PrestateTracerConfig =
                    if let Some(value) = &self.trace_config.tracer_config {
                        serde_json::from_value(value.clone())?
                    } else {
                        PrestateTracerConfig::default()
                    };
                config.validate()?;
                let prestate_traces = context
                    .blockchain
                    .trace_block_prestate(
                        block,
                        reexec,
                        timeout,
                        config.diff_mode,
                        config.include_empty,
                    )
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                // Each trace result is already the correct variant (Prestate or Diff)
                // based on the diff_mode flag, so we serialize directly.
                let block_trace: Vec<serde_json::Value> = prestate_traces
                    .into_iter()
                    .map(|(hash, result)| {
                        let trace_value = match result {
                            PrestateResult::Prestate(trace) => serde_json::to_value(trace)?,
                            PrestateResult::Diff(diff) => serde_json::to_value(diff)?,
                        };
                        serde_json::to_value(BlockTraceComponent {
                            tx_hash: hash,
                            result: trace_value,
                        })
                    })
                    .collect::<Result<_, serde_json::Error>>()?;
                Ok(serde_json::to_value(block_trace)?)
            }
            TracerType::OpcodeTracer => {
                let cfg: OpcodeTracerConfig = self
                    .trace_config
                    .tracer_config
                    .as_ref()
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()?
                    .unwrap_or_default();
                let emit = StructLoggerEmit {
                    mem_size: cfg.enable_memory,
                    return_data: cfg.enable_return_data,
                    refund: false,
                };
                let opcode_traces = context
                    .blockchain
                    .trace_block_opcodes(block, reexec, timeout, cfg)
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                // Wrap each result with StructLoggerResult so it serializes in the
                // geth-RPC shape expected by `debug_traceBlockByNumber` consumers.
                let block_trace: Vec<serde_json::Value> = opcode_traces
                    .into_iter()
                    .map(|(hash, result)| {
                        let wrapped = serde_json::to_value(StructLoggerResult {
                            result: &result,
                            emit,
                        })?;
                        serde_json::to_value(BlockTraceComponent {
                            tx_hash: hash,
                            result: wrapped,
                        })
                    })
                    .collect::<Result<_, serde_json::Error>>()?;
                Ok(serde_json::to_value(block_trace)?)
            }
            TracerType::FlatCallTracer => {
                let call_traces = context
                    .blockchain
                    .trace_block_calls(block, reexec, timeout, false, false)
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                let block_trace: BlockTrace<Vec<FlatCallFrame>> = call_traces
                    .into_iter()
                    .map(|(hash, trace)| {
                        let frame = trace
                            .into_iter()
                            .next()
                            .ok_or_else(|| RpcErr::Internal("Empty call trace".to_string()))?;
                        let flat_frames = flatten_call_trace(&frame);
                        Ok((hash, flat_frames).into())
                    })
                    .collect::<Result<_, RpcErr>>()?;
                Ok(serde_json::to_value(block_trace)?)
            }
        }
    }
}

// ── flatCallTracer types and helpers ─────────────────────────────────────
//
// Output mirrors geth's built-in `flatCallTracer`
// (https://github.com/ethereum/go-ethereum/blob/master/eth/tracers/native/call_flat.go),
// which itself follows the Parity/OpenEthereum trace shape but adds a
// `creationMethod` field on `create` frames. Three variants exist:
//
// - `type: "call"`  → action has callType/from/gas/input/to/value,
//                     result has gasUsed/output
// - `type: "create"`→ action has creationMethod/from/gas/init/value,
//                     result has address/code/gasUsed
// - `type: "suicide"`→ action has address/balance/refundAddress, no result

/// A single flattened call frame.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FlatCallFrame {
    action: FlatCallAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<FlatCallResult>,
    subtraces: usize,
    trace_address: Vec<usize>,
    #[serde(rename = "type")]
    frame_type: &'static str,
}

/// Per-variant action object. The outer `type` field disambiguates which shape
/// is in use; `#[serde(untagged)]` emits the inner fields directly without a
/// discriminator key, matching geth.
#[derive(Serialize)]
#[serde(untagged)]
enum FlatCallAction {
    #[serde(rename_all = "camelCase")]
    Call {
        call_type: &'static str,
        from: Address,
        #[serde(with = "serde_utils::u64::hex_str")]
        gas: u64,
        #[serde(with = "serde_utils::bytes")]
        input: Bytes,
        to: Address,
        value: U256,
    },
    #[serde(rename_all = "camelCase")]
    Create {
        creation_method: &'static str,
        from: Address,
        #[serde(with = "serde_utils::u64::hex_str")]
        gas: u64,
        #[serde(with = "serde_utils::bytes")]
        init: Bytes,
        value: U256,
    },
    #[serde(rename_all = "camelCase")]
    Suicide {
        address: Address,
        balance: U256,
        refund_address: Address,
    },
}

#[derive(Serialize)]
#[serde(untagged)]
enum FlatCallResult {
    #[serde(rename_all = "camelCase")]
    Call {
        #[serde(with = "serde_utils::u64::hex_str")]
        gas_used: u64,
        #[serde(with = "serde_utils::bytes")]
        output: Bytes,
    },
    #[serde(rename_all = "camelCase")]
    Create {
        address: Address,
        #[serde(with = "serde_utils::bytes")]
        code: Bytes,
        #[serde(with = "serde_utils::u64::hex_str")]
        gas_used: u64,
    },
}

/// Flattens a nested `CallTraceFrame` tree into a flat array with
/// `traceAddress` and `subtraces` fields. Maximum recursion depth is bounded
/// by the EVM call depth limit (1024), so stack usage is safe.
fn flatten_call_trace(root: &CallTraceFrame) -> Vec<FlatCallFrame> {
    let mut result = Vec::new();
    flatten_recursive(root, &[], &mut result);
    result
}

fn flatten_recursive(
    frame: &CallTraceFrame,
    trace_address: &[usize],
    result: &mut Vec<FlatCallFrame>,
) {
    let frame_type: &'static str = match frame.call_type {
        CallType::CALL | CallType::CALLCODE | CallType::STATICCALL | CallType::DELEGATECALL => {
            "call"
        }
        CallType::CREATE | CallType::CREATE2 => "create",
        CallType::SELFDESTRUCT => "suicide",
    };

    let action = match frame.call_type {
        CallType::CALL | CallType::CALLCODE | CallType::STATICCALL | CallType::DELEGATECALL => {
            FlatCallAction::Call {
                call_type: match frame.call_type {
                    CallType::CALL => "call",
                    CallType::CALLCODE => "callcode",
                    CallType::STATICCALL => "staticcall",
                    CallType::DELEGATECALL => "delegatecall",
                    _ => unreachable!(),
                },
                from: frame.from,
                gas: frame.gas,
                input: frame.input.clone(),
                to: frame.to,
                value: frame.value,
            }
        }
        CallType::CREATE | CallType::CREATE2 => FlatCallAction::Create {
            creation_method: if matches!(frame.call_type, CallType::CREATE) {
                "create"
            } else {
                "create2"
            },
            from: frame.from,
            gas: frame.gas,
            init: frame.input.clone(),
            value: frame.value,
        },
        // SELFDESTRUCT: `from` is the destructed contract, `to` is the refund
        // address, `value` is the balance forwarded — match those onto Parity's
        // suicide action shape.
        CallType::SELFDESTRUCT => FlatCallAction::Suicide {
            address: frame.from,
            balance: frame.value,
            refund_address: frame.to,
        },
    };

    // Suicide frames never carry a result; failed frames omit it too (the
    // failure is surfaced via the `error` field).
    let result_value = if frame.error.is_some() || matches!(frame.call_type, CallType::SELFDESTRUCT)
    {
        None
    } else {
        Some(match frame.call_type {
            CallType::CREATE | CallType::CREATE2 => FlatCallResult::Create {
                address: frame.to,
                code: frame.output.clone(),
                gas_used: frame.gas_used,
            },
            _ => FlatCallResult::Call {
                gas_used: frame.gas_used,
                output: frame.output.clone(),
            },
        })
    };

    // SELFDESTRUCT frames are always leaves in the call tree — the EVM cannot
    // execute further code after self-destructing. Assert this invariant so
    // that a future tracer bug doesn't silently produce malformed output.
    debug_assert!(
        !matches!(frame.call_type, CallType::SELFDESTRUCT) || frame.calls.is_empty(),
        "SELFDESTRUCT frame must be a leaf (no children), got {} subcalls",
        frame.calls.len(),
    );

    result.push(FlatCallFrame {
        action,
        error: frame.error.clone(),
        result: result_value,
        subtraces: frame.calls.len(),
        trace_address: trace_address.to_vec(),
        frame_type,
    });

    for (i, sub_call) in frame.calls.iter().enumerate() {
        let mut child_address = trace_address.to_vec();
        child_address.push(i);
        flatten_recursive(sub_call, &child_address, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::RpcHandler;
    use serde_json::json;

    // --- TracerType deserialization tests ---

    #[test]
    fn deserialize_tracer_type_flat_call() {
        let t: TracerType = serde_json::from_value(json!("flatCallTracer")).unwrap();
        assert!(matches!(t, TracerType::FlatCallTracer));
    }

    #[test]
    fn deserialize_tracer_type_unknown_fails() {
        assert!(serde_json::from_value::<TracerType>(json!("unknownTracer")).is_err());
    }

    // --- flatCallTracer parse test ---

    #[test]
    fn parse_trace_tx_flat_call_tracer() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!({"tracer": "flatCallTracer"}),
        ]);
        let req = TraceTransactionRequest::parse(&params).unwrap();
        assert!(matches!(
            req.trace_config.tracer,
            TracerType::FlatCallTracer
        ));
    }

    // --- flatten_call_trace tests ---
    //
    // The Serialize-only types are inspected via their JSON projection so we
    // verify the wire shape clients actually receive.

    fn flat_json(frame: &CallTraceFrame) -> Vec<serde_json::Value> {
        flatten_call_trace(frame)
            .iter()
            .map(|f| serde_json::to_value(f).unwrap())
            .collect()
    }

    #[test]
    fn flatten_single_call_frame() {
        let frame = CallTraceFrame {
            call_type: CallType::CALL,
            from: Address::zero(),
            to: Address::from_low_u64_be(1),
            gas: 21000,
            gas_used: 21000,
            ..Default::default()
        };
        let frames = flat_json(&frame);
        assert_eq!(frames.len(), 1);
        let f = &frames[0];
        assert_eq!(f["type"], "call");
        assert_eq!(f["traceAddress"], json!([]));
        assert_eq!(f["subtraces"], 0);
        assert_eq!(f["action"]["callType"], "call");
        assert_eq!(
            f["action"]["to"],
            format!("{:#x}", Address::from_low_u64_be(1))
        );
        // Call results carry gasUsed + output, not address/code.
        assert!(f["result"]["gasUsed"].is_string());
        assert!(f["result"]["output"].is_string());
        assert!(f["result"].get("address").is_none());
    }

    #[test]
    fn flatten_nested_frames_pre_order_with_trace_address() {
        let grandchild = CallTraceFrame {
            call_type: CallType::STATICCALL,
            from: Address::from_low_u64_be(2),
            to: Address::from_low_u64_be(3),
            ..Default::default()
        };
        let child = CallTraceFrame {
            call_type: CallType::DELEGATECALL,
            from: Address::from_low_u64_be(1),
            to: Address::from_low_u64_be(2),
            calls: vec![grandchild],
            ..Default::default()
        };
        let root = CallTraceFrame {
            call_type: CallType::CALL,
            from: Address::zero(),
            to: Address::from_low_u64_be(1),
            calls: vec![child],
            ..Default::default()
        };
        let frames = flat_json(&root);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0]["traceAddress"], json!([]));
        assert_eq!(frames[0]["subtraces"], 1);
        assert_eq!(frames[1]["traceAddress"], json!([0]));
        assert_eq!(frames[1]["action"]["callType"], "delegatecall");
        assert_eq!(frames[1]["subtraces"], 1);
        assert_eq!(frames[2]["traceAddress"], json!([0, 0]));
        assert_eq!(frames[2]["action"]["callType"], "staticcall");
        assert_eq!(frames[2]["subtraces"], 0);
    }

    #[test]
    fn flatten_create_emits_init_and_address_code_result() {
        let deployed_addr = Address::from_low_u64_be(0x42);
        let frame = CallTraceFrame {
            call_type: CallType::CREATE,
            from: Address::zero(),
            to: deployed_addr,
            gas: 50000,
            gas_used: 32000,
            input: Bytes::from_static(b"init"),
            output: Bytes::from_static(&[0xfe, 0xfe, 0xfe]),
            ..Default::default()
        };
        let frames = flat_json(&frame);
        let f = &frames[0];
        assert_eq!(f["type"], "create");
        assert_eq!(f["action"]["creationMethod"], "create");
        // The init code lives under `init`, not `input`.
        assert!(
            f["action"].get("input").is_none(),
            "create action must use `init`, not `input`"
        );
        assert!(f["action"]["init"].is_string());
        // `to` does not appear on create actions.
        assert!(f["action"].get("to").is_none());
        // Result carries address + code, not output.
        assert_eq!(f["result"]["address"], format!("{deployed_addr:#x}"));
        assert!(f["result"]["code"].is_string());
        assert!(f["result"].get("output").is_none());
        assert!(f["result"]["gasUsed"].is_string());
    }

    #[test]
    fn flatten_create2_uses_create2_method() {
        let frame = CallTraceFrame {
            call_type: CallType::CREATE2,
            ..Default::default()
        };
        let frames = flat_json(&frame);
        assert_eq!(frames[0]["action"]["creationMethod"], "create2");
    }

    #[test]
    fn flatten_selfdestruct_uses_suicide_shape() {
        let destructed = Address::from_low_u64_be(0xaa);
        let beneficiary = Address::from_low_u64_be(0xbb);
        let balance = U256::from(123_456u64);
        let frame = CallTraceFrame {
            call_type: CallType::SELFDESTRUCT,
            from: destructed,
            to: beneficiary,
            value: balance,
            ..Default::default()
        };
        let frames = flat_json(&frame);
        let f = &frames[0];
        assert_eq!(f["type"], "suicide");
        // Action shape is {address, balance, refundAddress} — nothing from the
        // call shape leaks through.
        assert_eq!(f["action"]["address"], format!("{destructed:#x}"));
        assert_eq!(f["action"]["refundAddress"], format!("{beneficiary:#x}"));
        assert!(f["action"]["balance"].is_string());
        assert!(f["action"].get("from").is_none());
        assert!(f["action"].get("to").is_none());
        assert!(f["action"].get("callType").is_none());
        assert!(f["action"].get("input").is_none());
        // Suicide frames never carry a result.
        assert!(f.get("result").is_none() || f["result"].is_null());
    }

    #[test]
    fn flatten_failed_call_omits_result_and_keeps_error() {
        let frame = CallTraceFrame {
            call_type: CallType::CALL,
            from: Address::zero(),
            to: Address::from_low_u64_be(1),
            error: Some("out of gas".to_string()),
            ..Default::default()
        };
        let frames = flat_json(&frame);
        let f = &frames[0];
        assert_eq!(f["error"], "out of gas");
        assert!(f.get("result").is_none() || f["result"].is_null());
    }
}
