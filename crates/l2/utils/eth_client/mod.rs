use crate::utils::config::eth::EthConfig;
use errors::{
    EstimateGasPriceError, EthClientError, GetBalanceError, GetBlockByHashError,
    GetBlockNumberError, GetGasPriceError, GetLogsError, GetNonceError, GetTransactionByHashError,
    GetTransactionReceiptError, SendRawTransactionError,
};
use ethereum_rust_core::types::{
    BlockBody, EIP1559Transaction, GenericTransaction, PrivilegedL2Transaction, TxKind, TxType,
};
use ethereum_rust_rlp::encode::RLPEncode;
use ethereum_rust_rpc::{
    types::receipt::{RpcLog, RpcReceipt},
    utils::{RpcErrorResponse, RpcRequest, RpcRequestId, RpcSuccessResponse},
};
use ethereum_types::{Address, H256, U256};
use keccak_hash::keccak;
use libsecp256k1::{sign, Message, SecretKey};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use transaction::PayloadRLPEncode;

pub mod errors;
pub mod eth_sender;
pub mod transaction;

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum RpcResponse {
    Success(RpcSuccessResponse),
    Error(RpcErrorResponse),
}

pub struct EthClient {
    client: Client,
    url: String,
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
        tx: &mut EIP1559Transaction,
        private_key: SecretKey,
    ) -> Result<H256, EthClientError> {
        let mut payload = vec![TxType::EIP1559 as u8];
        payload.append(tx.encode_payload_to_vec().as_mut());

        let data = Message::parse(&keccak(payload).0);
        let signature = sign(&data, &private_key);

        tx.signature_r = U256::from(signature.0.r.b32());
        tx.signature_s = U256::from(signature.0.s.b32());
        tx.signature_y_parity = signature.1.serialize() != 0;

        let mut encoded_tx = Vec::new();
        tx.encode(&mut encoded_tx);

        let mut data = vec![TxType::EIP1559 as u8];
        data.append(&mut encoded_tx);

        self.send_raw_transaction(data.as_slice()).await
    }

    pub async fn send_privileged_l2_transaction(
        &self,
        mut tx: PrivilegedL2Transaction,
        private_key: SecretKey,
    ) -> Result<H256, EthClientError> {
        let mut payload = vec![TxType::Privileged as u8];
        payload.append(tx.encode_payload_to_vec().as_mut());

        let data = Message::parse(&keccak(payload).0);
        let signature = sign(&data, &private_key);

        tx.signature_r = U256::from(signature.0.r.b32());
        tx.signature_s = U256::from(signature.0.s.b32());
        tx.signature_y_parity = signature.1.serialize() != 0;

        let mut encoded_tx = Vec::new();
        tx.encode(&mut encoded_tx);

        let mut data = vec![TxType::Privileged as u8];
        data.append(&mut encoded_tx);

        self.send_raw_transaction(data.as_slice()).await
    }

    pub async fn estimate_gas(
        &self,
        transaction: GenericTransaction,
    ) -> Result<u64, EthClientError> {
        let to = match transaction.to {
            TxKind::Call(addr) => addr,
            TxKind::Create => Address::zero(),
        };
        let data = json!({
            "to": format!("{to:#x}"),
            "input": format!("{:#x}", transaction.input),
            "from": format!("{:#x}", transaction.from),
            "nonce": format!("{:#x}", transaction.nonce),
        });

        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_estimateGas".to_string(),
            params: Some(vec![data, json!("latest")]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => u64::from_str_radix(
                &serde_json::from_value::<String>(result.result)
                    .map_err(EstimateGasPriceError::SerdeJSONError)?[2..],
                16,
            )
            .map_err(EstimateGasPriceError::ParseIntError)
            .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                let error_data = if let Some(error_data) = error_response.error.data {
                    if &error_data == "0x" {
                        "unknown error".to_owned()
                    } else {
                        let abi_decoded_error_data =
                            hex::decode(error_data.strip_prefix("0x").unwrap()).unwrap();
                        let string_length = U256::from_big_endian(&abi_decoded_error_data[36..68]);
                        let string_data =
                            &abi_decoded_error_data[68..68 + string_length.as_usize()];
                        String::from_utf8(string_data.to_vec()).unwrap()
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
                &serde_json::from_value::<String>(result.result)
                    .map_err(GetNonceError::SerdeJSONError)?[2..],
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

    pub async fn get_block_by_hash(&self, block_hash: H256) -> Result<BlockBody, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_getBlockByHash".to_string(),
            params: Some(vec![json!(format!("{block_hash:#x}")), json!(true)]),
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
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GetTransactionByHashTransaction {
    #[serde(default)]
    pub chain_id: u64,
    #[serde(default)]
    pub nonce: u64,
    #[serde(default)]
    pub max_priority_fee_per_gas: u64,
    #[serde(default)]
    pub max_fee_per_gas: u64,
    #[serde(default)]
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
    #[serde(default, with = "ethereum_rust_core::serde_utils::u64::hex_str")]
    pub signature_r: u64,
    #[serde(default, with = "ethereum_rust_core::serde_utils::u64::hex_str")]
    pub signature_s: u64,
    #[serde(default)]
    pub block_number: U256,
    #[serde(default)]
    pub block_hash: H256,
    #[serde(default)]
    pub from: Address,
    #[serde(default)]
    pub hash: H256,
    #[serde(default, with = "ethereum_rust_core::serde_utils::u64::hex_str")]
    pub transaction_index: u64,
}
