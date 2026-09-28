use super::frame::{parse_transaction, prepare, require_success};
use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};
use ethrex_common::{
    U256,
    types::{FrameTransaction, GenericTransaction},
};
use serde_json::{Value, json};

pub struct FillTransactionRequest {
    transaction: GenericTransaction,
}

impl RpcHandler for FillTransactionRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params.as_deref().unwrap_or_default();
        let [value] = params else {
            return Err(RpcErr::BadParams(
                "expected one transaction parameter".into(),
            ));
        };
        let transaction = parse_transaction(value)?;
        if transaction.frames.is_none() {
            return Err(RpcErr::BadParams(
                "eth_fillTransaction currently supports frame transactions only".into(),
            ));
        }
        Ok(Self { transaction })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let number = context.storage.get_latest_block_number().await?;
        let header = context
            .storage
            .get_block_header(number)?
            .ok_or_else(|| RpcErr::Internal("latest block is missing".into()))?;
        let mut transaction = self.transaction.clone();
        transaction.chain_id = Some(
            transaction
                .chain_id
                .unwrap_or(context.storage.get_chain_config().chain_id),
        );
        if transaction.nonce.is_none() {
            transaction.nonce = Some(
                context
                    .storage
                    .get_nonce_by_account_address(number, transaction.from)
                    .await?
                    .unwrap_or_default(),
            );
        }
        if transaction.max_priority_fee_per_gas.is_none() {
            let tip = context
                .gas_tip_estimator
                .lock()
                .await
                .estimate_gas_tip(&context.storage, context.blockchain.options.min_tip_wei)
                .await?;
            transaction.max_priority_fee_per_gas =
                Some(tip.min(transaction.max_fee_per_gas.unwrap_or(u64::MAX)));
        }
        if transaction.max_fee_per_gas.is_none() {
            let fee = header
                .base_fee_per_gas
                .unwrap_or_default()
                .saturating_mul(2)
                .saturating_add(transaction.max_priority_fee_per_gas.unwrap_or_default());
            transaction.max_fee_per_gas = Some(fee);
        }
        if transaction.max_fee_per_blob_gas.is_none() {
            let fee = if transaction.blob_versioned_hashes.is_empty() {
                U256::zero()
            } else {
                let value = super::block::GetBlobBaseFee::parse(&None)?
                    .handle(context.clone())
                    .await?;
                let fee: U256 = serde_json::from_value(value)?;
                fee.saturating_mul(U256::from(2))
            };
            transaction.max_fee_per_blob_gas = Some(fee);
        }
        let needs_gas = transaction
            .frames
            .iter()
            .flatten()
            .any(|frame| frame.execution_gas.is_none() || frame.state_gas.is_none());
        let tx = if needs_gas {
            let (tx, result) = prepare(&transaction, &header, &context).await?;
            require_success(&result)?;
            tx
        } else {
            FrameTransaction::try_from(&transaction)
                .map_err(|error| RpcErr::BadParams(error.to_string()))?
        };
        let Value::Object(mut filled) = serde_json::to_value(&tx)? else {
            return Err(RpcErr::Internal(
                "frame transaction must serialize as an object".into(),
            ));
        };
        filled.remove("sender");
        filled.insert("from".into(), serde_json::to_value(tx.sender)?);
        Ok(json!({"tx":filled}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{rpc::map_http_requests, utils::RpcRequest};

    #[tokio::test]
    async fn fills_frame_transaction_and_preserves_zero_limits() {
        let context = super::super::frame::tests::context().await;
        let request: RpcRequest = serde_json::from_value(json!({"jsonrpc":"2.0", "id":1,
        "method":"eth_fillTransaction", "params":[{
            "type":"0x6", "from":"0x00000a8d3f37af8def18832962ee008d8dca4f7b",
            "frames":[{"mode":"0x1", "flags":"0x3", "stateGas":"0x0"}],
            "signatures":[{"scheme":"0x1"}]
        }]}))
        .unwrap();
        let result = map_http_requests(&request, context).await.unwrap();
        assert_eq!(result["tx"]["chainId"], "0x9");
        assert_eq!(result["tx"]["nonce"], "0x5");
        assert_eq!(result["tx"]["frames"][0]["executionGas"], "0x64");
        assert_eq!(result["tx"]["frames"][0]["stateGas"], "0x0");
        assert_eq!(result["tx"]["signatures"][0]["signature"], "0x");
        assert_eq!(result["tx"]["maxFeePerBlobGas"], "0x0");
        assert!(result["tx"]["maxFeePerGas"].as_str().is_some());
        assert!(result.get("raw").is_none());
        assert!(result["tx"].get("gas").is_none());
    }
    #[tokio::test]
    async fn preserves_explicit_fees_nonce_and_zero_gas_without_execution() {
        let context = super::super::frame::tests::context().await;
        let handler = FillTransactionRequest::parse(&Some(vec![json!({
            "type":"0x6", "from":"0x00000a8d3f37af8def18832962ee008d8dca4f7b", "nonce":"0x2", "chainId":"0xa",
            "maxFeePerGas":"0x0", "maxPriorityFeePerGas":"0x0",
            "frames":[{"mode":"0x1", "flags":"0x3", "executionGas":"0x0", "stateGas":"0x0"}],
            "signatures":[{"scheme":"0x1", "signer":null, "signature":null}]
        })])).unwrap();
        let value = handler.handle(context).await.unwrap();
        assert_eq!(value["tx"]["nonce"], "0x2");
        assert_eq!(value["tx"]["chainId"], "0xa");
        assert_eq!(value["tx"]["maxFeePerGas"], "0x0");
        assert_eq!(value["tx"]["maxPriorityFeePerGas"], "0x0");
        assert_eq!(value["tx"]["frames"][0]["executionGas"], "0x0");
        assert_eq!(value["tx"]["frames"][0]["stateGas"], "0x0");
    }
}
