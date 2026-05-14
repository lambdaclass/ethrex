use std::time::Duration;

use ethrex_blockchain::vm::{OverlaidVmDatabase, StoreVmDatabase};
use ethrex_common::H256;
use ethrex_common::types::GenericTransaction;
use ethrex_common::{
    serde_utils,
    tracing::{CallTraceFrame, PrestateResult},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    rpc::RpcHandler,
    types::{
        block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
        block_override::BlockOverrideSet,
        state_override::StateOverrideSet,
    },
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

/// `debug_traceCall` request: trace a synthetic transaction (no on-chain hash)
/// at a given historical block, with optional state and block overrides.
pub struct TraceCallRequest {
    transaction: GenericTransaction,
    block: Option<BlockIdentifierOrHash>,
    trace_config: TraceConfig,
    state_overrides: Option<StateOverrideSet>,
    block_overrides: Option<BlockOverrideSet>,
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

#[derive(Default, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
enum TracerType {
    #[default]
    CallTracer,
    PrestateTracer,
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
        }
    }
}

impl RpcHandler for TraceCallRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() {
            return Err(RpcErr::BadParams("No params provided".to_owned()));
        }
        if params.len() > 5 {
            return Err(RpcErr::BadParams(format!(
                "Expected one to five params and {} were provided",
                params.len()
            )));
        }
        let transaction = serde_json::from_value(params[0].clone())?;
        let block = match params.get(1) {
            Some(v) if !v.is_null() => Some(BlockIdentifierOrHash::parse(v.clone(), 1)?),
            _ => None,
        };
        let trace_config = match params.get(2) {
            Some(v) if !v.is_null() => serde_json::from_value(v.clone())?,
            _ => TraceConfig::default(),
        };
        let state_overrides = match params.get(3) {
            Some(v) if !v.is_null() => Some(serde_json::from_value(v.clone())?),
            _ => None,
        };
        let block_overrides = match params.get(4) {
            Some(v) if !v.is_null() => Some(serde_json::from_value(v.clone())?),
            _ => None,
        };
        Ok(TraceCallRequest {
            transaction,
            block,
            trace_config,
            state_overrides,
            block_overrides,
        })
    }

    async fn handle(
        &self,
        context: crate::rpc::RpcApiContext,
    ) -> Result<Value, crate::utils::RpcErr> {
        // Resolve historical header. Default to "latest" when no block param.
        let block = self
            .block
            .clone()
            .unwrap_or(BlockIdentifierOrHash::Identifier(BlockIdentifier::default()));
        let real_header = match block.resolve_block_header(&context.storage).await? {
            Some(header) => header,
            None => return Ok(Value::Null),
        };
        let real_head_number = context.storage.get_latest_block_number().await?;
        let chain_config = context.storage.get_chain_config();
        let effective_header = match &self.block_overrides {
            Some(bo) if !bo.is_empty() => bo.apply_to(real_header.clone(), &chain_config),
            _ => real_header.clone(),
        };

        let timeout = self.trace_config.timeout.unwrap_or(DEFAULT_TIMEOUT);
        let tracer = self.trace_config.tracer.clone();
        let tracer_config = self.trace_config.tracer_config.clone();

        let storage = context.storage.clone();
        let blockchain = context.blockchain.clone();
        let transaction = self.transaction.clone();
        let state_overrides = self.state_overrides.clone();

        let operation: Box<dyn FnOnce() -> Result<Value, crate::utils::RpcErr> + Send + 'static> =
            Box::new(move || {
                let inner = StoreVmDatabase::new(storage, real_header.clone())?;
                let mut vm = match state_overrides {
                    Some(set) if !set.is_empty() => {
                        let wrapper =
                            OverlaidVmDatabase::new(inner, set.into_overrides(), real_head_number);
                        blockchain.new_evm(wrapper)?
                    }
                    _ => blockchain.new_evm(inner)?,
                };

                match tracer {
                    TracerType::CallTracer => {
                        let cfg: CallTracerConfig = match tracer_config {
                            Some(v) => serde_json::from_value(v)?,
                            None => CallTracerConfig::default(),
                        };
                        let trace = vm.trace_call_from_generic(
                            &transaction,
                            &effective_header,
                            cfg.only_top_call,
                            cfg.with_log,
                        )?;
                        // Geth returns a single frame, not an array.
                        let top = trace
                            .into_iter()
                            .next()
                            .ok_or(RpcErr::Internal("Empty call trace".to_string()))?;
                        Ok(serde_json::to_value(top)?)
                    }
                    TracerType::PrestateTracer => {
                        let cfg: PrestateTracerConfig = match tracer_config {
                            Some(v) => serde_json::from_value(v)?,
                            None => PrestateTracerConfig::default(),
                        };
                        cfg.validate()?;
                        let result = vm.prestate_call_from_generic(
                            &transaction,
                            &effective_header,
                            cfg.diff_mode,
                            cfg.include_empty,
                        )?;
                        match result {
                            PrestateResult::Prestate(t) => Ok(serde_json::to_value(t)?),
                            PrestateResult::Diff(d) => Ok(serde_json::to_value(d)?),
                        }
                    }
                }
            });

        tokio::time::timeout(timeout, tokio::task::spawn_blocking(operation))
            .await
            .map_err(|_| RpcErr::Internal("Tracing Timeout".to_string()))?
            .map_err(|_| RpcErr::Internal("Unexpected Runtime Error".to_string()))?
    }
}
