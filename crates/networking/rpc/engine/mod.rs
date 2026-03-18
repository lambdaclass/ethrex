pub mod blobs;
pub mod client_version;
pub mod exchange_transition_config;
pub mod fork_choice;
pub mod payload;
#[cfg(feature = "eip-8025")]
pub mod proof;
#[cfg(feature = "eip-8025")]
pub mod proof_types;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
    utils::RpcRequest,
};
use serde_json::{Value, json};

pub type ExchangeCapabilitiesRequest = Vec<String>;

/// List of capabilities that the execution layer client supports. Add new capabilities here.
/// More info: https://github.com/ethereum/execution-apis/blob/main/src/engine/common.md#engine_exchangecapabilities
pub const CAPABILITIES: [&str; 24] = [
    "engine_forkchoiceUpdatedV1",
    "engine_forkchoiceUpdatedV2",
    "engine_forkchoiceUpdatedV3",
    "engine_forkchoiceUpdatedV4",
    "engine_newPayloadV1",
    "engine_newPayloadV2",
    "engine_newPayloadV3",
    "engine_newPayloadV4",
    "engine_newPayloadV5",
    "engine_getPayloadV1",
    "engine_getPayloadV2",
    "engine_getPayloadV3",
    "engine_getPayloadV4",
    "engine_getPayloadV5",
    "engine_getPayloadV6",
    "engine_exchangeTransitionConfigurationV1",
    "engine_getPayloadBodiesByHashV1",
    "engine_getPayloadBodiesByRangeV1",
    "engine_getPayloadBodiesByHashV2",
    "engine_getPayloadBodiesByRangeV2",
    "engine_getBlobsV1",
    "engine_getBlobsV2",
    "engine_getBlobsV3",
    "engine_getClientVersionV1",
];

/// EIP-8025 proof capabilities, advertised only when the feature is enabled.
#[cfg(feature = "eip-8025")]
pub const EIP8025_CAPABILITIES: [&str; 3] = [
    "engine_requestProofsV1",
    "engine_verifyExecutionProofV1",
    "engine_verifyNewPayloadRequestHeaderV1",
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

    async fn handle(&self, _context: RpcApiContext) -> Result<Value, RpcErr> {
        #[cfg(not(feature = "eip-8025"))]
        {
            Ok(json!(CAPABILITIES))
        }
        #[cfg(feature = "eip-8025")]
        {
            let mut caps: Vec<&str> = CAPABILITIES.to_vec();
            caps.extend_from_slice(&EIP8025_CAPABILITIES);
            Ok(json!(caps))
        }
    }
}
