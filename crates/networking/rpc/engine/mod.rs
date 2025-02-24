pub mod exchange_transition_config;
pub mod fork_choice;
pub mod payload;

use crate::{utils::RpcRequest, RpcApiContext, RpcErr, RpcHandler};
use serde_json::{json, Value};

pub type ExchangeCapabilitiesRequest = Vec<String>;

/// List of capabilities that the execution layer client supports. Add new capabilities here.
/// More info: https://github.com/ethereum/execution-apis/blob/main/src/engine/common.md#engine_exchangecapabilities
pub const CAPABILITIES: [&str; 13] = [
    "engine_forkchoiceUpdatedV1",
    "engine_forkchoiceUpdatedV2",
    "engine_forkchoiceUpdatedV3",
    "engine_newPayloadV1",
    "engine_newPayloadV2",
    "engine_newPayloadV3",
    "engine_getPayloadV1",
    "engine_getPayloadV2",
    "engine_getPayloadV3",
    "engine_getPayloadV4",
    "engine_exchangeTransitionConfigurationV1",
    "engine_getPayloadBodiesByHashV1",
    "engine_getPayloadBodiesByRangeV1",
];

impl From<ExchangeCapabilitiesRequest> for RpcRequest {
    fn from(val: ExchangeCapabilitiesRequest) -> Self {
        RpcRequest {
            method: "engine_exchangeCapabilities".to_string(),
            params: Some(vec![serde_json::json!(val)]),
            ..Default::default()
        }
    }
}

impl RpcHandler for ExchangeCapabilitiesRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?
            .first()
            .ok_or(RpcErr::BadParams("Expected 1 param".to_owned()))
            .and_then(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|error| RpcErr::BadParams(error.to_string()))
            })
    }

    fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        Ok(json!(CAPABILITIES))
    }
}
