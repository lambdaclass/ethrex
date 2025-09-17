use crate::rpc::{RpcApiContext, RpcHandler};
use crate::utils::RpcErr;
use serde_json::Value;

// TODO: This does not need a struct,
// but I'm leaving it like this for consistency
// with the other RPC endpoints.
// The handle function could simply be
// a function called 'estimate'.
#[derive(Debug, Clone)]
pub struct GasPrice;

impl RpcHandler for GasPrice {
    fn parse(_: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(GasPrice {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let latest_block_number = context.storage.get_latest_block_number().await?;
        let latest_header = context
            .storage
            .get_block_header(latest_block_number)?
            .ok_or(RpcErr::Internal(format!(
                "Missing latest block with number {latest_block_number}"
            )))?;
        let Some(base_fee) = latest_header.base_fee_per_gas else {
            return Err(RpcErr::Internal(
                "Error calculating gas price: missing base_fee on block".to_string(),
            ));
        };
        let estimated_gas_tip = context
            .gas_tip_estimator
            .lock()
            .await
            .estimate_gas_tip(&context.storage)
            .await?;
        // To complete the gas price, we need to add the base fee to the estimated gas tip.
        let gas_price = base_fee + estimated_gas_tip;

        let gas_as_hex = format!("0x{gas_price:x}");
        Ok(serde_json::Value::String(gas_as_hex))
    }
}

#[cfg(test)]
mod tests {
    use super::GasPrice;
    use crate::eth::test_utils::{
        BASE_PRICE_IN_WEI, add_eip1559_tx_blocks, add_legacy_tx_blocks, add_mixed_tx_blocks,
        setup_store,
    };

    use crate::utils::test_utils::default_context_with_storage;
    use crate::{
        rpc::{RpcHandler, map_http_requests},
        utils::{RpcRequest, parse_json_hex},
    };
    use ethrex_common::types::MIN_GAS_TIP;
    use serde_json::json;

    #[tokio::test]
    async fn test_for_legacy_txs() {
        let storage = setup_store().await;
        let context = default_context_with_storage(storage).await;

        add_legacy_tx_blocks(&context.storage, 100, 10).await;

        let gas_price = GasPrice {};
        let response = gas_price.handle(context).await.unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, 2 * BASE_PRICE_IN_WEI);
    }

    // FIXME: There is a problem when decoding EIP1559 transactions
    // Internal("Error decoding field 'transactions' of type alloc::vec::Vec<ethrex_common::types::transaction::Transaction>: UnexpectedString")
    // #[tokio::test]
    // async fn test_for_eip_1559_txs() {
    //     let storage = setup_store().await;
    //     let context = default_context_with_storage(storage).await;

    //     add_eip1559_tx_blocks(&context.storage, 100, 10).await;

    //     let gas_price = GasPrice {};
    //     let response = gas_price.handle(context).await.unwrap();
    //     let parsed_result = parse_json_hex(&response).unwrap();
    //     assert_eq!(parsed_result, 2 * BASE_PRICE_IN_WEI);
    // }

    // // FIXME: There is a problem when decoding EIP1559 transactions
    // Internal("Error decoding field 'transactions' of type alloc::vec::Vec<ethrex_common::types::transaction::Transaction>: UnexpectedString")
    // #[tokio::test]
    // async fn test_with_mixed_transactions() {
    //     let storage = setup_store().await;
    //     let context = default_context_with_storage(storage).await;

    //     add_mixed_tx_blocks(&context.storage, 100, 10).await;

    //     let gas_price = GasPrice {};
    //     let response = gas_price.handle(context).await.unwrap();
    //     let parsed_result = parse_json_hex(&response).unwrap();
    //     assert_eq!(parsed_result, 2 * BASE_PRICE_IN_WEI);
    // }

    #[tokio::test]
    async fn test_with_not_enough_blocks_or_transactions() {
        let storage = setup_store().await;
        let context = default_context_with_storage(storage).await;

        add_mixed_tx_blocks(&context.storage, 100, 0).await;

        let gas_price = GasPrice {};
        let response = gas_price.handle(context).await.unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, BASE_PRICE_IN_WEI + MIN_GAS_TIP);
    }
    #[tokio::test]
    async fn test_with_no_blocks_but_genesis() {
        let storage = setup_store().await;
        let context = default_context_with_storage(storage).await;
        let gas_price = GasPrice {};
        // genesis base fee is = BASE_PRICE_IN_WEI
        let expected_gas_price = BASE_PRICE_IN_WEI + MIN_GAS_TIP;
        let response = gas_price.handle(context).await.unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, expected_gas_price);
    }
    #[tokio::test]
    async fn request_smoke_test() {
        let raw_json = json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_gasPrice",
            "id":1
        });
        let expected_response = json!("0x3b9aca00");
        let request: RpcRequest = serde_json::from_value(raw_json).expect("Test json is not valid");
        let storage = setup_store().await;
        let context = default_context_with_storage(storage).await;

        add_legacy_tx_blocks(&context.storage, 100, 1).await;

        let response = map_http_requests(&request, context).await.unwrap();
        assert_eq!(response, expected_response)
    }
}
