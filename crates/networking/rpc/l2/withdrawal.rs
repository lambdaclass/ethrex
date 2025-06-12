use keccak_hash::H256;
use serde_json::Value;
use tracing::info;

use crate::{
    l2::utils::{get_withdrawal_hash, merkle_proof},
    rpc::{RpcApiContext, RpcHandler},
    types::withdrawal::WithdrawalProof,
    utils::RpcErr,
};

pub struct GetWithdrawalProof {
    pub transaction_hash: H256,
}

impl RpcHandler for GetWithdrawalProof {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetWithdrawalProof, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected one param and {} were provided",
                params.len()
            )));
        };
        Ok(GetWithdrawalProof {
            transaction_hash: serde_json::from_value(params[0].clone())?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        info!(
            "Requested withdrawal proof for transaction {:#x}",
            self.transaction_hash,
        );

        // Gets the transaction from the storage
        let withdrawal_transaction = match storage
            .get_transaction_by_hash(self.transaction_hash)
            .await?
        {
            Some(transaction) => transaction,
            _ => return Ok(Value::Null),
        };

        // Gets the withdrawal hash from the transaction
        let tx_withdrawal_hash = get_withdrawal_hash(&withdrawal_transaction)
            .ok_or_else(|| RpcErr::BadParams("Transaction is not a withdrawal".to_string()))?;

        // Gets the block number where the transaction was included
        let (block_number, _block_hash, _index) = match storage
            .get_transaction_location(self.transaction_hash)
            .await?
        {
            Some(location) => location,
            _ => return Ok(Value::Null),
        };

        // Gets the batch number for the block
        let batch_number = match context
            .rollup_store
            .get_batch_number_by_block(block_number)
            .await?
        {
            Some(location) => location,
            _ => return Ok(Value::Null),
        };

        // Gets the withdrawal hashes for the batch
        let batch_withdrawal_hashes = match context
            .rollup_store
            .get_withdrawal_hashes_by_batch(batch_number)
            .await?
        {
            Some(withdrawals) => withdrawals,
            _ => return Ok(Value::Null),
        };

        // Gets the index of the withdrawal in the batch
        let Some(index) = batch_withdrawal_hashes
            .iter()
            .position(|hash| *hash == tx_withdrawal_hash)
        else {
            return Ok(Value::Null);
        };

        // Calculates the merkle proof of the batch
        let Some(path) = merkle_proof(batch_withdrawal_hashes, tx_withdrawal_hash) else {
            return Ok(Value::Null);
        };

        let withdrawal_proof = WithdrawalProof {
            batch_number,
            index,
            withdrawal_hash: tx_withdrawal_hash,
            merkle_proof: path,
        };
        serde_json::to_value(withdrawal_proof).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
