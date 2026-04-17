use std::time::Duration;

use ethrex_common::H256;
use ethrex_common::{
    serde_utils,
    tracing::{CallTraceFrame, PrePostState, PrestateTrace},
};
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

#[derive(Default, Deserialize)]
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
                let config = if let Some(value) = &self.trace_config.tracer_config {
                    serde_json::from_value(value.clone())?
                } else {
                    PrestateTracerConfig::default()
                };
                let (pre_trace, pre_post) = context
                    .blockchain
                    .trace_transaction_prestate(self.tx_hash, reexec, timeout, config.diff_mode)
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                if config.diff_mode {
                    Ok(serde_json::to_value(pre_post.unwrap_or_default())?)
                } else {
                    Ok(serde_json::to_value(pre_trace)?)
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
                let config = if let Some(value) = &self.trace_config.tracer_config {
                    serde_json::from_value(value.clone())?
                } else {
                    PrestateTracerConfig::default()
                };
                let prestate_traces = context
                    .blockchain
                    .trace_block_prestate(block, reexec, timeout, config.diff_mode)
                    .await
                    .map_err(|err| RpcErr::Internal(err.to_string()))?;
                if config.diff_mode {
                    let block_trace: BlockTrace<PrePostState> = prestate_traces
                        .into_iter()
                        .map(|(hash, _, pre_post)| (hash, pre_post.unwrap_or_default()).into())
                        .collect();
                    Ok(serde_json::to_value(block_trace)?)
                } else {
                    let block_trace: BlockTrace<PrestateTrace> = prestate_traces
                        .into_iter()
                        .map(|(hash, pre_trace, _)| (hash, pre_trace).into())
                        .collect();
                    Ok(serde_json::to_value(block_trace)?)
                }
            }
        }
    }
}
