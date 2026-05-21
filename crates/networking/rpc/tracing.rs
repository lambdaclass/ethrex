use std::collections::HashMap;
use std::time::Duration;

use ethrex_common::H256;
use ethrex_common::{
    serde_utils,
    tracing::{CallTraceFrame, PrestateResult, StructLoggerEmit, StructLoggerResult},
};
use ethrex_vm::tracing::OpcodeTracerConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{rpc::RpcApiContext, rpc::RpcHandler, types::block_identifier::BlockIdentifier, utils::RpcErr};

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
    /// Multiplexer tracer that runs multiple sub-tracers on the same transaction
    /// and returns a map of `tracerName -> result`.
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
            TracerType::MuxTracer => {
                Err(RpcErr::Internal(
                    "muxTracer is not supported for block tracing".to_owned(),
                ))
            }
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
            let call_trace = context
                .blockchain
                .trace_transaction_calls(tx_hash, reexec, timeout, false, false)
                .await
                .map_err(|err| RpcErr::Internal(err.to_string()))?;
            let top_frame = call_trace
                .into_iter()
                .next()
                .ok_or(RpcErr::Internal("Empty call trace".to_string()))?;
            let mut selectors = HashMap::new();
            collect_four_byte_selectors(&top_frame, &mut selectors);
            Ok(serde_json::to_value(selectors)?)
        }
        "noopTracer" => Ok(serde_json::to_value(serde_json::Map::new())?),
        unknown => Err(RpcErr::BadParams(format!(
            "unknown sub-tracer: {unknown}"
        ))),
    }
}

/// Recursively collects 4-byte function selectors and calldata sizes from a call trace tree.
fn collect_four_byte_selectors(frame: &CallTraceFrame, selectors: &mut HashMap<String, u64>) {
    if frame.input.len() >= 4 {
        let selector = hex::encode(&frame.input[..4]);
        let key = format!("0x{selector}-{}", frame.input.len());
        *selectors.entry(key).or_insert(0) += 1;
    }
    for sub_call in &frame.calls {
        collect_four_byte_selectors(sub_call, selectors);
    }
}
