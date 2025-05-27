use std::time::Duration;

use keccak_hash::H256;
use serde::{de::Error, Deserialize};
use serde_json::Value;

use crate::{rpc::RpcHandler, utils::RpcErr};

/// Default amount of blocks to re-excute if it is not given
const DEFAULT_REEXEC: usize = 128;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct TraceTransactionRequest {
    tx_hash: H256,
    tracer_config: TracerConfig,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TracerConfig {
    tracer: TracerType,
    // This differs for each different tracer so we will parse it afterwards when we know the type
    tracer_config: Option<Value>,
    timeout: Option<Duration>,
    reexec: Option<usize>,
}

#[derive(Default)]
enum TracerType {
    #[default]
    CallTracer,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CallTracerConfig {
    only_top_call: bool,
    with_log: bool,
}

impl<'de> Deserialize<'de> for TracerType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let tracer_name = String::deserialize(deserializer)?;
        match &*tracer_name {
            "callTracer" => Ok(TracerType::CallTracer),
            s => Err(D::Error::custom(format!(
                "Unknown tracer {s}. Supported tracers: callTracer"
            ))),
        }
    }
}

impl RpcHandler for TraceTransactionRequest {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 || params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 1 or 2 params".to_owned()));
        };
        let tracer_config = if params.len() == 2 {
            serde_json::from_value(params[1].clone())?
        } else {
            TracerConfig::default()
        };

        Ok(TraceTransactionRequest {
            tx_hash: serde_json::from_value(params[0].clone())?,
            tracer_config,
        })
    }

    async fn handle(
        &self,
        context: crate::rpc::RpcApiContext,
    ) -> Result<serde_json::Value, crate::utils::RpcErr> {
        // This match will make more sense once we support other tracers
        match self.tracer_config.tracer {
            TracerType::CallTracer => {
                // Parse tracer config now that we know the type
                let config = if let Some(value) = &self.tracer_config.tracer_config {
                    serde_json::from_value(value.clone())?
                } else {
                    CallTracerConfig::default()
                };
                let reexec = self.tracer_config.reexec.unwrap_or(DEFAULT_REEXEC);
                let timeout = self.tracer_config.timeout.unwrap_or(DEFAULT_TIMEOUT);
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
                Ok(serde_json::to_value(call_trace)?)
            }
        }
    }
}
