use std::fmt;

use crate::{
    clients::eth::errors::{
        CallError, GetBatchByNumberError, GetPeerCountError, GetWitnessError, TxPoolContentError,
    },
    debug::execution_witness::RpcExecutionWitness,
    mempool::MempoolContent,
    types::{
        block::RpcBlock,
        block_identifier::{BlockIdentifier, BlockTag},
        receipt::{RpcLog, RpcReceipt},
    },
    utils::{RpcErrorResponse, RpcRequest, RpcSuccessResponse},
};
use bytes::Bytes;
use errors::{
    EstimateGasError, EthClientError, GetBalanceError, GetBlockByHashError, GetBlockByNumberError,
    GetBlockNumberError, GetCodeError, GetGasPriceError, GetLogsError, GetMaxPriorityFeeError,
    GetNonceError, GetRawBlockError, GetTransactionByHashError, GetTransactionReceiptError,
    SendRawTransactionError,
};
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccessListEntry, BlobsBundle, Block, BlockHash, GenericTransaction, TxKind, TxType,
        batch::Batch,
    },
    utils::decode_hex,
};
use ethrex_rlp::decode::RLPDecode;
use keccak_hash::keccak;
use reqwest::{Client, Url};
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::str::FromStr;

pub mod errors;

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum RpcResponse {
    Success(RpcSuccessResponse),
    Error(RpcErrorResponse),
}

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
    pub gas_price_per_blob: Option<U256>,
    pub block: Option<BlockIdentifier>,
    pub blobs_bundle: Option<BlobsBundle>,
}

pub const MAX_NUMBER_OF_RETRIES: u64 = 10;
pub const BACKOFF_FACTOR: u64 = 2;
// Give at least 8 blocks before trying to bump gas.
pub const MIN_RETRY_DELAY: u64 = 96;
pub const MAX_RETRY_DELAY: u64 = 1800;

// 0x08c379a0 == Error(String)
pub const ERROR_FUNCTION_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];

#[derive(Serialize, Deserialize, Debug)]
pub struct L1MessageProof {
    pub batch_number: u64,
    pub message_id: U256,
    pub message_hash: H256,
    pub merkle_proof: Vec<H256>,
}

// TODO: This struct is duplicated from `crates/l2/networking/rpc/l2/batch.rs`.
// It can't be imported because of circular dependencies. After fixed, we should
// remove the duplication.
#[derive(Serialize, Deserialize)]
pub struct RpcBatch {
    #[serde(flatten)]
    pub batch: Batch,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hashes: Option<Vec<BlockHash>>,
}

impl EthClient {
    pub fn new(url: &str) -> Result<EthClient, EthClientError> {
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
        urls: Vec<&str>,
        max_number_of_retries: u64,
        backoff_factor: u64,
        min_retry_delay: u64,
        max_retry_delay: u64,
        maximum_allowed_max_fee_per_gas: Option<u64>,
        maximum_allowed_max_fee_per_blob_gas: Option<u64>,
    ) -> Result<Self, EthClientError> {
        let urls = urls
            .iter()
            .map(|url| {
                Url::parse(url)
                    .map_err(|_| EthClientError::ParseUrlError("Failed to parse urls".to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

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

    pub fn new_with_multiple_urls(urls: Vec<String>) -> Result<EthClient, EthClientError> {
        Self::new_with_config(
            urls.iter().map(AsRef::as_ref).collect(),
            MAX_NUMBER_OF_RETRIES,
            BACKOFF_FACTOR,
            MIN_RETRY_DELAY,
            MAX_RETRY_DELAY,
            None,
            None,
        )
    }

    async fn send_request(&self, request: RpcRequest) -> Result<RpcResponse, EthClientError> {
        let mut response = Err(EthClientError::Custom("All rpc calls failed".to_string()));

        for url in self.urls.iter() {
            response = self.send_request_to_url(url, &request).await;
            if response.is_ok() {
                // Some RPC servers don't implement all the endpoints or don't implement them completely/correctly
                // so if the server returns Ok(RpcResponse::Error) we retry with the others
                if let Ok(RpcResponse::Success(ref _a)) = response {
                    return response;
                }
            }
        }
        response
    }

    async fn send_request_to_all(
        &self,
        request: RpcRequest,
    ) -> Result<RpcResponse, EthClientError> {
        let mut response = Err(EthClientError::FailedAllRPC("No RPC endpoints".to_string()));

        for url in self.urls.iter() {
            let maybe_response = self.send_request_to_url(url, &request).await;

            if response.is_ok() {
                continue;
            }

            response = match &maybe_response {
                Ok(RpcResponse::Success(_)) => maybe_response,
                Ok(RpcResponse::Error(err)) => {
                    Err(EthClientError::FailedAllRPC(err.error.message.clone()))
                }
                Err(_) => maybe_response,
            };
        }

        response
    }

    async fn send_request_to_url(
        &self,
        rpc_url: &Url,
        request: &RpcRequest,
    ) -> Result<RpcResponse, EthClientError> {
        self.client
            .post(rpc_url.as_str())
            .header("content-type", "application/json")
            .body(serde_json::ser::to_string(&request).map_err(|error| {
                EthClientError::FailedToSerializeRequestBody(format!("{error}: {request:?}"))
            })?)
            .send()
            .await?
            .json::<RpcResponse>()
            .await
            .map_err(EthClientError::from)
    }

    pub async fn send_raw_transaction(&self, data: &[u8]) -> Result<H256, EthClientError> {
        let params = Some(vec![json!("0x".to_string() + &hex::encode(data))]);
        let request = RpcRequest::new("eth_sendRawTransaction", params);

        match self.send_request_to_all(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(SendRawTransactionError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(SendRawTransactionError::RPCError(error_response.error.message).into())
            }
        }
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
        if let Some(nonce) = transaction.nonce {
            if let Value::Object(ref mut map) = data {
                map.insert("nonce".to_owned(), json!(format!("{nonce:#x}")));
            }
        }

        let request = RpcRequest::new("eth_estimateGas", Some(vec![data, json!("latest")]));

        match self.send_request(request).await? {
            RpcResponse::Success(result) => {
                let res = serde_json::from_value::<String>(result.result)
                    .map_err(EstimateGasError::SerdeJSONError)?;
                let res = res.get(2..).ok_or(EstimateGasError::Custom(
                    "Failed to slice index response in estimate_gas".to_owned(),
                ))?;
                u64::from_str_radix(res, 16)
            }
            .map_err(EstimateGasError::ParseIntError)
            .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                let error_data = if let Some(error_data) = error_response.error.data {
                    if &error_data == "0x" {
                        "unknown error".to_owned()
                    } else {
                        let abi_decoded_error_data = hex::decode(
                            error_data.strip_prefix("0x").ok_or(EthClientError::Custom(
                                "Failed to strip_prefix in estimate_gas".to_owned(),
                            ))?,
                        )
                        .map_err(|_| {
                            EthClientError::Custom(
                                "Failed to hex::decode in estimate_gas".to_owned(),
                            )
                        })?;
                        let string_length = U256::from_big_endian(
                            abi_decoded_error_data
                                .get(36..68)
                                .ok_or(EthClientError::Custom(
                                    "Failed to slice index abi_decoded_error_data in estimate_gas"
                                        .to_owned(),
                                ))?,
                        );

                        let string_len = if string_length > usize::MAX.into() {
                            return Err(EthClientError::Custom(
                                "Failed to convert string_length to usize in estimate_gas"
                                    .to_owned(),
                            ));
                        } else {
                            string_length.as_usize()
                        };
                        let string_data = abi_decoded_error_data.get(68..68 + string_len).ok_or(
                            EthClientError::Custom(
                                "Failed to slice index abi_decoded_error_data in estimate_gas"
                                    .to_owned(),
                            ),
                        )?;
                        String::from_utf8(string_data.to_vec()).map_err(|_| {
                            EthClientError::Custom(
                                "Failed to String::from_utf8 in estimate_gas".to_owned(),
                            )
                        })?
                    }
                } else {
                    "unknown error".to_owned()
                };
                Err(EstimateGasError::RPCError(format!(
                    "{}: {}",
                    error_response.error.message, error_data
                ))
                .into())
            }
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
            gas_price: if let Some(gas_price) = overrides.max_fee_per_gas {
                gas_price
            } else {
                self.get_gas_price().await?.as_u64()
            },
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

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(CallError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(CallError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn get_max_priority_fee(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest::new("eth_maxPriorityFeePerGas", None);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetMaxPriorityFeeError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetMaxPriorityFeeError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn get_gas_price(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest::new("eth_gasPrice", None);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetGasPriceError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetGasPriceError::RPCError(error_response.error.message).into())
            }
        }
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
                    .map_err(GetNonceError::SerdeJSONError)?
                    .get(2..)
                    .ok_or(EthClientError::Custom(
                        "Failed to deserialize get_nonce request".to_owned(),
                    ))?,
                16,
            )
            .map_err(GetNonceError::ParseIntError)
            .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetNonceError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn get_block_number(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest::new("eth_blockNumber", None);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetBlockNumberError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetBlockNumberError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn get_block_by_hash(&self, block_hash: H256) -> Result<RpcBlock, EthClientError> {
        let params = Some(vec![json!(block_hash), json!(true)]);
        let request = RpcRequest::new("eth_getBlockByHash", params);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetBlockByHashError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetBlockByHashError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn peer_count(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest::new("net_peerCount", Some(vec![]));

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetPeerCountError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetPeerCountError::RPCError(error_response.error.message).into())
            }
        }
    }

    /// Fetches a block from the Ethereum blockchain by its number or the latest/earliest/pending block.
    /// If no `block_number` is provided, get the latest.
    pub async fn get_block_by_number(
        &self,
        block: BlockIdentifier,
    ) -> Result<RpcBlock, EthClientError> {
        let params = Some(vec![block.into(), json!(false)]); // With false it just returns the hash of the transactions.
        let request = RpcRequest::new("eth_getBlockByNumber", params);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetBlockByNumberError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetBlockByNumberError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn get_raw_block(&self, block: BlockIdentifier) -> Result<Block, EthClientError> {
        let request = RpcRequest::new("debug_getRawBlock", Some(vec![block.into()]));

        let encoded_block: Result<String, _> = match self.send_request(request).await? {
            RpcResponse::Success(result) => {
                serde_json::from_value(result.result).map_err(GetRawBlockError::SerdeJSONError)
            }
            RpcResponse::Error(error_response) => {
                Err(GetRawBlockError::RPCError(error_response.error.message))
            }
        };

        let encoded_block = decode_hex(&encoded_block?)
            .map_err(|e| EthClientError::Custom(format!("Failed to decode hex: {e}")))?;

        let block = Block::decode_unfinished(&encoded_block)
            .map_err(|e| GetRawBlockError::RLPDecodeError(e.to_string()))?;
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

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetLogsError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetLogsError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn get_transaction_receipt(
        &self,
        tx_hash: H256,
    ) -> Result<Option<RpcReceipt>, EthClientError> {
        let params = Some(vec![json!(format!("{:#x}", tx_hash))]);
        let request = RpcRequest::new("eth_getTransactionReceipt", params);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetTransactionReceiptError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetTransactionReceiptError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn get_balance(
        &self,
        address: Address,
        block: BlockIdentifier,
    ) -> Result<U256, EthClientError> {
        let params = Some(vec![json!(format!("{:#x}", address)), block.into()]);
        let request = RpcRequest::new("eth_getBalance", params);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetBalanceError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetBalanceError::RPCError(error_response.error.message).into())
            }
        }
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

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetBalanceError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetBalanceError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn get_chain_id(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest::new("eth_chainId", None);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetBalanceError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetBalanceError::RPCError(error_response.error.message).into())
            }
        }
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
                    .map_err(GetCodeError::SerdeJSONError)
                    .map_err(EthClientError::from)?,
            )
            .map(Into::into)
            .map_err(GetCodeError::NotHexError)
            .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetCodeError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn get_transaction_by_hash(
        &self,
        tx_hash: H256,
    ) -> Result<Option<GetTransactionByHashTransaction>, EthClientError> {
        let params = Some(vec![json!(format!("{tx_hash:#x}"))]);
        let request = RpcRequest::new("eth_getTransactionByHash", params);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetTransactionByHashError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetTransactionByHashError::RPCError(error_response.error.message).into())
            }
        }
    }

    /// Build a GenericTransaction with the given parameters.
    /// Either `overrides.nonce` or `overrides.from` must be provided.
    /// If `overrides.gas_price`, `overrides.chain_id` or `overrides.gas_price`
    /// are not provided, the client will fetch them from the network.
    /// If `overrides.gas_limit` is not provided, the client will estimate the tx cost.
    pub async fn build_generic_tx(
        &self,
        r#type: TxType,
        to: Address,
        from: Address,
        calldata: Bytes,
        overrides: Overrides,
    ) -> Result<GenericTransaction, EthClientError> {
        match r#type {
            TxType::EIP1559 | TxType::EIP4844 | TxType::Privileged => {}
            TxType::EIP2930 | TxType::EIP7702 | TxType::Legacy => {
                return Err(EthClientError::Custom(
                    "Unsupported tx type in build_generic_tx".to_owned(),
                ));
            }
        }
        let mut tx = GenericTransaction {
            r#type,
            to: overrides.to.clone().unwrap_or(TxKind::Call(to)),
            chain_id: Some(if let Some(chain_id) = overrides.chain_id {
                chain_id
            } else {
                self.get_chain_id().await?.try_into().map_err(|_| {
                    EthClientError::Custom("Failed at get_chain_id().try_into()".to_owned())
                })?
            }),
            nonce: Some(
                self.get_nonce_from_overrides_or_rpc(&overrides, from)
                    .await?,
            ),
            max_fee_per_gas: Some(
                self.get_fee_from_override_or_get_gas_price(overrides.max_fee_per_gas)
                    .await?,
            ),
            max_priority_fee_per_gas: Some(
                self.priority_fee_from_override_or_rpc(overrides.max_priority_fee_per_gas)
                    .await?,
            ),
            max_fee_per_blob_gas: overrides.gas_price_per_blob,
            value: overrides.value.unwrap_or_default(),
            input: calldata,
            access_list: overrides
                .access_list
                .iter()
                .map(AccessListEntry::from)
                .collect(),
            from,
            ..Default::default()
        };
        tx.gas_price = tx.max_fee_per_gas.unwrap_or_default();
        if let Some(blobs_bundle) = &overrides.blobs_bundle {
            tx.blob_versioned_hashes = blobs_bundle.generate_versioned_hashes();
            add_blobs_to_generic_tx(&mut tx, blobs_bundle);
        }
        tx.gas = Some(match overrides.gas_limit {
            Some(gas) => gas,
            None => self.estimate_gas(tx.clone()).await?,
        });

        Ok(tx)
    }

    async fn get_nonce_from_overrides_or_rpc(
        &self,
        overrides: &Overrides,
        address: Address,
    ) -> Result<u64, EthClientError> {
        if let Some(nonce) = overrides.nonce {
            return Ok(nonce);
        }
        self.get_nonce(address, BlockIdentifier::Tag(BlockTag::Latest))
            .await
    }

    pub async fn get_last_committed_batch(
        &self,
        on_chain_proposer_address: Address,
    ) -> Result<u64, EthClientError> {
        self._call_variable(b"lastCommittedBatch()", on_chain_proposer_address)
            .await
    }

    pub async fn get_last_verified_batch(
        &self,
        on_chain_proposer_address: Address,
    ) -> Result<u64, EthClientError> {
        self._call_variable(b"lastVerifiedBatch()", on_chain_proposer_address)
            .await
    }

    pub async fn get_sp1_vk(
        &self,
        on_chain_proposer_address: Address,
    ) -> Result<[u8; 32], EthClientError> {
        self._call_bytes32_variable(b"SP1_VERIFICATION_KEY()", on_chain_proposer_address)
            .await
    }

    pub async fn get_last_fetched_l1_block(
        &self,
        common_bridge_address: Address,
    ) -> Result<u64, EthClientError> {
        self._call_variable(b"lastFetchedL1Block()", common_bridge_address)
            .await
    }

    pub async fn get_pending_privileged_transactions(
        &self,
        common_bridge_address: Address,
    ) -> Result<Vec<H256>, EthClientError> {
        let response = self
            ._generic_call(b"getPendingTransactionHashes()", common_bridge_address)
            .await?;
        Self::from_hex_string_to_h256_array(&response)
    }

    pub fn from_hex_string_to_h256_array(hex_string: &str) -> Result<Vec<H256>, EthClientError> {
        let bytes = hex::decode(hex_string.strip_prefix("0x").unwrap_or(hex_string))
            .map_err(|_| EthClientError::Custom("Invalid hex string".to_owned()))?;

        // The ABI encoding for dynamic arrays is:
        // 1. Offset to data (32 bytes)
        // 2. Length of array (32 bytes)
        // 3. Array elements (each 32 bytes)
        if bytes.len() < 64 {
            return Err(EthClientError::Custom("Response too short".to_owned()));
        }

        // Get the offset (should be 0x20 for simple arrays)
        let offset = U256::from_big_endian(&bytes[0..32]).as_usize();

        // Get the length of the array
        let length = U256::from_big_endian(&bytes[offset..offset + 32]).as_usize();

        // Calculate the start of the array data
        let data_start = offset + 32;
        let data_end = data_start + (length * 32);

        if data_end > bytes.len() {
            return Err(EthClientError::Custom("Invalid array length".to_owned()));
        }

        // Convert the slice directly to H256 array
        bytes[data_start..data_end]
            .chunks_exact(32)
            .map(|chunk| Ok(H256::from_slice(chunk)))
            .collect()
    }

    async fn _generic_call(
        &self,
        selector: &[u8],
        contract_address: Address,
    ) -> Result<String, EthClientError> {
        let selector = keccak(selector)
            .as_bytes()
            .get(..4)
            .ok_or(EthClientError::Custom("Failed to get selector.".to_owned()))?
            .to_vec();

        let mut calldata = Vec::new();
        calldata.extend_from_slice(&selector);

        let leading_zeros = 32 - ((calldata.len() - 4) % 32);
        calldata.extend(vec![0; leading_zeros]);

        let hex_string = self
            .call(contract_address, calldata.into(), Overrides::default())
            .await?;

        Ok(hex_string)
    }

    async fn _call_variable(
        &self,
        selector: &[u8],
        on_chain_proposer_address: Address,
    ) -> Result<u64, EthClientError> {
        let hex_string = self
            ._generic_call(selector, on_chain_proposer_address)
            .await?;

        let value = from_hex_string_to_u256(&hex_string)?
            .try_into()
            .map_err(|_| {
                EthClientError::Custom("Failed to convert from_hex_string_to_u256()".to_owned())
            })?;

        Ok(value)
    }

    async fn _call_address_variable(
        eth_client: &EthClient,
        selector: &[u8],
        on_chain_proposer_address: Address,
    ) -> Result<Address, EthClientError> {
        let hex_string =
            Self::_generic_call(eth_client, selector, on_chain_proposer_address).await?;

        let hex_str = &hex_string.strip_prefix("0x").ok_or(EthClientError::Custom(
            "Couldn't strip prefix from request.".to_owned(),
        ))?[24..]; // Get the needed bytes

        let value = Address::from_str(hex_str)
            .map_err(|_| EthClientError::Custom("Failed to convert from_str()".to_owned()))?;
        Ok(value)
    }

    async fn _call_bytes32_variable(
        &self,
        selector: &[u8],
        contract_address: Address,
    ) -> Result<[u8; 32], EthClientError> {
        let hex_string = self._generic_call(selector, contract_address).await?;

        let hex = hex_string.strip_prefix("0x").ok_or(EthClientError::Custom(
            "Couldn't strip '0x' prefix from hex string".to_owned(),
        ))?;

        let bytes = hex::decode(hex)
            .map_err(|e| EthClientError::Custom(format!("Failed to decode hex string: {e}")))?;

        let arr: [u8; 32] = bytes.try_into().map_err(|_| {
            EthClientError::Custom("Failed to convert bytes to [u8; 32]".to_owned())
        })?;

        Ok(arr)
    }

    pub async fn wait_for_transaction_receipt(
        &self,
        tx_hash: H256,
        max_retries: u64,
    ) -> Result<RpcReceipt, EthClientError> {
        let mut receipt = self.get_transaction_receipt(tx_hash).await?;
        let mut r#try = 1;
        while receipt.is_none() {
            println!("[{try}/{max_retries}] Retrying to get transaction receipt for {tx_hash:#x}");

            if max_retries == r#try {
                return Err(EthClientError::Custom(format!(
                    "Transaction receipt for {tx_hash:#x} not found after {max_retries} retries"
                )));
            }
            r#try += 1;

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            receipt = self.get_transaction_receipt(tx_hash).await?;
        }
        receipt.ok_or(EthClientError::Custom(
            "Transaction receipt is None".to_owned(),
        ))
    }

    pub async fn get_message_proof(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Vec<L1MessageProof>>, EthClientError> {
        use errors::GetMessageProofError;
        let params = Some(vec![json!(format!("{:#x}", transaction_hash))]);
        let request = RpcRequest::new("ethrex_getMessageProof", params);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetMessageProofError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetMessageProofError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn wait_for_message_proof(
        &self,
        transaction_hash: H256,
        max_retries: u64,
    ) -> Result<Vec<L1MessageProof>, EthClientError> {
        let mut message_proof = self.get_message_proof(transaction_hash).await?;
        let mut r#try = 1;
        while message_proof.is_none() {
            println!(
                "[{try}/{max_retries}] Retrying to get message proof for tx {transaction_hash:#x}"
            );

            if max_retries == r#try {
                return Err(EthClientError::Custom(format!(
                    "L1Message proof for tx {transaction_hash:#x} not found after {max_retries} retries"
                )));
            }
            r#try += 1;

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            message_proof = self.get_message_proof(transaction_hash).await?;
        }
        message_proof.ok_or(EthClientError::Custom("L1Message proof is None".to_owned()))
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

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetWitnessError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetWitnessError::RPCError(error_response.error.message).into())
            }
        }
    }

    async fn get_fee_from_override_or_get_gas_price(
        &self,
        maybe_gas_fee: Option<u64>,
    ) -> Result<u64, EthClientError> {
        if let Some(gas_fee) = maybe_gas_fee {
            return Ok(gas_fee);
        }
        self.get_gas_price()
            .await?
            .try_into()
            .map_err(|_| EthClientError::Custom("Failed to get gas for fee".to_owned()))
    }

    async fn priority_fee_from_override_or_rpc(
        &self,
        maybe_priority_fee: Option<u64>,
    ) -> Result<u64, EthClientError> {
        if let Some(priority_fee) = maybe_priority_fee {
            return Ok(priority_fee);
        }

        if let Ok(priority_fee) = self.get_max_priority_fee().await {
            if let Ok(priority_fee_u64) = priority_fee.try_into() {
                return Ok(priority_fee_u64);
            }
        }

        self.get_fee_from_override_or_get_gas_price(None).await
    }

    pub async fn tx_pool_content(&self) -> Result<MempoolContent, EthClientError> {
        let request = RpcRequest::new("txpool_content", None);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(TxPoolContentError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(TxPoolContentError::RPCError(error_response.error.message).into())
            }
        }
    }

    pub async fn get_batch_by_number(&self, batch_number: u64) -> Result<RpcBatch, EthClientError> {
        let params = Some(vec![json!(format!("{batch_number:#x}")), json!(true)]);
        let request = RpcRequest::new("ethrex_getBatchByNumber", params);

        match self.send_request(request).await? {
            RpcResponse::Success(result) => serde_json::from_value(result.result)
                .map_err(GetBatchByNumberError::SerdeJSONError)
                .map_err(EthClientError::from),
            RpcResponse::Error(error_response) => {
                Err(GetBatchByNumberError::RPCError(error_response.error.message).into())
            }
        }
    }
}

pub fn from_hex_string_to_u256(hex_string: &str) -> Result<U256, EthClientError> {
    let hex_string = hex_string.strip_prefix("0x").ok_or(EthClientError::Custom(
        "Couldn't strip prefix from request.".to_owned(),
    ))?;

    if hex_string.is_empty() {
        return Err(EthClientError::Custom(
            "Failed to fetch last_committed_block. Manual intervention required.".to_owned(),
        ));
    }

    let value = U256::from_str_radix(hex_string, 16).map_err(|_| {
        EthClientError::Custom(
            "Failed to parse after call, U256::from_str_radix failed.".to_owned(),
        )
    })?;
    Ok(value)
}

pub fn get_address_from_secret_key(secret_key: &SecretKey) -> Result<Address, EthClientError> {
    let public_key = secret_key
        .public_key(secp256k1::SECP256K1)
        .serialize_uncompressed();
    let hash = keccak(&public_key[1..]);

    // Get the last 20 bytes of the hash
    let address_bytes: [u8; 20] = hash
        .as_ref()
        .get(12..32)
        .ok_or(EthClientError::Custom(
            "Failed to get_address_from_secret_key: error slicing address_bytes".to_owned(),
        ))?
        .try_into()
        .map_err(|err| {
            EthClientError::Custom(format!("Failed to get_address_from_secret_key: {err}"))
        })?;

    Ok(Address::from(address_bytes))
}

pub fn add_blobs_to_generic_tx(tx: &mut GenericTransaction, bundle: &BlobsBundle) {
    tx.blobs = bundle
        .blobs
        .iter()
        .map(|blob| Bytes::copy_from_slice(blob))
        .collect()
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GetTransactionByHashTransaction {
    #[serde(default, with = "ethrex_common::serde_utils::u64::hex_str")]
    pub chain_id: u64,
    #[serde(default, with = "ethrex_common::serde_utils::u64::hex_str")]
    pub nonce: u64,
    #[serde(default, with = "ethrex_common::serde_utils::u64::hex_str")]
    pub max_priority_fee_per_gas: u64,
    #[serde(default, with = "ethrex_common::serde_utils::u64::hex_str")]
    pub max_fee_per_gas: u64,
    #[serde(default, with = "ethrex_common::serde_utils::u64::hex_str")]
    pub gas_limit: u64,
    #[serde(default)]
    pub to: Address,
    #[serde(default)]
    pub value: U256,
    #[serde(default, with = "ethrex_common::serde_utils::vec_u8", alias = "input")]
    pub data: Vec<u8>,
    #[serde(default)]
    pub access_list: Vec<(Address, Vec<H256>)>,
    #[serde(default)]
    pub r#type: TxType,
    #[serde(default)]
    pub signature_y_parity: bool,
    #[serde(default, with = "ethrex_common::serde_utils::u64::hex_str")]
    pub signature_r: u64,
    #[serde(default, with = "ethrex_common::serde_utils::u64::hex_str")]
    pub signature_s: u64,
    #[serde(default)]
    pub block_number: U256,
    #[serde(default)]
    pub block_hash: H256,
    #[serde(default)]
    pub from: Address,
    #[serde(default)]
    pub hash: H256,
    #[serde(default, with = "ethrex_common::serde_utils::u64::hex_str")]
    pub transaction_index: u64,
    #[serde(default)]
    pub blob_versioned_hashes: Option<Vec<H256>>,
}

impl fmt::Display for GetTransactionByHashTransaction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            r#"
chain_id: {},
nonce: {},
max_priority_fee_per_gas: {},
max_fee_per_gas: {},
gas_limit: {},
to: {:#x},
value: {},
data: {:#?},
access_list: {:#?},
type: {:?},
signature_y_parity: {},
signature_r: {:x},
signature_s: {:x},
block_number: {},
block_hash: {:#x},
from: {:#x},
hash: {:#x},
transaction_index: {}"#,
            self.chain_id,
            self.nonce,
            self.max_priority_fee_per_gas,
            self.max_fee_per_gas,
            self.gas_limit,
            self.to,
            self.value,
            self.data,
            self.access_list,
            self.r#type,
            self.signature_y_parity,
            self.signature_r,
            self.signature_s,
            self.block_number,
            self.block_hash,
            self.from,
            self.hash,
            self.transaction_index,
        )?;

        if let Some(blob_versioned_hashes) = &self.blob_versioned_hashes {
            write!(f, "\nblob_versioned_hashes: {blob_versioned_hashes:#?}")?;
        }

        fmt::Result::Ok(())
    }
}
