use std::time::Duration;

use ethrex_common::H256;
use ethrex_common::types::{Block, BlockHash, GenericTransaction};
use ethrex_common::{
    serde_utils,
    tracing::{CallTraceFrame, PrestateResult, StructLoggerEmit, StructLoggerResult},
};
use ethrex_vm::tracing::OpcodeTracerConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
    utils::RpcErr,
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

pub struct TraceBlockByHashRequest {
    block_hash: BlockHash,
    trace_config: TraceConfig,
}

pub struct TraceCallRequest {
    transaction: GenericTransaction,
    block: BlockIdentifierOrHash,
    trace_config: TraceCallConfig,
}

/// `debug_traceCall`'s third parameter. Extends [`TraceConfig`] with `txIndex`, the
/// in-block transaction index whose pre-state the call should run on top of (geth's
/// `TraceCallConfig`). `stateOverrides`/`blockOverrides` are not supported.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TraceCallConfig {
    #[serde(flatten)]
    base: TraceConfig,
    #[serde(default)]
    tx_index: Option<u64>,
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
        trace_block(block, &self.trace_config, context).await
    }
}

impl RpcHandler for TraceBlockByHashRequest {
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

        Ok(TraceBlockByHashRequest {
            block_hash: serde_json::from_value(params[0].clone())?,
            trace_config,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<serde_json::Value, RpcErr> {
        let block = context
            .storage
            .get_block_by_hash(self.block_hash)
            .await?
            .ok_or(RpcErr::Internal("Block not Found".to_string()))?;
        trace_block(block, &self.trace_config, context).await
    }
}

/// Traces every transaction in `block` with the tracer selected by `trace_config` and
/// returns the geth-RPC array shape shared by `debug_traceBlockByNumber` and
/// `debug_traceBlockByHash` (one `{txHash, result}` entry per transaction).
async fn trace_block(
    block: Block,
    trace_config: &TraceConfig,
    context: RpcApiContext,
) -> Result<Value, RpcErr> {
    let reexec = trace_config.reexec.unwrap_or(DEFAULT_REEXEC);
    let timeout = trace_config.timeout.unwrap_or(DEFAULT_TIMEOUT);
    match trace_config.tracer {
        TracerType::CallTracer => {
            // Parse tracer config now that we know the type
            let config = if let Some(value) = &trace_config.tracer_config {
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
            let config: PrestateTracerConfig = if let Some(value) = &trace_config.tracer_config {
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
            let cfg: OpcodeTracerConfig = trace_config
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
            // geth-RPC shape expected by block-trace consumers.
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
    }
}

impl RpcHandler for TraceCallRequest {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() || params.len() > 3 {
            return Err(RpcErr::BadParams("Expected 1 to 3 params".to_owned()));
        }

        let transaction = serde_json::from_value(params[0].clone())?;

        // Block defaults to `latest` when omitted, matching geth.
        let block = match params.get(1) {
            Some(value) => BlockIdentifierOrHash::parse(value.clone(), 1)?,
            None => BlockIdentifierOrHash::Identifier(BlockIdentifier::default()),
        };

        let trace_config = match params.get(2) {
            Some(value) => serde_json::from_value(value.clone())?,
            None => TraceCallConfig::default(),
        };

        Ok(TraceCallRequest {
            transaction,
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

        // `None` traces on top of the full block (geth's default); `Some(i)` runs the
        // call against the state just before the block's transaction `i`.
        let tx_index = self.trace_config.tx_index.map(|i| i as usize);
        // Match geth: `txIndex` must be < tx count, the sole exception being index 0 on
        // an empty block. Reject otherwise so we don't silently trace against a bogus
        // state (all txs applied but withdrawals skipped).
        if let Some(i) = tx_index {
            let tx_count = block.body.transactions.len();
            if i >= tx_count && !(i == 0 && tx_count == 0) {
                return Err(RpcErr::BadParams(format!(
                    "txIndex {i} out of range for block with {tx_count} transactions"
                )));
            }
        }
        let reexec = self.trace_config.base.reexec.unwrap_or(DEFAULT_REEXEC);
        let timeout = self.trace_config.base.timeout.unwrap_or(DEFAULT_TIMEOUT);

        // Fill the nonce from account state when the caller omits it, matching geth's
        // `ToMessage` (`args.Nonce = db.GetNonce(from)`) and `eth_estimateGas`. Without this
        // the VM's nonce check compares the account's real nonce against a default of 0 and
        // rejects the call.
        let transaction = match self.transaction.nonce {
            Some(_) => self.transaction.clone(),
            None => {
                let nonce = context
                    .storage
                    .get_nonce_by_account_address(block_number, self.transaction.from)
                    .await?;
                let mut transaction = self.transaction.clone();
                transaction.nonce = nonce;
                transaction
            }
        };

        match self.trace_config.base.tracer {
            TracerType::CallTracer => {
                // Parse tracer config now that we know the type
                let config = if let Some(value) = &self.trace_config.base.tracer_config {
                    serde_json::from_value(value.clone())?
                } else {
                    CallTracerConfig::default()
                };
                let call_trace = context
                    .blockchain
                    .trace_call_calls(
                        block,
                        tx_index,
                        transaction.clone(),
                        reexec,
                        timeout,
                        config.only_top_call,
                        config.with_log,
                    )
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                // Geth returns a single CallTraceFrame object, not an array.
                let top_frame = call_trace
                    .into_iter()
                    .next()
                    .ok_or(RpcErr::Internal("Empty call trace".to_string()))?;
                Ok(serde_json::to_value(top_frame)?)
            }
            TracerType::PrestateTracer => {
                let config: PrestateTracerConfig =
                    if let Some(value) = &self.trace_config.base.tracer_config {
                        serde_json::from_value(value.clone())?
                    } else {
                        PrestateTracerConfig::default()
                    };
                config.validate()?;
                let result = context
                    .blockchain
                    .trace_call_prestate(
                        block,
                        tx_index,
                        transaction.clone(),
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
                    .base
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
                    .trace_call_opcodes(block, tx_index, transaction.clone(), reexec, timeout, cfg)
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                // `debug_traceCall` returns the geth-RPC structLogger shape.
                Ok(serde_json::to_value(StructLoggerResult {
                    result: &result,
                    emit,
                })?)
            }
        }
    }
}
