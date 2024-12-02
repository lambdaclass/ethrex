use std::{fmt, time::Duration};

use crate::utils::config::eth::EthConfig;
use bytes::Bytes;
use errors::{
    EstimateGasPriceError, EthClientError, GetBalanceError, GetBlockByHashError,
    GetBlockByNumberError, GetBlockNumberError, GetGasPriceError, GetLogsError, GetNonceError,
    GetTransactionByHashError, GetTransactionReceiptError, SendRawTransactionError,
};
use eth_sender::Overrides;
use ethereum_types::{Address, H256, U256};
use ethrex_core::{
    types::{
        BlobsBundle, EIP1559Transaction, EIP4844Transaction, GenericTransaction,
        PrivilegedL2Transaction, PrivilegedTxType, Signable, TxKind, TxType,
    },
    H160,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::{
    types::{
        block::RpcBlock,
        receipt::{RpcLog, RpcReceipt},
        transaction::WrappedEIP4844Transaction,
    },
    utils::{RpcErrorResponse, RpcRequest, RpcRequestId, RpcSuccessResponse},
};
use keccak_hash::keccak;
use reqwest::Client;
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ops::Div;
use tokio::time::{sleep, Instant};
use tracing::warn;

use super::get_address_from_secret_key;

pub mod errors;
pub mod eth_sender;

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum RpcResponse {
    Success(RpcSuccessResponse),
    Error(RpcErrorResponse),
}

#[derive(Debug, Clone)]
pub struct EthClient {
    client: Client,
    pub url: String,
}

#[derive(Debug, Clone)]
pub enum WrappedTransaction {
    EIP4844(WrappedEIP4844Transaction),
    EIP1559(EIP1559Transaction),
    L2(PrivilegedL2Transaction),
}

pub enum BlockByNumber {
    Number(u64),
    Latest,
    Earliest,
    Pending,
}

// 0x08c379a0 == Error(String)
pub const ERROR_FUNCTION_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];

impl EthClient {
    pub fn new(url: &str) -> Self {
        Self {
            client: Client::new(),
            url: url.to_string(),
        }
    }

    pub fn new_from_config(config: EthConfig) -> Self {
        Self {
            client: Client::new(),
            url: config.rpc_url,
        }
    }

    async fn send_request(&self, request: RpcRequest) -> Result<RpcResponse, EthClientError> {
        self.client
            .post(&self.url)
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
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_sendRawTransaction".to_string(),
            params: Some(vec![json!("0x".to_string() + &hex::encode(data))]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(SendRawTransactionError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(SendRawTransactionError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn send_eip1559_transaction(
        &self,
        tx: &EIP1559Transaction,
        private_key: &SecretKey,
    ) -> Result<H256, EthClientError> {
        let signed_tx = tx.sign(private_key);

        let mut encoded_tx = signed_tx.encode_to_vec();
        encoded_tx.insert(0, TxType::EIP1559.into());

        self.send_raw_transaction(encoded_tx.as_slice()).await
    }

    pub async fn send_eip4844_transaction(
        &self,
        wrapped_tx: &WrappedEIP4844Transaction,
        private_key: &SecretKey,
    ) -> Result<H256, EthClientError> {
        let mut wrapped_tx = wrapped_tx.clone();
        wrapped_tx.tx.sign_inplace(private_key);

        let mut encoded_tx = wrapped_tx.encode_to_vec();
        encoded_tx.insert(0, TxType::EIP4844.into());

        self.send_raw_transaction(encoded_tx.as_slice()).await
    }

    /// Sends a [WrappedTransaction] with retries and gas bumping.
    ///
    /// The total wait time for each retry is determined by dividing the `max_seconds_to_wait`
    /// by the `retries` parameter. The transaction is sent again with a gas bump if the receipt
    /// is not confirmed within each retry period.
    ///
    /// seconds_per_retry = max_seconds_to_wait / retries;
    pub async fn send_wrapped_transaction_with_retry(
        &self,
        wrapped_tx: &WrappedTransaction,
        private_key: &SecretKey,
        max_seconds_to_wait: u64,
        retries: u64,
    ) -> Result<H256, EthClientError> {
        let tx_hash_res = match wrapped_tx {
            WrappedTransaction::EIP4844(wrapped_eip4844_transaction) => {
                self.send_eip4844_transaction(wrapped_eip4844_transaction, private_key)
                    .await
            }
            WrappedTransaction::EIP1559(eip1559_transaction) => {
                self.send_eip1559_transaction(eip1559_transaction, private_key)
                    .await
            }
            WrappedTransaction::L2(privileged_l2_transaction) => {
                self.send_privileged_l2_transaction(privileged_l2_transaction, private_key)
                    .await
            }
        };

        // Check if the tx is `already known`, bump gas and resend it.
        let mut tx_hash = match tx_hash_res {
            Ok(hash) => hash,
            Err(e) => {
                let error = format!("{e}");
                if error.contains("already known")
                    || error.contains("replacement transaction underpriced")
                {
                    H256::zero()
                } else {
                    return Err(e);
                }
            }
        };

        let mut wrapped_tx = wrapped_tx.clone();

        let seconds_per_retry = max_seconds_to_wait / retries;
        let timer_total = Instant::now();

        for r in 0..retries {
            // Check if we are not waiting more than needed.
            if timer_total.elapsed().as_secs() > max_seconds_to_wait {
                return Err(EthClientError::Custom(
                    "TimeOut: Failed to send_wrapped_transaction_with_retry".to_owned(),
                ));
            }

            // Wait for the receipt with some time between retries.
            let timer_per_retry = Instant::now();
            while timer_per_retry.elapsed().as_secs() < seconds_per_retry {
                match self.get_transaction_receipt(tx_hash).await? {
                    Some(_) => return Ok(tx_hash),
                    None => sleep(Duration::from_secs(1)).await,
                }
            }

            // If receipt is not found after the time period, increase gas and resend the transaction.
            tx_hash = match &mut wrapped_tx {
                WrappedTransaction::EIP4844(wrapped_eip4844_transaction) => {
                    warn!("Resending EIP4844Transaction, attempts [{r}/{retries}]");
                    self.bump_and_resend_eip4844(wrapped_eip4844_transaction, private_key)
                        .await?
                }
                WrappedTransaction::EIP1559(eip1559_transaction) => {
                    warn!("Resending EIP1559Transaction, attempts [{r}/{retries}]");
                    self.bump_and_resend_eip1559(eip1559_transaction, private_key)
                        .await?
                }
                WrappedTransaction::L2(privileged_l2_transaction) => {
                    warn!("Resending PrivilegedL2Transaction, attempts [{r}/{retries}]");
                    self.bump_and_resend_privileged_l2(privileged_l2_transaction, private_key)
                        .await?
                }
            };
        }

        // If the loop ends without success, return a timeout error
        Err(EthClientError::Custom(
            "Max retries exceeded while waiting for transaction receipt".to_owned(),
        ))
    }

    pub async fn bump_and_resend_eip1559(
        &self,
        tx: &mut EIP1559Transaction,
        private_key: &SecretKey,
    ) -> Result<H256, EthClientError> {
        let from = get_address_from_secret_key(private_key).map_err(|e| {
            EthClientError::Custom(format!("Failed to get_address_from_secret_key: {e}"))
        })?;
        // Sometimes the penalty is a 100%
        // Increase max fee per gas by 110% (set it to 210% of the original)
        self.bump_eip1559(tx, 110);
        let wrapped_tx = &mut WrappedTransaction::EIP1559(tx.clone());
        self.estimate_gas_for_wrapped_tx(wrapped_tx, from).await?;

        if let WrappedTransaction::EIP1559(eip1559) = wrapped_tx {
            tx.max_fee_per_gas = eip1559.max_fee_per_gas;
            tx.max_priority_fee_per_gas = eip1559.max_fee_per_gas;
            tx.gas_limit = eip1559.gas_limit;
        }
        self.send_eip1559_transaction(tx, private_key).await
    }

    /// Increase max fee per gas by percentage% (set it to (100+percentage)% of the original)
    pub fn bump_eip1559(&self, tx: &mut EIP1559Transaction, percentage: u64) {
        tx.max_fee_per_gas = (tx.max_fee_per_gas * (100 + percentage)) / 100;
        tx.max_priority_fee_per_gas += (tx.max_priority_fee_per_gas * (100 + percentage)) / 100;
    }

    pub async fn bump_and_resend_eip4844(
        &self,
        wrapped_tx: &mut WrappedEIP4844Transaction,
        private_key: &SecretKey,
    ) -> Result<H256, EthClientError> {
        let from = get_address_from_secret_key(private_key).map_err(|e| {
            EthClientError::Custom(format!("Failed to get_address_from_secret_key: {e}"))
        })?;
        // Sometimes the penalty is a 100%
        // Increase max fee per gas by 110% (set it to 210% of the original)
        self.bump_eip4844(wrapped_tx, 110);
        let wrapped_eip4844 = &mut WrappedTransaction::EIP4844(wrapped_tx.clone());
        self.estimate_gas_for_wrapped_tx(wrapped_eip4844, from)
            .await?;

        if let WrappedTransaction::EIP4844(eip4844) = wrapped_eip4844 {
            wrapped_tx.tx.max_fee_per_gas = eip4844.tx.max_fee_per_gas;
            wrapped_tx.tx.max_priority_fee_per_gas = eip4844.tx.max_fee_per_gas;
            wrapped_tx.tx.gas = eip4844.tx.gas;
            wrapped_tx.tx.max_fee_per_blob_gas = eip4844.tx.max_fee_per_blob_gas;
        }
        self.send_eip4844_transaction(wrapped_tx, private_key).await
    }

    /// Increase max fee per gas by percentage% (set it to (100+percentage)% of the original)
    pub fn bump_eip4844(&self, wrapped_tx: &mut WrappedEIP4844Transaction, percentage: u64) {
        wrapped_tx.tx.max_fee_per_gas = (wrapped_tx.tx.max_fee_per_gas * (100 + percentage)) / 100;
        wrapped_tx.tx.max_priority_fee_per_gas +=
            (wrapped_tx.tx.max_priority_fee_per_gas * (100 + percentage)) / 100;
        let factor = 1 + (percentage / 100) * 10;
        wrapped_tx.tx.max_fee_per_blob_gas = wrapped_tx
            .tx
            .max_fee_per_blob_gas
            .saturating_mul(U256::from(factor))
            .div(10);
    }

    pub async fn bump_and_resend_privileged_l2(
        &self,
        tx: &mut PrivilegedL2Transaction,
        private_key: &SecretKey,
    ) -> Result<H256, EthClientError> {
        let from = get_address_from_secret_key(private_key).map_err(|e| {
            EthClientError::Custom(format!("Failed to get_address_from_secret_key: {e}"))
        })?;
        // Sometimes the penalty is a 100%
        // Increase max fee per gas by 110% (set it to 210% of the original)
        self.bump_privileged_l2(tx, 110);
        let wrapped_tx = &mut WrappedTransaction::L2(tx.clone());
        self.estimate_gas_for_wrapped_tx(wrapped_tx, from).await?;
        if let WrappedTransaction::L2(l2_tx) = wrapped_tx {
            tx.max_fee_per_gas = l2_tx.max_fee_per_gas;
            tx.max_priority_fee_per_gas = l2_tx.max_fee_per_gas;
            tx.gas_limit = l2_tx.gas_limit;
        }
        self.send_privileged_l2_transaction(tx, private_key).await
    }

    /// Increase max fee per gas by percentage% (set it to (100+percentage)% of the original)
    pub fn bump_privileged_l2(&self, tx: &mut PrivilegedL2Transaction, percentage: u64) {
        tx.max_fee_per_gas = (tx.max_fee_per_gas * (100 + percentage)) / 100;
        tx.max_priority_fee_per_gas += (tx.max_priority_fee_per_gas * (100 + percentage)) / 100;
    }

    pub async fn send_privileged_l2_transaction(
        &self,
        tx: &PrivilegedL2Transaction,
        private_key: &SecretKey,
    ) -> Result<H256, EthClientError> {
        let signed_tx = tx.sign(private_key);

        let mut encoded_tx = signed_tx.encode_to_vec();
        encoded_tx.insert(0, TxType::Privileged.into());

        self.send_raw_transaction(encoded_tx.as_slice()).await
    }

    pub async fn estimate_gas(
        &self,
        transaction: GenericTransaction,
    ) -> Result<u64, EthClientError> {
        let to = match transaction.to {
            TxKind::Call(addr) => addr,
            TxKind::Create => Address::zero(),
        };
        let mut data = json!({
            "to": format!("{to:#x}"),
            "input": format!("0x{:#x}", transaction.input),
            "from": format!("{:#x}", transaction.from),
            "value": format!("{:#x}", transaction.value),
        });

        // Add the nonce just if present, otherwise the RPC will use the latest nonce
        if let Some(nonce) = transaction.nonce {
            if let Value::Object(ref mut map) = data {
                map.insert("nonce".to_owned(), json!(format!("{nonce:#x}")));
            }
        }

        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_estimateGas".to_string(),
            params: Some(vec![data, json!("latest")]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => u64::from_str_radix(
                serde_json::from_value::<String>(result.result)
                    .map_err(EstimateGasPriceError::SerdeJSONError)?
                    .get(2..)
                    .ok_or(EstimateGasPriceError::Custom(
                        "Failed to slice index response in estimate_gas".to_owned(),
                    ))?,
                16,
            )
            .map_err(EstimateGasPriceError::ParseIntError)
            .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
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
                        let string_data = abi_decoded_error_data
                            .get(68..68 + string_length.as_usize())
                            .ok_or(EthClientError::Custom(
                                "Failed to slice index abi_decoded_error_data in estimate_gas"
                                    .to_owned(),
                            ))?;
                        String::from_utf8(string_data.to_vec()).map_err(|_| {
                            EthClientError::Custom(
                                "Failed to String::from_utf8 in estimate_gas".to_owned(),
                            )
                        })?
                    }
                } else {
                    "unknown error".to_owned()
                };
                Err(EstimateGasPriceError::RPCError(format!(
                    "{}: {}",
                    error_response.error.message, error_data
                ))
                .into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get_gas_price(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_gasPrice".to_string(),
            params: None,
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(GetGasPriceError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(GetGasPriceError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get_nonce(&self, address: Address) -> Result<u64, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_getTransactionCount".to_string(),
            params: Some(vec![json!(format!("{address:#x}")), json!("latest")]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => u64::from_str_radix(
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
            Ok(RpcResponse::Error(error_response)) => {
                Err(GetNonceError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get_block_number(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_blockNumber".to_string(),
            params: None,
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(GetBlockNumberError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(GetBlockNumberError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get_block_by_hash(&self, block_hash: H256) -> Result<RpcBlock, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_getBlockByHash".to_string(),
            params: Some(vec![json!(block_hash), json!(true)]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(GetBlockByHashError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(GetBlockByHashError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    /// Fetches a block from the Ethereum blockchain by its number or the latest/earliest/pending block.
    /// If no `block_number` is provided, get the latest.
    pub async fn get_block_by_number(
        &self,
        block: BlockByNumber,
    ) -> Result<RpcBlock, EthClientError> {
        let r = match block {
            BlockByNumber::Number(n) => format!("{n:#x}"),
            BlockByNumber::Latest => "latest".to_owned(),
            BlockByNumber::Earliest => "earliest".to_owned(),
            BlockByNumber::Pending => "pending".to_owned(),
        };
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_getBlockByNumber".to_string(),
            // With false it just returns the hash of the transactions.
            params: Some(vec![json!(r), json!(false)]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(GetBlockByNumberError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(GetBlockByNumberError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get_logs(
        &self,
        from_block: U256,
        to_block: U256,
        address: Address,
        topic: H256,
    ) -> Result<Vec<RpcLog>, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_getLogs".to_string(),
            params: Some(vec![serde_json::json!(
                {
                    "fromBlock": format!("{:#x}", from_block),
                    "toBlock": format!("{:#x}", to_block),
                    "address": format!("{:#x}", address),
                    "topics": [format!("{:#x}", topic)]
                }
            )]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(GetLogsError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(GetLogsError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get_transaction_receipt(
        &self,
        tx_hash: H256,
    ) -> Result<Option<RpcReceipt>, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_getTransactionReceipt".to_string(),
            params: Some(vec![json!(format!("{:#x}", tx_hash))]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(GetTransactionReceiptError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(GetTransactionReceiptError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get_balance(&self, address: Address) -> Result<U256, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_getBalance".to_string(),
            params: Some(vec![json!(format!("{:#x}", address)), json!("latest")]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(GetBalanceError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(GetBalanceError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get_chain_id(&self) -> Result<U256, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_chainId".to_string(),
            params: None,
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(GetBalanceError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(GetBalanceError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get_transaction_by_hash(
        &self,
        tx_hash: H256,
    ) -> Result<Option<GetTransactionByHashTransaction>, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_getTransactionByHash".to_string(),
            params: Some(vec![json!(format!("{tx_hash:#x}"))]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(GetTransactionByHashError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(GetTransactionByHashError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn estimate_gas_for_wrapped_tx(
        &self,
        wrapped_tx: &mut WrappedTransaction,
        from: H160,
    ) -> Result<u64, EthClientError> {
        loop {
            let mut transaction = match wrapped_tx {
                WrappedTransaction::EIP4844(wrapped_eip4844_transaction) => {
                    GenericTransaction::from(wrapped_eip4844_transaction.clone().tx)
                }
                WrappedTransaction::EIP1559(eip1559_transaction) => {
                    GenericTransaction::from(eip1559_transaction.clone())
                }
                WrappedTransaction::L2(privileged_l2_transaction) => {
                    GenericTransaction::from(privileged_l2_transaction.clone())
                }
            };

            transaction.from = from;

            match self.estimate_gas(transaction).await {
                Ok(gas_limit) => return Ok(gas_limit),
                Err(e) => {
                    let error = format!("{e}").to_owned();
                    if error.contains("transaction underpriced") {
                        match wrapped_tx {
                            WrappedTransaction::EIP4844(wrapped_eip4844_transaction) => {
                                self.bump_eip4844(wrapped_eip4844_transaction, 110);
                            }
                            WrappedTransaction::EIP1559(eip1559_transaction) => {
                                self.bump_eip1559(eip1559_transaction, 110);
                            }
                            WrappedTransaction::L2(privileged_l2_transaction) => {
                                self.bump_privileged_l2(privileged_l2_transaction, 110);
                            }
                        };
                        continue;
                    }
                    return Err(e);
                }
            };
        }
    }

    /// Build an EIP1559 transaction with the given parameters.
    /// Either `overrides.nonce` or `overrides.from` must be provided.
    /// If `overrides.gas_price`, `overrides.chain_id` or `overrides.gas_price`
    /// are not provided, the client will fetch them from the network.
    /// If `overrides.gas_limit` is not provided, the client will estimate the tx cost.
    pub async fn build_eip1559_transaction(
        &self,
        to: Address,
        from: Address,
        calldata: Bytes,
        overrides: Overrides,
        bump_retries: u64,
    ) -> Result<EIP1559Transaction, EthClientError> {
        let get_gas_price;
        let mut tx = EIP1559Transaction {
            to: TxKind::Call(to),
            chain_id: if let Some(chain_id) = overrides.chain_id {
                chain_id
            } else {
                self.get_chain_id().await?.as_u64()
            },
            nonce: self
                .get_nonce_from_overrides_or_rpc(&overrides, from)
                .await?,
            max_priority_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                get_gas_price = gas_price;
                gas_price
            } else {
                get_gas_price = self.get_gas_price().await?.as_u64();
                get_gas_price
            },
            max_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                gas_price
            } else {
                get_gas_price
            },
            value: overrides.value.unwrap_or_default(),
            data: calldata,
            access_list: overrides.access_list,
            ..Default::default()
        };

        let mut wrapped_tx;

        if let Some(overrides_gas_limit) = overrides.gas_limit {
            tx.gas_limit = overrides_gas_limit;
            Ok(tx)
        } else {
            let mut retry = 0_u64;
            while retry < bump_retries {
                wrapped_tx = WrappedTransaction::EIP1559(tx.clone());
                match self
                    .estimate_gas_for_wrapped_tx(&mut wrapped_tx, from)
                    .await
                {
                    Ok(gas_limit) => {
                        // Estimation succeeded.
                        tx.gas_limit = gas_limit;
                        return Ok(tx);
                    }
                    Err(e) => {
                        let error = format!("{e}");
                        if error.contains("replacement transaction underpriced") {
                            warn!("Bumping gas while building: already known");
                            retry += 1;
                            self.bump_eip1559(&mut tx, 110);
                            continue;
                        }
                        return Err(e);
                    }
                }
            }
            Err(EthClientError::EstimateGasPriceError(
                EstimateGasPriceError::Custom(
                    "Exceeded maximum retries while estimating gas.".to_string(),
                ),
            ))
        }
    }

    /// Build an EIP4844 transaction with the given parameters.
    /// Either `overrides.nonce` or `overrides.from` must be provided.
    /// If `overrides.gas_price`, `overrides.chain_id` or `overrides.gas_price`
    /// are not provided, the client will fetch them from the network.
    /// If `overrides.gas_limit` is not provided, the client will estimate the tx cost.
    pub async fn build_eip4844_transaction(
        &self,
        to: Address,
        from: Address,
        calldata: Bytes,
        overrides: Overrides,
        blobs_bundle: BlobsBundle,
        bump_retries: u64,
    ) -> Result<WrappedEIP4844Transaction, EthClientError> {
        let blob_versioned_hashes = blobs_bundle.generate_versioned_hashes();

        let get_gas_price;
        let tx = EIP4844Transaction {
            to,
            chain_id: if let Some(chain_id) = overrides.chain_id {
                chain_id
            } else {
                self.get_chain_id().await?.as_u64()
            },
            nonce: self
                .get_nonce_from_overrides_or_rpc(&overrides, from)
                .await?,
            max_priority_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                get_gas_price = gas_price;
                gas_price
            } else {
                get_gas_price = self.get_gas_price().await?.as_u64();
                get_gas_price
            },
            max_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                gas_price
            } else {
                get_gas_price
            },
            value: overrides.value.unwrap_or_default(),
            data: calldata,
            access_list: overrides.access_list,
            max_fee_per_blob_gas: overrides.gas_price_per_blob.unwrap_or_default(),
            blob_versioned_hashes,
            ..Default::default()
        };

        let mut wrapped_eip4844 = WrappedEIP4844Transaction { tx, blobs_bundle };
        let mut wrapped_tx;
        if let Some(overrides_gas_limit) = overrides.gas_limit {
            wrapped_eip4844.tx.gas = overrides_gas_limit;
            Ok(wrapped_eip4844)
        } else {
            let mut retry = 0_u64;
            while retry < bump_retries {
                wrapped_tx = WrappedTransaction::EIP4844(wrapped_eip4844.clone());

                match self
                    .estimate_gas_for_wrapped_tx(&mut wrapped_tx, from)
                    .await
                {
                    Ok(gas_limit) => {
                        // Estimation succeeded.
                        wrapped_eip4844.tx.gas = gas_limit;
                        return Ok(wrapped_eip4844);
                    }
                    Err(e) => {
                        let error = format!("{e}");
                        if error.contains("already known") {
                            warn!("Bumping gas while building: already known");
                            retry += 1;
                            self.bump_eip4844(&mut wrapped_eip4844, 110);
                            continue;
                        }
                        return Err(e);
                    }
                }
            }
            Err(EthClientError::EstimateGasPriceError(
                EstimateGasPriceError::Custom(
                    "Exceeded maximum retries while estimating gas.".to_string(),
                ),
            ))
        }
    }

    /// Build a PrivilegedL2 transaction with the given parameters.
    /// Either `overrides.nonce` or `overrides.from` must be provided.
    /// If `overrides.gas_price`, `overrides.chain_id` or `overrides.gas_price`
    /// are not provided, the client will fetch them from the network.
    /// If `overrides.gas_limit` is not provided, the client will estimate the tx cost.
    pub async fn build_privileged_transaction(
        &self,
        tx_type: PrivilegedTxType,
        to: Address,
        from: Address,
        calldata: Bytes,
        overrides: Overrides,
        bump_retries: u64,
    ) -> Result<PrivilegedL2Transaction, EthClientError> {
        let get_gas_price;
        let mut tx = PrivilegedL2Transaction {
            tx_type,
            to: TxKind::Call(to),
            chain_id: if let Some(chain_id) = overrides.chain_id {
                chain_id
            } else {
                self.get_chain_id().await?.as_u64()
            },
            nonce: self
                .get_nonce_from_overrides_or_rpc(&overrides, from)
                .await?,
            max_priority_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                get_gas_price = gas_price;
                gas_price
            } else {
                get_gas_price = self.get_gas_price().await?.as_u64();
                get_gas_price
            },
            max_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                gas_price
            } else {
                get_gas_price
            },
            value: overrides.value.unwrap_or_default(),
            data: calldata,
            access_list: overrides.access_list,
            ..Default::default()
        };

        let mut wrapped_tx;

        if let Some(overrides_gas_limit) = overrides.gas_limit {
            tx.gas_limit = overrides_gas_limit;
            Ok(tx)
        } else {
            let mut retry = 0_u64;
            while retry < bump_retries {
                wrapped_tx = WrappedTransaction::L2(tx.clone());
                match self
                    .estimate_gas_for_wrapped_tx(&mut wrapped_tx, from)
                    .await
                {
                    Ok(gas_limit) => {
                        // Estimation succeeded.
                        tx.gas_limit = gas_limit;
                        return Ok(tx);
                    }
                    Err(e) => {
                        let error = format!("{e}");
                        if error.contains("already known") {
                            warn!("Bumping gas while building: already known");
                            retry += 1;
                            self.bump_privileged_l2(&mut tx, 110);
                            continue;
                        }
                        return Err(e);
                    }
                }
            }
            Err(EthClientError::EstimateGasPriceError(
                EstimateGasPriceError::Custom(
                    "Exceeded maximum retries while estimating gas.".to_string(),
                ),
            ))
        }
    }

    async fn get_nonce_from_overrides_or_rpc(
        &self,
        overrides: &Overrides,
        address: Address,
    ) -> Result<u64, EthClientError> {
        if let Some(nonce) = overrides.nonce {
            return Ok(nonce);
        }
        self.get_nonce(address).await
    }

    pub async fn get_last_committed_block(
        eth_client: &EthClient,
        on_chain_proposer_address: Address,
    ) -> Result<u64, EthClientError> {
        Self::_call_block_variable(
            eth_client,
            b"lastCommittedBlock()",
            on_chain_proposer_address,
        )
        .await
    }

    pub async fn get_last_verified_block(
        eth_client: &EthClient,
        on_chain_proposer_address: Address,
    ) -> Result<u64, EthClientError> {
        Self::_call_block_variable(
            eth_client,
            b"lastVerifiedBlock()",
            on_chain_proposer_address,
        )
        .await
    }

    async fn _call_block_variable(
        eth_client: &EthClient,
        selector: &[u8],
        on_chain_proposer_address: Address,
    ) -> Result<u64, EthClientError> {
        let selector = keccak(selector)
            .as_bytes()
            .get(..4)
            .ok_or(EthClientError::Custom("Failed to get selector.".to_owned()))?
            .to_vec();

        let mut calldata = Vec::new();
        calldata.extend_from_slice(&selector);

        let leading_zeros = 32 - ((calldata.len() - 4) % 32);
        calldata.extend(vec![0; leading_zeros]);

        let hex_string = eth_client
            .call(
                on_chain_proposer_address,
                calldata.into(),
                Overrides::default(),
            )
            .await?;

        let hex_string = hex_string.strip_prefix("0x").ok_or(EthClientError::Custom(
            "Couldn't strip prefix from last_committed_block.".to_owned(),
        ))?;

        if hex_string.is_empty() {
            return Err(EthClientError::Custom(
                "Failed to fetch last_committed_block. Manual intervention required.".to_owned(),
            ));
        }

        let value = U256::from_str_radix(hex_string, 16)
            .map_err(|_| {
                EthClientError::Custom(
                    "Failed to parse after call, U256::from_str_radix failed.".to_owned(),
                )
            })?
            .as_u64();

        Ok(value)
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GetTransactionByHashTransaction {
    #[serde(default, with = "ethrex_core::serde_utils::u64::hex_str")]
    pub chain_id: u64,
    #[serde(default, with = "ethrex_core::serde_utils::u64::hex_str")]
    pub nonce: u64,
    #[serde(default, with = "ethrex_core::serde_utils::u64::hex_str")]
    pub max_priority_fee_per_gas: u64,
    #[serde(default, with = "ethrex_core::serde_utils::u64::hex_str")]
    pub max_fee_per_gas: u64,
    #[serde(default, with = "ethrex_core::serde_utils::u64::hex_str")]
    pub gas_limit: u64,
    #[serde(default)]
    pub to: Address,
    #[serde(default)]
    pub value: U256,
    #[serde(default)]
    pub data: Vec<u8>,
    #[serde(default)]
    pub access_list: Vec<(Address, Vec<H256>)>,
    #[serde(default)]
    pub r#type: TxType,
    #[serde(default)]
    pub signature_y_parity: bool,
    #[serde(default, with = "ethrex_core::serde_utils::u64::hex_str")]
    pub signature_r: u64,
    #[serde(default, with = "ethrex_core::serde_utils::u64::hex_str")]
    pub signature_s: u64,
    #[serde(default)]
    pub block_number: U256,
    #[serde(default)]
    pub block_hash: H256,
    #[serde(default)]
    pub from: Address,
    #[serde(default)]
    pub hash: H256,
    #[serde(default, with = "ethrex_core::serde_utils::u64::hex_str")]
    pub transaction_index: u64,
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
            transaction_index: {}
            "#,
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
            self.transaction_index
        )
    }
}
