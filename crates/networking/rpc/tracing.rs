use std::collections::HashMap;
use std::time::Duration;

use ethrex_common::{Address, H256};
use ethrex_common::{
    serde_utils,
    tracing::{CallTraceFrame, CallType, PrestateResult, StructLoggerEmit, StructLoggerResult},
};
use ethrex_vm::tracing::OpcodeTracerConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    rpc::RpcApiContext, rpc::RpcHandler, types::block_identifier::BlockIdentifier, utils::RpcErr,
};

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
    /// Multiplexer tracer that runs multiple sub-tracers on the same
    /// transaction and returns `{tracerName: result}` per sub-tracer.
    /// Selected via `"tracer": "muxTracer"`.
    ///
    /// **Divergences from geth**:
    /// - geth attaches all sub-tracers to a single execution; ethrex re-runs
    ///   the transaction once per sub-tracer (correct output, N× the cost).
    /// - `debug_traceBlockByNumber` with muxTracer is not yet supported and
    ///   returns a `BadParams` error.
    /// - Supported sub-tracers: `callTracer`, `prestateTracer`, `opcodeTracer`,
    ///   `4byteTracer`, `noopTracer`. Other registered tracers
    ///   (e.g. `flatCallTracer`) are not yet routable from inside muxTracer.
    MuxTracer,
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
            TracerType::MuxTracer => {
                let mux_config: HashMap<String, Value> =
                    if let Some(value) = &self.trace_config.tracer_config {
                        serde_json::from_value(value.clone())?
                    } else {
                        return Err(RpcErr::BadParams(
                            "muxTracer requires tracerConfig".to_owned(),
                        ));
                    };
                let mut results = serde_json::Map::new();
                // NOTE: each sub-tracer receives the full `timeout`, not a
                // proportional share — with N sub-tracers the wall-clock budget
                // is up to N × timeout. This matches geth (which also re-executes
                // per tracer in its JS tracer path) and is documented on MuxTracer.
                for (tracer_name, sub_config) in &mux_config {
                    let result = run_tx_sub_tracer(
                        tracer_name,
                        sub_config,
                        self.tx_hash,
                        reexec,
                        timeout,
                        &context,
                    )
                    .await?;
                    results.insert(tracer_name.clone(), result);
                }
                Ok(Value::Object(results))
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
            TracerType::MuxTracer => Err(RpcErr::BadParams(
                "muxTracer is not supported for debug_traceBlockByNumber yet \
                 — use debug_traceTransaction with muxTracer per transaction"
                    .to_owned(),
            )),
        }
    }
}

/// Runs a single sub-tracer for a transaction as part of a muxTracer request.
async fn run_tx_sub_tracer(
    tracer_name: &str,
    sub_config: &Value,
    tx_hash: H256,
    reexec: u32,
    timeout: Duration,
    context: &RpcApiContext,
) -> Result<Value, RpcErr> {
    match tracer_name {
        "callTracer" => {
            let config: CallTracerConfig = serde_json::from_value(sub_config.clone())?;
            let call_trace = context
                .blockchain
                .trace_transaction_calls(
                    tx_hash,
                    reexec,
                    timeout,
                    config.only_top_call,
                    config.with_log,
                )
                .await
                .map_err(|err| RpcErr::Internal(err.to_string()))?;
            let top_frame = call_trace
                .into_iter()
                .next()
                .ok_or(RpcErr::Internal("Empty call trace".to_string()))?;
            Ok(serde_json::to_value(top_frame)?)
        }
        "prestateTracer" => {
            let config: PrestateTracerConfig = serde_json::from_value(sub_config.clone())?;
            config.validate()?;
            let result = context
                .blockchain
                .trace_transaction_prestate(
                    tx_hash,
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
        "opcodeTracer" => {
            let cfg: OpcodeTracerConfig = serde_json::from_value(sub_config.clone())?;
            let emit = StructLoggerEmit {
                mem_size: cfg.enable_memory,
                return_data: cfg.enable_return_data,
                refund: false,
            };
            let result = context
                .blockchain
                .trace_transaction_opcodes(tx_hash, reexec, timeout, cfg)
                .await
                .map_err(|err| RpcErr::Internal(err.to_string()))?;
            Ok(serde_json::to_value(StructLoggerResult {
                result: &result,
                emit,
            })?)
        }
        "4byteTracer" => {
            // 4byteTracer accepts no configuration; any supplied sub_config is ignored.
            let call_trace = context
                .blockchain
                .trace_transaction_calls(tx_hash, reexec, timeout, false, false)
                .await
                .map_err(|err| RpcErr::Internal(err.to_string()))?;
            let top_frame = call_trace
                .into_iter()
                .next()
                .ok_or_else(|| RpcErr::Internal("Empty call trace".to_string()))?;
            let mut selectors = HashMap::new();
            collect_four_byte_selectors(&top_frame, &mut selectors);
            Ok(serde_json::to_value(selectors)?)
        }
        "noopTracer" => Ok(serde_json::to_value(serde_json::Map::new())?),
        unknown => Err(RpcErr::BadParams(format!("unknown sub-tracer: {unknown}"))),
    }
}

/// Collects 4-byte function selectors and calldata sizes from a call trace
/// tree, matching geth's built-in `4byteTracer`
/// (https://github.com/ethereum/go-ethereum/blob/master/eth/tracers/native/4byte.go):
///
/// - The top-level transaction call is **not** counted; only nested calls are.
/// - `CALL`, `DELEGATECALL`, `STATICCALL`, and `CALLCODE` are counted
///   (matching geth's `CaptureEnter`, which fires for all call types).
///   `CREATE`, `CREATE2`, and `SELFDESTRUCT` are skipped because their
///   input is init-code, not an ABI-encoded call.
/// - Invocations targeting precompile addresses are skipped.
/// - The reported size is `len(calldata) - 4` (the argument-bytes length).
fn collect_four_byte_selectors(top_frame: &CallTraceFrame, selectors: &mut HashMap<String, u64>) {
    for sub_call in &top_frame.calls {
        collect_four_byte_recursive(sub_call, selectors);
    }
}

fn collect_four_byte_recursive(frame: &CallTraceFrame, selectors: &mut HashMap<String, u64>) {
    if matches!(frame.call_type, CallType::CALL | CallType::DELEGATECALL | CallType::STATICCALL | CallType::CALLCODE)
        && frame.input.len() >= 4
        && !is_precompile_address(&frame.to)
    {
        let selector = hex::encode(&frame.input[..4]);
        let arg_size = frame.input.len() - 4;
        let key = format!("0x{selector}-{arg_size}");
        *selectors.entry(key).or_insert(0) += 1;
    }
    for sub_call in &frame.calls {
        collect_four_byte_recursive(sub_call, selectors);
    }
}

/// Fork-agnostic precompile address check used by `4byteTracer`. Treats any
/// address that maps to a precompile in some fork ethrex supports as a
/// precompile — slightly more aggressive than geth's per-fork check but
/// defensible: calldata to those addresses is never a function selector.
fn is_precompile_address(addr: &Address) -> bool {
    let bytes = addr.as_bytes();
    // L1 precompiles occupy 0x...01 through 0x...11 (BLAKE2F at 0x09, point
    // evaluation at 0x0a, BLS12 family up to 0x11).
    if bytes[..19].iter().all(|&b| b == 0) && (1..=0x11).contains(&bytes[19]) {
        return true;
    }
    // L2 P256VERIFY sits at 0x...0100.
    if bytes[..18].iter().all(|&b| b == 0) && bytes[18] == 0x01 && bytes[19] == 0x00 {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::RpcHandler;
    use bytes::Bytes;
    use serde_json::json;

    // --- TraceTransactionRequest parse tests ---

    #[test]
    fn parse_trace_tx_with_hash_only() {
        let params = Some(vec![json!(
            "0x0000000000000000000000000000000000000000000000000000000000000001"
        )]);
        let req = TraceTransactionRequest::parse(&params).unwrap();
        assert_eq!(req.tx_hash, H256::from_low_u64_be(1));
    }

    #[test]
    fn parse_trace_tx_with_config() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!({"tracer": "callTracer", "tracerConfig": {"onlyTopCall": true}}),
        ]);
        let req = TraceTransactionRequest::parse(&params).unwrap();
        assert_eq!(req.tx_hash, H256::from_low_u64_be(1));
        assert!(matches!(req.trace_config.tracer, TracerType::CallTracer));
    }

    #[test]
    fn parse_trace_tx_no_params() {
        let result = TraceTransactionRequest::parse(&None);
        assert!(result.is_err());
    }

    #[test]
    fn parse_trace_tx_too_many_params() {
        let params = Some(vec![json!("0x01"), json!({}), json!("extra")]);
        let result = TraceTransactionRequest::parse(&params);
        assert!(result.is_err());
    }

    #[test]
    fn parse_trace_tx_default_tracer_is_call_tracer() {
        let params = Some(vec![json!(
            "0x0000000000000000000000000000000000000000000000000000000000000001"
        )]);
        let req = TraceTransactionRequest::parse(&params).unwrap();
        assert!(matches!(req.trace_config.tracer, TracerType::CallTracer));
    }

    // --- TraceBlockByNumberRequest parse tests ---

    #[test]
    fn parse_trace_block_by_number_latest() {
        let params = Some(vec![json!("latest")]);
        let req = TraceBlockByNumberRequest::parse(&params).unwrap();
        assert!(matches!(
            req.block,
            BlockIdentifier::Tag(crate::types::block_identifier::BlockTag::Latest)
        ));
    }

    #[test]
    fn parse_trace_block_by_number_hex() {
        let params = Some(vec![json!("0xa")]);
        let req = TraceBlockByNumberRequest::parse(&params).unwrap();
        assert!(matches!(req.block, BlockIdentifier::Number(10)));
    }

    #[test]
    fn parse_trace_block_by_number_with_config() {
        let params = Some(vec![json!("0x1"), json!({"tracer": "prestateTracer"})]);
        let req = TraceBlockByNumberRequest::parse(&params).unwrap();
        assert!(matches!(
            req.trace_config.tracer,
            TracerType::PrestateTracer
        ));
    }

    #[test]
    fn parse_trace_block_by_number_no_params() {
        let result = TraceBlockByNumberRequest::parse(&None);
        assert!(result.is_err());
    }

    // --- TracerType deserialization tests ---

    #[test]
    fn deserialize_tracer_type_call_tracer() {
        let t: TracerType = serde_json::from_value(json!("callTracer")).unwrap();
        assert!(matches!(t, TracerType::CallTracer));
    }

    #[test]
    fn deserialize_tracer_type_prestate_tracer() {
        let t: TracerType = serde_json::from_value(json!("prestateTracer")).unwrap();
        assert!(matches!(t, TracerType::PrestateTracer));
    }

    #[test]
    fn deserialize_tracer_type_opcode_tracer() {
        let t: TracerType = serde_json::from_value(json!("opcodeTracer")).unwrap();
        assert!(matches!(t, TracerType::OpcodeTracer));
    }

    #[test]
    fn deserialize_tracer_type_mux_tracer() {
        let t: TracerType = serde_json::from_value(json!("muxTracer")).unwrap();
        assert!(matches!(t, TracerType::MuxTracer));
    }

    #[test]
    fn deserialize_tracer_type_unknown_fails() {
        let result = serde_json::from_value::<TracerType>(json!("unknownTracer"));
        assert!(result.is_err());
    }

    // --- TraceConfig deserialization tests ---

    #[test]
    fn deserialize_trace_config_defaults() {
        let cfg: TraceConfig = serde_json::from_value(json!({})).unwrap();
        assert!(matches!(cfg.tracer, TracerType::CallTracer));
        assert!(cfg.tracer_config.is_none());
        assert!(cfg.timeout.is_none());
        assert!(cfg.reexec.is_none());
    }

    #[test]
    fn deserialize_trace_config_with_timeout() {
        let cfg: TraceConfig = serde_json::from_value(json!({"timeout": "10s"})).unwrap();
        assert_eq!(cfg.timeout, Some(Duration::from_secs(10)));
    }

    #[test]
    fn deserialize_trace_config_with_reexec() {
        let cfg: TraceConfig = serde_json::from_value(json!({"reexec": 256})).unwrap();
        assert_eq!(cfg.reexec, Some(256));
    }

    // --- PrestateTracerConfig validation tests ---

    #[test]
    fn prestate_config_default_is_valid() {
        let cfg = PrestateTracerConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn prestate_config_diff_mode_only_is_valid() {
        let cfg = PrestateTracerConfig {
            diff_mode: true,
            include_empty: false,
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn prestate_config_include_empty_only_is_valid() {
        let cfg = PrestateTracerConfig {
            diff_mode: false,
            include_empty: true,
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn prestate_config_diff_mode_and_include_empty_is_invalid() {
        let cfg = PrestateTracerConfig {
            diff_mode: true,
            include_empty: true,
        };
        assert!(cfg.validate().is_err());
    }

    // --- collect_four_byte_selectors tests ---
    //
    // Helpers that build a top-level frame with `calls` children. The tracer
    // skips the top frame itself (matches geth's depth-0 skip), so the helper
    // makes that intent explicit in the test bodies.

    fn top_frame_with_calls(calls: Vec<CallTraceFrame>) -> CallTraceFrame {
        CallTraceFrame {
            call_type: CallType::CALL,
            input: Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef, 0xff]),
            calls,
            ..Default::default()
        }
    }

    fn collect(top: &CallTraceFrame) -> HashMap<String, u64> {
        let mut selectors = HashMap::new();
        collect_four_byte_selectors(top, &mut selectors);
        selectors
    }

    #[test]
    fn four_byte_skips_top_level_call() {
        let top = top_frame_with_calls(vec![]);
        assert!(collect(&top).is_empty());
    }

    #[test]
    fn four_byte_short_input_subcall_ignored() {
        let short = CallTraceFrame {
            call_type: CallType::CALL,
            input: Bytes::from_static(&[0xa9, 0x05, 0x9c]),
            ..Default::default()
        };
        assert!(collect(&top_frame_with_calls(vec![short])).is_empty());
    }

    #[test]
    fn four_byte_single_subcall_uses_arg_size() {
        let child = CallTraceFrame {
            call_type: CallType::CALL,
            input: Bytes::from_static(&[0xa9, 0x05, 0x9c, 0xbb, 0x00, 0x01]),
            ..Default::default()
        };
        let s = collect(&top_frame_with_calls(vec![child]));
        // 6 bytes total → selector + 2 arg bytes → key suffix "-2", not "-6".
        assert_eq!(s.len(), 1);
        assert_eq!(s["0xa9059cbb-2"], 1);
    }

    #[test]
    fn four_byte_nested_subcalls() {
        let grandchild = CallTraceFrame {
            call_type: CallType::CALL,
            input: Bytes::from_static(&[0x23, 0xb8, 0x72, 0xdd, 0x01, 0x02, 0x03]),
            ..Default::default()
        };
        let child = CallTraceFrame {
            call_type: CallType::CALL,
            input: Bytes::from_static(&[0xa9, 0x05, 0x9c, 0xbb, 0xaa]),
            calls: vec![grandchild],
            ..Default::default()
        };
        let s = collect(&top_frame_with_calls(vec![child]));
        assert_eq!(s.len(), 2);
        assert_eq!(s["0xa9059cbb-1"], 1);
        assert_eq!(s["0x23b872dd-3"], 1);
    }

    #[test]
    fn four_byte_duplicate_subcalls_counted() {
        let mk = || CallTraceFrame {
            call_type: CallType::CALL,
            input: Bytes::from_static(&[0xa9, 0x05, 0x9c, 0xbb, 0xaa]),
            ..Default::default()
        };
        let s = collect(&top_frame_with_calls(vec![mk(), mk()]));
        assert_eq!(s.len(), 1);
        assert_eq!(s["0xa9059cbb-1"], 2);
    }

    #[test]
    fn four_byte_counts_all_call_types_except_create_and_selfdestruct() {
        let mk_with = |call_type: CallType| CallTraceFrame {
            call_type,
            input: Bytes::from_static(&[0xa9, 0x05, 0x9c, 0xbb, 0x01]),
            ..Default::default()
        };
        let top = top_frame_with_calls(vec![
            mk_with(CallType::CALL),
            mk_with(CallType::DELEGATECALL),
            mk_with(CallType::STATICCALL),
            mk_with(CallType::CALLCODE),
            mk_with(CallType::CREATE),
            mk_with(CallType::CREATE2),
            mk_with(CallType::SELFDESTRUCT),
        ]);
        let s = collect(&top);
        // CALL + DELEGATECALL + STATICCALL + CALLCODE = 4 hits.
        assert_eq!(s.len(), 1);
        assert_eq!(s["0xa9059cbb-1"], 4);
    }

    #[test]
    fn four_byte_skips_precompile_targets() {
        let mk = |addr: Address| CallTraceFrame {
            call_type: CallType::CALL,
            to: addr,
            input: Bytes::from_static(&[0xa9, 0x05, 0x9c, 0xbb, 0x01]),
            ..Default::default()
        };
        let top = top_frame_with_calls(vec![
            mk(Address::from_low_u64_be(0x01)),  // ECRECOVER
            mk(Address::from_low_u64_be(0x09)),  // BLAKE2F
            mk(Address::from_low_u64_be(0x0a)),  // POINT_EVALUATION
            mk(Address::from_low_u64_be(0x11)),  // BLS12_MAP_FP2_TO_G2
            mk(Address::from_low_u64_be(0x100)), // P256VERIFY (L2)
        ]);
        assert!(collect(&top).is_empty());
    }

    // --- MuxTracer parse tests ---

    #[test]
    fn parse_trace_tx_mux_tracer() {
        let params = Some(vec![
            json!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            json!({
                "tracer": "muxTracer",
                "tracerConfig": {
                    "callTracer": {},
                    "noopTracer": {}
                }
            }),
        ]);
        let req = TraceTransactionRequest::parse(&params).unwrap();
        assert!(matches!(req.trace_config.tracer, TracerType::MuxTracer));
        assert!(req.trace_config.tracer_config.is_some());
    }
}
