pub mod capabilities;
pub mod exchange_transition_config;
pub mod fork_choice;
pub mod payload;

use crate::{context::RpcApiContext, rpc_types::RpcRequest, server::RpcHandler, RpcErr};
use serde_json::Value;

pub use capabilities::ExchangeCapabilitiesRequest;

pub async fn map_engine_requests(
    req: &RpcRequest,
    context: RpcApiContext,
) -> Result<Value, RpcErr> {
    match req.method.as_str() {
        "engine_exchangeCapabilities" => {
            capabilities::ExchangeCapabilitiesRequest::call(req, context)
        }
        "engine_forkchoiceUpdatedV1" => fork_choice::ForkChoiceUpdatedV1::call(req, context),
        "engine_forkchoiceUpdatedV2" => fork_choice::ForkChoiceUpdatedV2::call(req, context),
        "engine_forkchoiceUpdatedV3" => {
            cfg_if::cfg_if! {
                if #[cfg(feature = "based")] {
                    fork_choice::ForkChoiceUpdatedV3::relay_to_gateway_or_fallback(req, context).await
                } else {
                    fork_choice::ForkChoiceUpdatedV3::call(req, context)
                }
            }
        }
        "engine_newPayloadV4" => payload::NewPayloadV4Request::call(req, context),
        "engine_newPayloadV3" => {
            cfg_if::cfg_if! {
                if #[cfg(feature = "based")] {
                    payload::NewPayloadV3Request::relay_to_gateway_or_fallback(req, context).await
                } else {
                    payload::NewPayloadV3Request::call(req, context)
                }
            }
        }
        "engine_newPayloadV2" => payload::NewPayloadV2Request::call(req, context),
        "engine_newPayloadV1" => payload::NewPayloadV1Request::call(req, context),
        "engine_exchangeTransitionConfigurationV1" => {
            exchange_transition_config::ExchangeTransitionConfigV1Req::call(req, context)
        }
        "engine_getPayloadV4" => payload::GetPayloadV4Request::call(req, context),
        "engine_getPayloadV3" => {
            cfg_if::cfg_if! {
                if #[cfg(feature = "based")] {
                    payload::GetPayloadV3Request::relay_to_gateway_or_fallback(req, context).await
                } else {
                    payload::GetPayloadV3Request::call(req, context)
                }
            }
        }
        "engine_getPayloadV2" => payload::GetPayloadV2Request::call(req, context),
        "engine_getPayloadV1" => payload::GetPayloadV1Request::call(req, context),
        "engine_getPayloadBodiesByHashV1" => {
            payload::GetPayloadBodiesByHashV1Request::call(req, context)
        }
        "engine_getPayloadBodiesByRangeV1" => {
            payload::GetPayloadBodiesByRangeV1Request::call(req, context)
        }
        unknown_engine_method => Err(RpcErr::MethodNotFound(unknown_engine_method.to_owned())),
    }
}
