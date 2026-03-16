use crate::{
    engine::{
        ExchangeCapabilitiesRequest,
        fork_choice::ForkChoiceUpdatedV3,
        payload::{GetPayloadV5Request, NewPayloadV4Request},
    },
    types::{
        fork_choice::{ForkChoiceResponse, ForkChoiceState, PayloadAttributesV3},
        payload::{ExecutionPayload, ExecutionPayloadResponse, PayloadStatus},
    },
    utils::{RpcRequest, RpcResponse},
};
use bytes::Bytes;
use errors::{
    EngineClientError, ExchangeCapabilitiesError, ForkChoiceUpdatedError, GetPayloadError,
    NewPayloadError,
};
use ethrex_common::H256;
use reqwest::Client;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod errors;

#[derive(Debug, Clone)]
pub struct EngineClient {
    client: Client,
    secret: Bytes,
    execution_client_url: String,
}

impl EngineClient {
    pub fn new(execution_client_url: &str, secret: Bytes) -> Self {
        Self {
            client: Client::new(),
            secret,
            execution_client_url: execution_client_url.to_string(),
        }
    }

    async fn send_request(&self, request: RpcRequest) -> Result<RpcResponse, EngineClientError> {
        self.client
            .post(&self.execution_client_url)
            .bearer_auth(self.auth_token()?)
            .header("content-type", "application/json")
            .body(serde_json::ser::to_string(&request).map_err(|error| {
                EngineClientError::FailedToSerializeRequestBody(format!("{error}: {request:?}"))
            })?)
            .send()
            .await?
            .json::<RpcResponse>()
            .await
            .map_err(EngineClientError::from)
    }

    pub async fn engine_exchange_capabilities(&self) -> Result<Vec<String>, EngineClientError> {
        let request = ExchangeCapabilitiesRequest::from(Self::capabilities()).into();

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(ExchangeCapabilitiesError::SerdeJSONError)
                .map_err(EngineClientError::from),
            RpcResponse::Error(error_response) => {
                let error_message = if let Some(data) = error_response.error.data {
                    format!("{}: {:?}", error_response.error.message, data)
                } else {
                    error_response.error.message.to_string()
                };
                Err(ExchangeCapabilitiesError::RPCError(error_message).into())
            }
        }
    }

    pub async fn engine_forkchoice_updated_v3(
        &self,
        state: ForkChoiceState,
        payload_attributes: Option<PayloadAttributesV3>,
    ) -> Result<ForkChoiceResponse, EngineClientError> {
        let request = RpcRequest::from(ForkChoiceUpdatedV3 {
            fork_choice_state: state,
            payload_attributes,
        });

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(ForkChoiceUpdatedError::SerdeJSONError)
                .map_err(EngineClientError::from),
            RpcResponse::Error(error_response) => {
                let error_message = if let Some(data) = error_response.error.data {
                    format!("{}: {:?}", error_response.error.message, data)
                } else {
                    error_response.error.message.to_string()
                };
                Err(ForkChoiceUpdatedError::RPCError(error_message).into())
            }
        }
    }

    pub async fn engine_get_payload_v5(
        &self,
        payload_id: u64,
    ) -> Result<ExecutionPayloadResponse, EngineClientError> {
        let request = GetPayloadV5Request { payload_id }.into();

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetPayloadError::SerdeJSONError)
                .map_err(EngineClientError::from),
            RpcResponse::Error(error_response) => {
                let error_message = if let Some(data) = error_response.error.data {
                    format!("{}: {:?}", error_response.error.message, data)
                } else {
                    error_response.error.message.to_string()
                };
                Err(GetPayloadError::RPCError(error_message).into())
            }
        }
    }

    pub async fn engine_new_payload_v4(
        &self,
        execution_payload: ExecutionPayload,
        expected_blob_versioned_hashes: Vec<H256>,
        parent_beacon_block_root: H256,
    ) -> Result<PayloadStatus, EngineClientError> {
        let request = NewPayloadV4Request {
            payload: execution_payload,
            expected_blob_versioned_hashes,
            parent_beacon_block_root,
            execution_requests: vec![],
        }
        .into();

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(NewPayloadError::SerdeJSONError)
                .map_err(EngineClientError::from),
            RpcResponse::Error(error_response) => {
                let error_message = if let Some(data) = error_response.error.data {
                    format!("{}: {:?}", error_response.error.message, data)
                } else {
                    error_response.error.message.to_string()
                };
                Err(NewPayloadError::RPCError(error_message).into())
            }
        }
    }

    fn auth_token(&self) -> Result<String, EngineClientError> {
        // Header
        let header = jsonwebtoken::Header::default();
        // Claims
        let valid_iat = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let claims = json!({"iat": valid_iat});
        let encoding_key = jsonwebtoken::EncodingKey::from_secret(&self.secret);
        // JWT Token
        jsonwebtoken::encode(&header, &claims, &encoding_key).map_err(EngineClientError::from)
    }

    fn capabilities() -> Vec<String> {
        vec![
            "engine_exchangeCapabilities".to_owned(),
            "engine_forkchoiceUpdatedV3".to_owned(),
            "engine_getPayloadV4".to_owned(),
            "engine_getPayloadV5".to_owned(),
            "engine_newPayloadV4".to_owned(),
        ]
    }
}
