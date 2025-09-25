use crate::l2::batch::RpcBatch;
use ethrex_common::Address;
use ethrex_common::H256;
use ethrex_l2_common::l1_messages::L1MessageProof;
use ethrex_rpc::{
    EthClient,
    clients::{
        EthClientError,
        eth::{
            RpcResponse,
            errors::{GetBatchByNumberError, GetFeeVaultAddressError, GetMessageProofError},
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

pub async fn get_fee_vault_address(client: &EthClient) -> Result<Option<Address>, EthClientError> {
    let request = RpcRequest::new("ethrex_getFeeVaultAddress", None);

    match client.send_request(request).await? {
        RpcResponse::Success(result) => serde_json::from_value(result.result)
            .map_err(GetFeeVaultAddressError::SerdeJSONError)
            .map_err(EthClientError::from),
        RpcResponse::Error(error_response) => {
            Err(GetFeeVaultAddressError::RPCError(error_response.error.message).into())
        }
    }
}
