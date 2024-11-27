pub mod exchange_transition_config;
pub mod fork_choice;
pub mod payload_v2;
pub mod payload_v3;

use crate::{utils::RpcRequest, RpcApiContext, RpcErr, RpcHandler};
use serde_json::{json, Value};

pub type ExchangeCapabilitiesRequest = Vec<String>;

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
        Ok(json!(*self))
    }
}
