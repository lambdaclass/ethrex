use std::str::FromStr;

use crate::l2::batch::RpcBatch;
use bytes::Bytes;
use ethrex_common::Address;
use ethrex_common::H256;
use ethrex_common::U256;
use ethrex_common::types::{AuthorizationList, AuthorizationTupleEntry};
use ethrex_l2_common::messages::L1MessageProof;
use ethrex_l2_common::messages::L2MessageProof;
use ethrex_rpc::clients::eth::errors::GetL1BlobBaseFeeRequestError;
use ethrex_rpc::clients::eth::errors::GetL1FeeVaultAddressError;
use ethrex_rpc::clients::eth::errors::GetOperatorFeeError;
use ethrex_rpc::clients::eth::errors::GetOperatorFeeVaultAddressError;
use ethrex_rpc::clients::eth::errors::SendEthrexTransactionError;
use ethrex_rpc::types::block_identifier::BlockIdentifier;
use ethrex_rpc::{
    EthClient,
    clients::{
        EthClientError,
        eth::{
            RpcResponse,
            errors::{
                GetBaseFeeVaultAddressError, GetBatchByNumberError, GetBatchNumberError,
                GetMessageProofError,
            },
        },
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
            .map_err(GetMessageProofError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => {
            Err(GetMessageProofError::RPCError(error_response.error.message).into())
        }
    }
}

pub async fn get_l2_message_proof(
    client: &EthClient,
    transaction_hash: H256,
) -> Result<Option<Vec<L2MessageProof>>, EthClientError> {
    let params = Some(vec![json!(format!("{:#x}", transaction_hash))]);
    let request = RpcRequest::new("ethrex_getL2MessageProof", params);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => serde_json::from_value(result.result)
            .map_err(GetMessageProofError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => {
            Err(GetMessageProofError::RPCError(error_response.error.message).into())
        }
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
            .map_err(GetBatchByNumberError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => {
            Err(GetBatchByNumberError::RPCError(error_response.error.message).into())
        }
    }
}

pub async fn get_batch_number(client: &EthClient) -> Result<u64, EthClientError> {
    let request = RpcRequest::new("ethrex_batchNumber", None);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => {
            let batch_number_hex: String = serde_json::from_value(result.result)
                .map_err(GetBatchNumberError::SerdeJSONError)
                .map_err(EthClientError::from)?;
            let hex_str = batch_number_hex
                .strip_prefix("0x")
                .unwrap_or(&batch_number_hex);
            u64::from_str_radix(hex_str, 16)
                .map_err(GetBatchNumberError::ParseIntError)
                .map_err(EthClientError::from)
        }
        RpcResponse::Error(error_response) => {
            Err(GetBatchNumberError::RPCError(error_response.error.message).into())
        }
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
            .map_err(GetBaseFeeVaultAddressError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => {
            Err(GetBaseFeeVaultAddressError::RPCError(error_response.error.message).into())
        }
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
            .map_err(GetOperatorFeeVaultAddressError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => {
            Err(GetOperatorFeeVaultAddressError::RPCError(error_response.error.message).into())
        }
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
            .map_err(GetOperatorFeeError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => {
            Err(GetOperatorFeeError::RPCError(error_response.error.message).into())
        }
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
            .map_err(GetL1FeeVaultAddressError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => {
            Err(GetL1FeeVaultAddressError::RPCError(error_response.error.message).into())
        }
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
            .map_err(GetL1BlobBaseFeeRequestError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => {
            Err(GetL1BlobBaseFeeRequestError::RPCError(error_response.error.message).into())
        }
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
                .map_err(SendEthrexTransactionError::SerdeJSONError)
                .map_err(EthClientError::from)?;
            H256::from_str(&tx_hash_str)
                .map_err(|e| SendEthrexTransactionError::ParseHashError(e.to_string()))
                .map_err(EthClientError::from)
        }
        RpcResponse::Error(error_response) => {
            Err(SendEthrexTransactionError::RPCError(error_response.error.message).into())
        }
    }
}
