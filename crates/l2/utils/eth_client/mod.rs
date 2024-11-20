use crate::utils::config::eth::EthConfig;
use bytes::Bytes;
use errors::{
    EstimateGasPriceError, EthClientError, GetBalanceError, GetBlockByHashError,
    GetBlockByNumberError, GetBlockNumberError, GetGasPriceError, GetLogsError, GetNonceError,
    GetTransactionByHashError, GetTransactionReceiptError, SendRawTransactionError,
};
use eth_sender::Overrides;
use ethereum_types::{Address, H256, U256};
use ethrex_core::types::{
    BlobsBundle, EIP1559Transaction, EIP4844Transaction, GenericTransaction,
    PrivilegedL2Transaction, PrivilegedTxType, Signable, TxKind, TxType,
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
use serde_json::json;
use sha2::{Digest, Sha256};

pub mod errors;
pub mod eth_sender;

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum RpcResponse {
    Success(RpcSuccessResponse),
    Error(RpcErrorResponse),
}

pub struct EthClient {
    client: Client,
    pub url: String,
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
        tx: EIP1559Transaction,
        private_key: &SecretKey,
    ) -> Result<H256, EthClientError> {
        let signed_tx = tx.sign(private_key);

        let mut encoded_tx = signed_tx.encode_to_vec();
        encoded_tx.insert(0, TxType::EIP1559 as u8);

        self.send_raw_transaction(encoded_tx.as_slice()).await
    }

    pub async fn send_eip4844_transaction(
        &self,
        mut wrapped_tx: WrappedEIP4844Transaction,
        private_key: &SecretKey,
    ) -> Result<H256, EthClientError> {
        wrapped_tx.tx.sign_inplace(private_key);

        let mut encoded_tx = wrapped_tx.encode_to_vec();
        encoded_tx.insert(0, TxType::EIP4844 as u8);

        self.send_raw_transaction(encoded_tx.as_slice()).await
    }

    pub async fn send_privileged_l2_transaction(
        &self,
        tx: PrivilegedL2Transaction,
        private_key: &SecretKey,
    ) -> Result<H256, EthClientError> {
        let signed_tx = tx.sign(private_key);

        let mut encoded_tx = signed_tx.encode_to_vec();
        encoded_tx.insert(0, TxType::Privileged as u8);

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
            data["nonce"] = json!(format!("{:#x}", nonce));
        }

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

    /// Build an EIP1559 transaction with the given parameters.
    /// Either `overrides.nonce` or `overrides.from` must be provided.
    /// If `overrides.gas_price`, `overrides.chain_id` or `overrides.gas_price`
    /// are not provided, the client will fetch them from the network.
    /// If `overrides.gas_limit` is not provided, the client will estimate the tx cost.
    pub async fn build_eip1559_transaction(
        &self,
        to: Address,
        calldata: Bytes,
        overrides: Overrides,
    ) -> Result<EIP1559Transaction, EthClientError> {
        let mut tx = EIP1559Transaction {
            to: TxKind::Call(to),
            chain_id: if let Some(chain_id) = overrides.chain_id {
                chain_id
            } else {
                self.get_chain_id().await?.as_u64()
            },
            nonce: self.get_nonce_from_overrides(&overrides).await?,
            max_priority_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                gas_price
            } else {
                self.get_gas_price().await?.as_u64()
            },
            max_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                gas_price
            } else {
                self.get_gas_price().await?.as_u64()
            },
            value: overrides.value.unwrap_or_default(),
            data: calldata,
            access_list: overrides.access_list,
            ..Default::default()
        };

        tx.gas_limit = if let Some(gas_limit) = overrides.gas_limit {
            gas_limit
        } else {
            let mut generic_tx = GenericTransaction::from(tx.clone());
            if let Some(from) = overrides.from {
                generic_tx.from = from;
            }
            self.estimate_gas(generic_tx).await?
        };

        Ok(tx)
    }

    /// Build an EIP4844 transaction with the given parameters.
    /// Either `overrides.nonce` or `overrides.from` must be provided.
    /// If `overrides.gas_price`, `overrides.chain_id` or `overrides.gas_price`
    /// are not provided, the client will fetch them from the network.
    /// If `overrides.gas_limit` is not provided, the client will estimate the tx cost.
    pub async fn build_eip4844_transaction(
        &self,
        to: Address,
        calldata: Bytes,
        overrides: Overrides,
        blobs_bundle: BlobsBundle,
    ) -> Result<WrappedEIP4844Transaction, EthClientError> {
        let blob_versioned_hashes = blobs_bundle
            .commitments
            .iter()
            .map(|commitment| {
                let mut hasher = Sha256::new();
                hasher.update(commitment);
                let mut blob_versioned_hash = hasher.finalize();
                blob_versioned_hash[0] = 0x01; // EIP-4844 versioning
                H256::from_slice(blob_versioned_hash.as_slice())
            })
            .collect::<Vec<H256>>();

        let mut tx = EIP4844Transaction {
            to,
            chain_id: if let Some(chain_id) = overrides.chain_id {
                chain_id
            } else {
                self.get_chain_id().await?.as_u64()
            },
            nonce: self.get_nonce_from_overrides(&overrides).await?,
            max_priority_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                gas_price
            } else {
                self.get_gas_price().await?.as_u64()
            },
            max_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                gas_price
            } else {
                self.get_gas_price().await?.as_u64()
            },
            value: overrides.value.unwrap_or_default(),
            data: calldata,
            access_list: overrides.access_list,
            max_fee_per_blob_gas: overrides.gas_price_per_blob.unwrap_or_default(),
            blob_versioned_hashes,
            ..Default::default()
        };

        tx.gas = if let Some(gas_limit) = overrides.gas_limit {
            gas_limit
        } else {
            let mut generic_tx = GenericTransaction::from(tx.clone());
            if let Some(from) = overrides.from {
                generic_tx.from = from;
            }
            self.estimate_gas(generic_tx).await?
        };

        Ok(WrappedEIP4844Transaction { tx, blobs_bundle })
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
        calldata: Bytes,
        overrides: Overrides,
    ) -> Result<PrivilegedL2Transaction, EthClientError> {
        let mut tx = PrivilegedL2Transaction {
            tx_type,
            to: TxKind::Call(to),
            chain_id: if let Some(chain_id) = overrides.chain_id {
                chain_id
            } else {
                self.get_chain_id().await?.as_u64()
            },
            nonce: self.get_nonce_from_overrides(&overrides).await?,
            max_priority_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                gas_price
            } else {
                self.get_gas_price().await?.as_u64()
            },
            max_fee_per_gas: if let Some(gas_price) = overrides.gas_price {
                gas_price
            } else {
                self.get_gas_price().await?.as_u64()
            },
            value: overrides.value.unwrap_or_default(),
            data: calldata,
            access_list: overrides.access_list,
            ..Default::default()
        };

        tx.gas_limit = if let Some(gas_limit) = overrides.gas_limit {
            gas_limit
        } else {
            let mut generic_tx = GenericTransaction::from(tx.clone());
            if let Some(from) = overrides.from {
                generic_tx.from = from;
            }
            self.estimate_gas(generic_tx).await?
        };

        Ok(tx)
    }

    async fn get_nonce_from_overrides(&self, overrides: &Overrides) -> Result<u64, EthClientError> {
        if let Some(nonce) = overrides.nonce {
            return Ok(nonce);
        }

        let address = overrides.from.ok_or(EthClientError::UnrecheableNonce)?;
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
            .expect("Failed to get initialize selector")
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

        let hex_string = hex_string
            .strip_prefix("0x")
            .expect("Couldn't strip prefix from last_committed_block.");

        // TODO return error
        if hex_string.is_empty() {
            panic!("Failed to fetch last_committed_block. Manual intervention required");
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
