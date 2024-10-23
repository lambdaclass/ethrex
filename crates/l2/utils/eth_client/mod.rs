use crate::utils::config::eth::EthConfig;
use errors::{
    CallError, EstimateGasPriceError, EthClientError, GetBalanceError, GetBlockNumberError,
    GetGasPriceError, GetLogsError, GetNonceError, GetTransactionReceiptError,
    SendRawTransactionError,
};
use ethereum_rust_core::types::{EIP1559Transaction, GenericTransaction, TxKind};
use ethereum_rust_rlp::encode::RLPEncode;
use ethereum_rust_rpc::{
    types::receipt::{RpcLog, RpcReceipt},
    utils::{RpcErrorResponse, RpcRequest, RpcRequestId, RpcSuccessResponse},
};
use ethereum_types::{Address, H256, U256};
use keccak_hash::keccak;
use libsecp256k1::{sign, Message, SecretKey};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use transaction::PayloadRLPEncode;

pub mod errors;
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

const EIP1559_TX_TYPE: u8 = 2;

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
        mut tx: EIP1559Transaction,
        private_key: SecretKey,
    ) -> Result<H256, EthClientError> {
        let mut payload = vec![EIP1559_TX_TYPE];
        payload.append(tx.encode_payload_to_vec().as_mut());

        let data = Message::parse(&keccak(payload).0);
        let signature = sign(&data, &private_key);

        tx.signature_r = U256::from(signature.0.r.b32());
        tx.signature_s = U256::from(signature.0.s.b32());
        tx.signature_y_parity = signature.1.serialize() != 0;

        let mut encoded_tx = Vec::new();
        tx.encode(&mut encoded_tx);

        let mut data = vec![EIP1559_TX_TYPE];
        data.append(&mut encoded_tx);

        self.send_raw_transaction(data.as_slice()).await
    }

    pub async fn call(&self, transaction: GenericTransaction) -> Result<String, EthClientError> {
        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_call".to_string(),
            params: Some(vec![
                json!({
                    "to": match transaction.to {
                        TxKind::Call(addr) => format!("{addr:#x}"),
                        TxKind::Create => format!("{:#x}", Address::zero()),
                    },
                    "input": format!("{:#x}", transaction.input),
                    "value": format!("{}", transaction.value),
                    "from": format!("{:#x}", transaction.from),
                }),
                json!("latest"),
            ]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(CallError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(CallError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn estimate_gas(
        &self,
        transaction: EIP1559Transaction,
    ) -> Result<u64, EthClientError> {
        let to = match transaction.to {
            TxKind::Call(addr) => addr,
            TxKind::Create => Address::zero(),
        };
        let data = json!({
            "to": format!("{to:#x}"),
            "input": format!("{:#x}", transaction.data),
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
                Err(EstimateGasPriceError::RPCError(error_response.error.message).into())
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
}
