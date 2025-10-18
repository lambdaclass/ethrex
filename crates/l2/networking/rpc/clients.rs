use crate::l2::batch::RpcBatch;
use ethrex_common::Address;
use ethrex_common::H256;
use ethrex_common::U256;
use ethrex_l2_common::l1_messages::L1MessageProof;
use ethrex_rpc::clients::eth::errors::GetL1BlobBaseFeeRequestError;
use ethrex_rpc::clients::eth::errors::GetOperatorFeeError;
use ethrex_rpc::clients::eth::errors::GetOperatorFeeVaultAddressError;
use ethrex_rpc::types::block_identifier::BlockIdentifier;
use ethrex_rpc::{
    EthClient,
    clients::{
        EthClientError,
        eth::{
            RpcResponse,
            errors::{GetBaseFeeVaultAddressError, GetBatchByNumberError, GetMessageProofError},
        },
    },
    utils::RpcRequest,
};
use serde_json::json;

pub async fn get_message_proof(
    client: &EthClient,
    transaction_hash: H256,
) -> Result<Option<Vec<L1MessageProof>>, EthClientError> {
    let params = Some(vec![json!(format!("{:#x}", transaction_hash))]);
    let request = RpcRequest::new("ethrex_getMessageProof", params);

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

pub async fn get_base_fee_vault_address(
    client: &EthClient,
    block: BlockIdentifier,
) -> Result<Address, EthClientError> {
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
) -> Result<Address, EthClientError> {
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
