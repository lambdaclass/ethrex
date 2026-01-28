use std::collections::BTreeMap;

use crate::{
    eth::client::EthConfigResponse,
    mempool::MempoolContent,
    types::{
        block::RpcBlock,
        block_identifier::BlockIdentifier,
        receipt::{RpcLog, RpcReceipt},
        transaction::RpcTransaction,
    },
    utils::{RpcRequest, RpcResponse},
};
use bytes::Bytes;
use errors::{EthClientError, RpcRequestError};
use ethrex_common::{
    Address, H256, U256,
    types::{
        AuthorizationTupleEntry, BlobsBundle, Block, GenericTransaction, TxKind,
        block_execution_witness::RpcExecutionWitness,
    },
    utils::decode_hex,
};
use ethrex_rlp::decode::RLPDecode;
use reqwest::{Client, Url};
use serde_json::{Value, json};
use tracing::{debug, trace, warn};

pub mod errors;

#[derive(Debug, Clone)]
pub struct EthClient {
    client: Client,
    pub urls: Vec<Url>,
    pub max_number_of_retries: u64,
    pub backoff_factor: u64,
    pub min_retry_delay: u64,
    pub max_retry_delay: u64,
    pub maximum_allowed_max_fee_per_gas: Option<u64>,
    pub maximum_allowed_max_fee_per_blob_gas: Option<u64>,
}

#[derive(Default, Clone, Debug)]
pub struct Overrides {
    pub from: Option<Address>,
    pub to: Option<TxKind>,
    pub value: Option<U256>,
    pub nonce: Option<u64>,
    pub chain_id: Option<u64>,
    pub gas_limit: Option<u64>,
    pub max_fee_per_gas: Option<u64>,
    pub max_priority_fee_per_gas: Option<u64>,
    pub access_list: Vec<(Address, Vec<H256>)>,
    pub fee_token: Option<Address>,
    pub gas_price_per_blob: Option<U256>,
    pub block: Option<BlockIdentifier>,
    pub blobs_bundle: Option<BlobsBundle>,
    pub authorization_list: Option<Vec<AuthorizationTupleEntry>>,
    pub wrapper_version: Option<u8>,
}

pub const MAX_NUMBER_OF_RETRIES: u64 = 10;
pub const BACKOFF_FACTOR: u64 = 2;
// Give at least 8 blocks before trying to bump gas.
pub const MIN_RETRY_DELAY: u64 = 96;
pub const MAX_RETRY_DELAY: u64 = 1800;

// 0x08c379a0 == Error(String)
pub const ERROR_FUNCTION_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];

impl EthClient {
    pub fn new(url: Url) -> Result<EthClient, EthClientError> {
        Self::new_with_config(
            vec![url],
            MAX_NUMBER_OF_RETRIES,
            BACKOFF_FACTOR,
            MIN_RETRY_DELAY,
            MAX_RETRY_DELAY,
            None,
            None,
        )
    }

    pub fn new_with_config(
        urls: Vec<Url>,
        max_number_of_retries: u64,
        backoff_factor: u64,
        min_retry_delay: u64,
        max_retry_delay: u64,
        maximum_allowed_max_fee_per_gas: Option<u64>,
        maximum_allowed_max_fee_per_blob_gas: Option<u64>,
    ) -> Result<Self, EthClientError> {
        Ok(Self {
            client: Client::new(),
            urls,
            max_number_of_retries,
            backoff_factor,
            min_retry_delay,
            max_retry_delay,
            maximum_allowed_max_fee_per_gas,
            maximum_allowed_max_fee_per_blob_gas,
        })
    }

    pub fn new_with_multiple_urls(urls: Vec<Url>) -> Result<EthClient, EthClientError> {
        Self::new_with_config(
            urls,
            MAX_NUMBER_OF_RETRIES,
            BACKOFF_FACTOR,
            MIN_RETRY_DELAY,
            MAX_RETRY_DELAY,
            None,
            None,
        )
    }

    /// Send a request to the RPC. Tries each URL until one succeeds.
    pub async fn send_request(&self, request: RpcRequest) -> Result<RpcResponse, EthClientError> {
        let mut response = Err(EthClientError::FailedAllRPC);

        for url in self.urls.iter() {
            response = self.send_request_to_url(url, &request).await;
            // Some RPC servers don't implement all the endpoints or don't implement them completely/correctly
            // so if the server returns Ok(RpcResponse::Error) we retry with the others
            match &response {
                Ok(RpcResponse::Success(_)) => {
                    debug!(endpoint = %url, "RPC request successful");
                    return response;
                }
                Ok(RpcResponse::Error(err)) => {
                    debug!(endpoint = %url, error = ?err.error, "RPC server returned an error");
                }
                Err(error) => {
                    warn!(endpoint = %url, %error, "Could not request RPC server");
                }
            }
        }

        response
    }

    /// Send a request to **all** RPC URLs.
    ///
    /// Return the first successful response, or the last error if all fail.
    async fn send_request_to_all(
        &self,
        request: RpcRequest,
    ) -> Result<RpcResponse, EthClientError> {
        let mut response = Err(EthClientError::FailedAllRPC);

        for url in self.urls.iter() {
            let maybe_response = self.send_request_to_url(url, &request).await;

            match &maybe_response {
                Ok(RpcResponse::Success(_)) => {
                    debug!(endpoint = %url, "RPC request successful");
                }
                Ok(RpcResponse::Error(err)) => {
                    debug!(endpoint = %url, error = ?err.error, "RPC server returned an error");
                }
                Err(error) => {
                    warn!(endpoint = %url, %error, "Could not request RPC server");
                }
            };

            response = response.or(maybe_response);
        }

        response
    }

    /// Send a request to a specific URL.
    async fn send_request_to_url(
        &self,
        rpc_url: &Url,
        request: &RpcRequest,
    ) -> Result<RpcResponse, EthClientError> {
        let id = uuid::Uuid::new_v4();
        trace!(endpoint = %rpc_url, ?request, %id, "Sending RPC request");

        self.client
            .post(rpc_url.as_str())
            .header("content-type", "application/json")
            .body(serde_json::ser::to_string(&request).map_err(|error| {
                EthClientError::FailedToSerializeRequestBody(format!("{error}: {request:?}"))
            })?)
            .send()
            .await
            .inspect(|_| trace!(endpoint = %rpc_url, %id, "Request finished successfully"))?
            .json::<RpcResponse>()
            .await
            .inspect(|body| trace!(endpoint = %rpc_url, %id, ?body, "Response deserialized successfully"))
            .inspect_err(|err| trace!(endpoint = %rpc_url, %id, %err, "Failed to deserialize response"))
            .map_err(EthClientError::from)
    }

    /// Helper to send a request and parse the response, handling errors uniformly.
    /// Extracts the method name from the request for error reporting.
    pub async fn send_request_parsed<T: serde::de::DeserializeOwned>(
        &self,
        request: RpcRequest,
    ) -> Result<T, EthClientError> {
        let method = request.method.clone();
        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(|e| RpcRequestError::SerdeJSONError { method, source: e })
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
                method,
                message: error_response.error.message,
            }
            .into()),
        }
    }

    /// Helper to send a request to all URLs and parse the response.
    async fn send_request_to_all_parsed<T: serde::de::DeserializeOwned>(
        &self,
        request: RpcRequest,
    ) -> Result<T, EthClientError> {
        let method = request.method.clone();
        match self.send_request_to_all(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(|e| RpcRequestError::SerdeJSONError { method, source: e })
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
                method,
                message: error_response.error.message,
            }
            .into()),
        }
    }

    pub async fn send_raw_transaction(&self, data: &[u8]) -> Result<H256, EthClientError> {
        let params = Some(vec![json!("0x".to_string() + &hex::encode(data))]);
        let request = RpcRequest::new("eth_sendRawTransaction", params);
        self.send_request_to_all_parsed(request).await
    }

    pub async fn estimate_gas(
        &self,
        transaction: GenericTransaction,
    ) -> Result<u64, EthClientError> {
        let to = match transaction.to {
            TxKind::Call(addr) => Some(format!("{addr:#x}")),
            TxKind::Create => None,
        };

        let mut data = json!({
            "to": to,
            "input": format!("0x{:#x}", transaction.input),
            "from": format!("{:#x}", transaction.from),
            "value": format!("{:#x}", transaction.value),

        });

        if !transaction.blob_versioned_hashes.is_empty() {
            let blob_versioned_hashes_str: Vec<_> = transaction
                .blob_versioned_hashes
                .into_iter()
                .map(|hash| format!("{hash:#x}"))
                .collect();

            data.as_object_mut()
                .ok_or_else(|| {
                    EthClientError::Custom("Failed to mutate data in estimate_gas".to_owned())
                })?
                .insert(
                    "blobVersionedHashes".to_owned(),
                    json!(blob_versioned_hashes_str),
                );
        }

        if !transaction.blobs.is_empty() {
            let blobs_str: Vec<_> = transaction
                .blobs
                .into_iter()
                .map(|blob| format!("0x{}", hex::encode(blob)))
                .collect();

            data.as_object_mut()
                .ok_or_else(|| {
                    EthClientError::Custom("Failed to mutate data in estimate_gas".to_owned())
                })?
                .insert("blobs".to_owned(), json!(blobs_str));
        }

        // Add the nonce just if present, otherwise the RPC will use the latest nonce
        if let Some(nonce) = transaction.nonce
            && let Value::Object(ref mut map) = data
        {
            map.insert("nonce".to_owned(), json!(format!("{nonce:#x}")));
        }

        let request = RpcRequest::new("eth_estimateGas", Some(vec![data, json!("latest")]));

        match self.send_request(request).await? {
            RpcResponse::Success(result) => {
                let res = serde_json::from_value::<String>(result.result).map_err(|e| {
                    RpcRequestError::SerdeJSONError {
                        method: "eth_estimateGas".to_string(),
                        source: e,
                    }
                })?;
                let res = res.get(2..).ok_or(RpcRequestError::Custom(
                    "Failed to slice index response in estimate_gas".to_owned(),
                ))?;
                u64::from_str_radix(res, 16)
            }
            .map_err(|e| RpcRequestError::ParseIntError {
                method: "eth_estimateGas".to_string(),
                source: e,
            })
            .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
                method: "eth_estimateGas".to_string(),
                message: error_response.error.message,
            }
            .into()),
        }
    }

    pub async fn call(
        &self,
        to: Address,
        calldata: Bytes,
        overrides: Overrides,
    ) -> Result<String, EthClientError> {
        let tx = GenericTransaction {
            to: TxKind::Call(to),
            input: calldata,
            value: overrides.value.unwrap_or_default(),
            from: overrides.from.unwrap_or_default(),
            gas: overrides.gas_limit,
            gas_price: overrides.max_fee_per_gas.unwrap_or_default(),
            ..Default::default()
        };
        let params = Some(vec![
            json!({
                "to": match tx.to {
                    TxKind::Call(addr) => format!("{addr:#x}"),
                    TxKind::Create => format!("{:#x}", Address::zero()),
                },
                "input": format!("0x{:#x}", tx.input),
                "value": format!("{:#x}", tx.value),
                "from": format!("{:#x}", tx.from),
            }),
            overrides
                .block
                .map(Into::into)
                .unwrap_or(serde_json::Value::String("latest".to_string())),
        ]);

        let request = RpcRequest::new("eth_call", params);
        self.send_request_parsed(request).await
    }

    pub async fn get_max_priority_fee(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest::new("eth_maxPriorityFeePerGas", None);
        self.send_request_parsed(request).await
    }

    pub async fn get_gas_price(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest::new("eth_gasPrice", None);
        self.send_request_parsed(request).await
    }

    pub async fn get_gas_price_with_extra(
        &self,
        bump_percent: u64,
    ) -> Result<U256, EthClientError> {
        let gas_price = self.get_gas_price().await?;

        Ok((gas_price * (100 + bump_percent)) / 100)
    }

    pub async fn get_nonce(
        &self,
        address: Address,
        block: BlockIdentifier,
    ) -> Result<u64, EthClientError> {
        let params = Some(vec![json!(format!("{address:#x}")), block.into()]);
        let request = RpcRequest::new("eth_getTransactionCount", params);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => u64::from_str_radix(
                serde_json::from_value::<String>(result.result)
                    .map_err(|e| RpcRequestError::SerdeJSONError {
                        method: "eth_getTransactionCount".to_string(),
                        source: e,
                    })?
                    .get(2..)
                    .ok_or(EthClientError::Custom(
                        "Failed to deserialize get_nonce request".to_owned(),
                    ))?,
                16,
            )
            .map_err(|e| RpcRequestError::ParseIntError {
                method: "eth_getTransactionCount".to_string(),
                source: e,
            })
            .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
                method: "eth_getTransactionCount".to_string(),
                message: error_response.error.message,
            }
            .into()),
        }
    }

    pub async fn get_block_number(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest::new("eth_blockNumber", None);
        self.send_request_parsed(request).await
    }

    pub async fn get_block_by_hash(&self, block_hash: H256) -> Result<RpcBlock, EthClientError> {
        let params = Some(vec![json!(block_hash), json!(true)]);
        let request = RpcRequest::new("eth_getBlockByHash", params);
        self.send_request_parsed(request).await
    }

    pub async fn peer_count(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest::new("net_peerCount", Some(vec![]));
        self.send_request_parsed(request).await
    }

    /// Fetches a block from the Ethereum blockchain by its number or the latest/earliest/pending block.
    /// If no `block_number` is provided, get the latest.
    pub async fn get_block_by_number(
        &self,
        block: BlockIdentifier,
        hydrated: bool,
    ) -> Result<RpcBlock, EthClientError> {
        let params = Some(vec![block.into(), json!(hydrated)]);
        let request = RpcRequest::new("eth_getBlockByNumber", params);
        self.send_request_parsed(request).await
    }

    pub async fn get_raw_block(&self, block: BlockIdentifier) -> Result<Block, EthClientError> {
        let request = RpcRequest::new("debug_getRawBlock", Some(vec![block.into()]));

        let encoded_block: Result<String, _> = match self.send_request(request).await? {
            RpcResponse::Success(result) => {
                serde_json::from_value(result.result).map_err(|e| RpcRequestError::SerdeJSONError {
                    method: "debug_getRawBlock".to_string(),
                    source: e,
                })
            }
            RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
                method: "debug_getRawBlock".to_string(),
                message: error_response.error.message,
            }),
        };

        let encoded_block = decode_hex(&encoded_block?)
            .map_err(|e| EthClientError::Custom(format!("Failed to decode hex: {e}")))?;

        let block = Block::decode_unfinished(&encoded_block).map_err(|e| {
            RpcRequestError::RLPDecodeError {
                method: "debug_getRawBlock".to_string(),
                message: e.to_string(),
            }
        })?;
        Ok(block.0)
    }

    pub async fn get_logs(
        &self,
        from_block: U256,
        to_block: U256,
        address: Address,
        topics: Vec<H256>,
    ) -> Result<Vec<RpcLog>, EthClientError> {
        let params = Some(vec![serde_json::json!(
            {
                "fromBlock": format!("{:#x}", from_block),
                "toBlock": format!("{:#x}", to_block),
                "address": format!("{:#x}", address),
                "topics": topics.iter().map(|topic| format!("{topic:#x}")).collect::<Vec<_>>()
            }
        )]);
        let request = RpcRequest::new("eth_getLogs", params);
        self.send_request_parsed(request).await
    }

    pub async fn get_transaction_receipt(
        &self,
        tx_hash: H256,
    ) -> Result<Option<RpcReceipt>, EthClientError> {
        let params = Some(vec![json!(format!("{:#x}", tx_hash))]);
        let request = RpcRequest::new("eth_getTransactionReceipt", params);
        self.send_request_parsed(request).await
    }

    pub async fn get_balance(
        &self,
        address: Address,
        block: BlockIdentifier,
    ) -> Result<U256, EthClientError> {
        let params = Some(vec![json!(format!("{:#x}", address)), block.into()]);
        let request = RpcRequest::new("eth_getBalance", params);
        self.send_request_parsed(request).await
    }

    pub async fn get_storage_at(
        &self,
        address: Address,
        slot: U256,
        block: BlockIdentifier,
    ) -> Result<U256, EthClientError> {
        let params = Some(vec![
            json!(format!("{:#x}", address)),
            json!(format!("{:#x}", slot)),
            block.into(),
        ]);
        let request = RpcRequest::new("eth_getStorageAt", params);
        self.send_request_parsed(request).await
    }

    pub async fn get_chain_id(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest::new("eth_chainId", None);
        self.send_request_parsed(request).await
    }

    pub async fn get_eth_config(&self) -> Result<EthConfigResponse, EthClientError> {
        let request = RpcRequest::new("eth_config", None);
        self.send_request_parsed(request).await
    }

    pub async fn get_code(
        &self,
        address: Address,
        block: BlockIdentifier,
    ) -> Result<Bytes, EthClientError> {
        let params = Some(vec![json!(format!("{:#x}", address)), block.into()]);
        let request = RpcRequest::new("eth_getCode", params);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => hex::decode(
                &serde_json::from_value::<String>(result.result)
                    .map(|hex_str| {
                        hex_str
                            .strip_prefix("0x")
                            .map(ToString::to_string)
                            .unwrap_or(hex_str)
                    })
                    .map_err(|e| RpcRequestError::SerdeJSONError {
                        method: "eth_getCode".to_string(),
                        source: e,
                    })
                    .map_err(EthClientError::from)?,
            )
            .map(Into::into)
            .map_err(|e| RpcRequestError::HexError {
                method: "eth_getCode".to_string(),
                source: e,
            })
            .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
                method: "eth_getCode".to_string(),
                message: error_response.error.message,
            }
            .into()),
        }
    }

    pub async fn get_transaction_by_hash(
        &self,
        tx_hash: H256,
    ) -> Result<Option<RpcTransaction>, EthClientError> {
        let params = Some(vec![json!(format!("{tx_hash:#x}"))]);
        let request = RpcRequest::new("eth_getTransactionByHash", params);
        self.send_request_parsed(request).await
    }

    /// Fethches the execution witnes for a given block or range of blocks.
    /// WARNNING: This method is only compatible with ethrex and not with other debug_executionWitness implementations.
    pub async fn get_witness(
        &self,
        from: BlockIdentifier,
        to: Option<BlockIdentifier>,
    ) -> Result<RpcExecutionWitness, EthClientError> {
        let params = if let Some(to_block) = to {
            Some(vec![from.into(), to_block.into()])
        } else {
            Some(vec![from.into()])
        };

        let request = RpcRequest::new("debug_executionWitness", params);
        self.send_request_parsed(request).await
    }

    pub async fn tx_pool_content(&self) -> Result<MempoolContent, EthClientError> {
        let request = RpcRequest::new("txpool_content", None);
        self.send_request_parsed(request).await
    }

    pub async fn get_blob_base_fee(&self, block: BlockIdentifier) -> Result<u64, EthClientError> {
        let params = Some(vec![block.into()]);
        let request = RpcRequest::new("eth_blobBaseFee", params);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => Ok(u64::from_str_radix(
                serde_json::from_value::<String>(result.result)
                    .map_err(|e| RpcRequestError::SerdeJSONError {
                        method: "eth_blobBaseFee".to_string(),
                        source: e,
                    })?
                    .trim_start_matches("0x"),
                16,
            )
            .map_err(|e| RpcRequestError::ParseIntError {
                method: "eth_blobBaseFee".to_string(),
                source: e,
            })?),
            RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
                method: "eth_blobBaseFee".to_string(),
                message: error_response.error.message,
            }
            .into()),
        }
    }

    /// Smoke test the all the urls by calling eth_blockNumber
    pub async fn test_urls(&self) -> BTreeMap<String, serde_json::Value> {
        let mut map = BTreeMap::new();
        for url in self.urls.iter() {
            let response = match self
                .send_request_to_url(url, &RpcRequest::new("eth_blockNumber", None))
                .await
            {
                Ok(RpcResponse::Success(ok)) => serde_json::to_value(ok).unwrap_or_else(|e| {
                    serde_json::Value::String(format!("Failed to serialize success response: {e}"))
                }),
                Ok(RpcResponse::Error(e)) => serde_json::to_value(e).unwrap_or_else(|e| {
                    serde_json::Value::String(format!("Failed to serialize error response: {e}"))
                }),
                Err(e) => serde_json::Value::String(format!("Request error: {e}")),
            };
            map.insert(url.to_string(), response);
        }
        map
    }
}
