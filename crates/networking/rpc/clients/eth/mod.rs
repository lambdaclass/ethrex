use std::fmt;

use crate::{
    clients::eth::errors::{CallError, GetPeerCountError, GetWitnessError, TxPoolContentError},
    debug::execution_witness::RpcExecutionWitness,
    mempool::MempoolContent,
    types::{
        block::RpcBlock,
        block_identifier::BlockIdentifier,
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
    types::{BlobsBundle, Block, GenericTransaction, TxKind, TxType},
    utils::decode_hex,
};
use ethrex_rlp::decode::RLPDecode;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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

    pub async fn send_request(&self, request: RpcRequest) -> Result<RpcResponse, EthClientError> {
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
                Err(EstimateGasError::RPCError(error_response.error.message.to_string()).into())
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
