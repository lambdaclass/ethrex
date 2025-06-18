use keccak_hash::H256;
use serde_json::Value;
use tracing::info;

use crate::{
    clients::eth::L1MessageProof,
    rpc::{RpcApiContext, RpcHandler},
    utils::{RpcErr, merkle_proof},
};

use ethrex_l2_common::l1_messages::get_block_l1_message_hashes;

pub struct GetL1MessageProof {
    pub transaction_hash: H256,
}

impl RpcHandler for GetL1MessageProof {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetL1MessageProof, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected one param and {} were provided",
                params.len()
            )));
        };
        Ok(GetL1MessageProof {
            transaction_hash: serde_json::from_value(params[0].clone())?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        info!(
            "Requested l1message proof for transaction {:#x}",
            self.transaction_hash,
        );

        // Gets the transaction from the storage
        let (tx_block_number, _, tx_index) = match storage
            .get_transaction_location(self.transaction_hash)
            .await?
        {
            Some(location) => location,
            _ => return Ok(Value::Null),
        };
        let tx_receipt = match storage.get_receipt(tx_block_number, tx_index).await? {
            Some(receipt) => receipt,
            _ => return Ok(Value::Null),
        };
        let tx = match storage
            .get_transaction_by_hash(self.transaction_hash)
            .await?
        {
            Some(tx) => tx,
            _ => return Ok(Value::Null),
        };

        // Gets the message hashes from the transaction
        let tx_message_hashes = get_block_l1_message_hashes(&[tx], &[tx_receipt])
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        // Gets the batch number for the block
        let batch_number = match context
            .rollup_store
            .get_batch_number_by_block(tx_block_number)
            .await?
        {
            Some(location) => location,
            _ => return Ok(Value::Null),
        };

        // Gets the message hashes for the batch
        let batch_message_hashes = match context
            .rollup_store
            .get_message_hashes_by_batch(batch_number)
            .await?
        {
            Some(messages) => messages,
            _ => return Ok(Value::Null),
        };

        let mut proofs = vec![];
        for (index, message_hash) in batch_message_hashes.iter().enumerate() {
            if !tx_message_hashes.contains(message_hash) {
                continue;
            }

            // Calculates the merkle proof of the batch
            let Some(path) = merkle_proof(batch_message_hashes.clone(), index) else {
                return Ok(Value::Null);
            };

            let proof = L1MessageProof {
                batch_number,
                index,
                message_hash: *message_hash,
                merkle_proof: path,
            };
            proofs.push(proof);
        }
        serde_json::to_value(proofs).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
