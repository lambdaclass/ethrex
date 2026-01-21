use std::str::FromStr;

use crate::l2::batch::RpcBatch;
use bytes::Bytes;
use ethrex_common::Address;
use ethrex_common::H256;
use ethrex_common::U256;
use ethrex_common::types::{AuthorizationList, AuthorizationTupleEntry};
use ethrex_l2_common::messages::L1MessageProof;
use ethrex_rpc::types::block_identifier::BlockIdentifier;
use ethrex_rpc::{
    EthClient,
    clients::{
        EthClientError,
        eth::{RpcResponse, errors::RpcRequestError},
    },
    utils::RpcRequest,
};
use hex;
use serde_json::json;

pub async fn get_l1_message_proof(
    client: &EthClient,
    transaction_hash: H256,
) -> Result<Option<Vec<L1MessageProof>>, EthClientError> {
    let params = Some(vec![json!(format!("{:#x}", transaction_hash))]);
    let request = RpcRequest::new("ethrex_getL1MessageProof", params);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => serde_json::from_value(result.result)
            .map_err(RpcRequestError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
            method: "ethrex_getL1MessageProof",
            message: error_response.error.message,
        }
        .into()),
    }
}

pub async fn get_batch_by_block(
    client: &EthClient,
    block: BlockIdentifier,
) -> Result<Option<RpcBatch>, EthClientError> {
    let params = Some(vec![block.into()]);
    let request = RpcRequest::new("ethrex_getBatchByBlock", params);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => serde_json::from_value(result.result)
            .map_err(RpcRequestError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
            method: "ethrex_getBatchByBlock",
            message: error_response.error.message,
        }
        .into()),
    }
}

pub async fn get_batch_by_number(
    client: &EthClient,
    batch_number: u64,
) -> Result<RpcBatch, EthClientError> {
    let params = Some(vec![json!(format!("{batch_number:#x}")), json!(true)]);
    let request = RpcRequest::new("ethrex_getBatchByNumber", params);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => serde_json::from_value(result.result)
            .map_err(RpcRequestError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
            method: "ethrex_getBatchByNumber",
            message: error_response.error.message,
        }
        .into()),
    }
}

pub async fn get_batch_number(client: &EthClient) -> Result<u64, EthClientError> {
    let request = RpcRequest::new("ethrex_batchNumber", None);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => {
            let batch_number_hex: String = serde_json::from_value(result.result)
                .map_err(RpcRequestError::SerdeJSONError)
                .map_err(EthClientError::from)?;
            let hex_str = batch_number_hex
                .strip_prefix("0x")
                .unwrap_or(&batch_number_hex);
            u64::from_str_radix(hex_str, 16)
                .map_err(RpcRequestError::ParseIntError)
                .map_err(EthClientError::from)
        }
        RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
            method: "ethrex_batchNumber",
            message: error_response.error.message,
        }
        .into()),
    }
}

pub async fn get_base_fee_vault_address(
    client: &EthClient,
    block: BlockIdentifier,
) -> Result<Option<Address>, EthClientError> {
    let params = Some(vec![block.into()]);
    let request = RpcRequest::new("ethrex_getBaseFeeVaultAddress", params);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => serde_json::from_value(result.result)
            .map_err(RpcRequestError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
            method: "ethrex_getBaseFeeVaultAddress",
            message: error_response.error.message,
        }
        .into()),
    }
}

pub async fn get_operator_fee_vault_address(
    client: &EthClient,
    block: BlockIdentifier,
) -> Result<Option<Address>, EthClientError> {
    let params = Some(vec![block.into()]);
    let request = RpcRequest::new("ethrex_getOperatorFeeVaultAddress", params);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => serde_json::from_value(result.result)
            .map_err(RpcRequestError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
            method: "ethrex_getOperatorFeeVaultAddress",
            message: error_response.error.message,
        }
        .into()),
    }
}

pub async fn get_operator_fee(
    client: &EthClient,
    block: BlockIdentifier,
) -> Result<U256, EthClientError> {
    let params = Some(vec![block.into()]);
    let request = RpcRequest::new("ethrex_getOperatorFee", params);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => serde_json::from_value(result.result)
            .map_err(RpcRequestError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
            method: "ethrex_getOperatorFee",
            message: error_response.error.message,
        }
        .into()),
    }
}

pub async fn get_l1_fee_vault_address(
    client: &EthClient,
    block: BlockIdentifier,
) -> Result<Option<Address>, EthClientError> {
    let params = Some(vec![block.into()]);
    let request = RpcRequest::new("ethrex_getL1FeeVaultAddress", params);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => serde_json::from_value(result.result)
            .map_err(RpcRequestError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
            method: "ethrex_getL1FeeVaultAddress",
            message: error_response.error.message,
        }
        .into()),
    }
}

pub async fn get_l1_blob_base_fee_per_gas(
    client: &EthClient,
    block_number: u64,
) -> Result<u64, EthClientError> {
    let params = Some(vec![json!(format!("{block_number:#x}"))]);
    let request = RpcRequest::new("ethrex_getL1BlobBaseFee", params);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => serde_json::from_value(result.result)
            .map_err(RpcRequestError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
            method: "ethrex_getL1BlobBaseFee",
            message: error_response.error.message,
        }
        .into()),
    }
}

pub async fn send_ethrex_transaction(
    client: &EthClient,
    to: Address,
    data: Bytes,
    authorization_list: Option<AuthorizationList>,
) -> Result<H256, EthClientError> {
    let authorization_list = authorization_list.map(|list| {
        list.iter()
            .map(AuthorizationTupleEntry::from)
            .collect::<Vec<_>>()
    });

    let payload = json!({
        "to": format!("{to:#x}"),
        "data": format!("0x{}", hex::encode(data)),
        "authorizationList": authorization_list,
    });
    let request = RpcRequest::new("ethrex_sendTransaction", Some(vec![payload]));

    match client.send_request(request).await? {
        RpcResponse::Success(result) => {
            let tx_hash_str: String = serde_json::from_value(result.result)
                .map_err(RpcRequestError::SerdeJSONError)
                .map_err(EthClientError::from)?;
            H256::from_str(&tx_hash_str)
                .map_err(|e| RpcRequestError::Custom(e.to_string()))
                .map_err(EthClientError::from)
        }
        RpcResponse::Error(error_response) => Err(RpcRequestError::RPCError {
            method: "ethrex_sendTransaction",
            message: error_response.error.message,
        }
        .into()),
    }
}
