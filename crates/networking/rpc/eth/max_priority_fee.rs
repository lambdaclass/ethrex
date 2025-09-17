use crate::rpc::{RpcApiContext, RpcHandler};
use crate::utils::RpcErr;
use serde_json::Value;

// TODO: This does not need a struct,
// but I'm leaving it like this for consistency
// with the other RPC endpoints.
// The handle function could simply be
// a function called 'estimate'.
#[derive(Debug, Clone)]
pub struct MaxPriorityFee;

impl RpcHandler for MaxPriorityFee {
    fn parse(_: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(MaxPriorityFee {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let gas_tip = context
            .gas_tip_estimator
            .lock()
            .await
            .estimate_gas_tip(&context.storage)
            .await?;

        let gas_as_hex = format!("0x{gas_tip:x}");
        Ok(serde_json::Value::String(gas_as_hex))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eth::test_utils::{
        BASE_PRICE_IN_WEI, add_eip1559_tx_blocks, add_legacy_tx_blocks, add_mixed_tx_blocks,
        setup_store,
    };

    use crate::utils::test_utils::default_context_with_storage;
    use crate::{
        rpc::{RpcApiContext, RpcHandler, map_http_requests},
        utils::{RpcRequest, parse_json_hex, test_utils::example_p2p_node},
    };
    use ethrex_common::types::MIN_GAS_TIP;
    use serde_json::json;

    async fn default_context() -> RpcApiContext {
        let storage = setup_store().await;
        default_context_with_storage(storage).await
    }

    #[tokio::test]
    async fn test_for_legacy_txs() {
        let context = default_context().await;

        add_legacy_tx_blocks(&context.storage, 100, 10).await;

        let gas_price = MaxPriorityFee {};
        let response = gas_price.handle(context).await.unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, BASE_PRICE_IN_WEI);
    }

    // FIXME: There is a problem when decoding EIP1559 transactions
    // Internal("Error decoding field 'transactions' of type alloc::vec::Vec<ethrex_common::types::transaction::Transaction>: UnexpectedString")
    // #[tokio::test]
    // async fn test_for_eip_1559_txs() {
    //     let context = default_context().await;

    //     add_eip1559_tx_blocks(&context.storage, 100, 10).await;

    //     let gas_price = MaxPriorityFee {};
    //     let response = gas_price.handle(context).await.unwrap();
    //     let parsed_result = parse_json_hex(&response).unwrap();
    //     assert_eq!(parsed_result, BASE_PRICE_IN_WEI);
    // }

    // FIXME: There is a problem when decoding EIP1559 transactions
    // Internal("Error decoding field 'transactions' of type alloc::vec::Vec<ethrex_common::types::transaction::Transaction>: UnexpectedString")
    // #[tokio::test]
    // async fn test_with_mixed_transactions() {
    //     let context = default_context().await;

    //     add_mixed_tx_blocks(&context.storage, 100, 10).await;

    //     let gas_price = MaxPriorityFee {};
    //     let response = gas_price.handle(context).await.unwrap();
    //     let parsed_result = parse_json_hex(&response).unwrap();
    //     assert_eq!(parsed_result, BASE_PRICE_IN_WEI);
    // }

    #[tokio::test]
    async fn test_with_not_enough_blocks_or_transactions() {
        let context = default_context().await;

        add_mixed_tx_blocks(&context.storage, 100, 0).await;

        let gas_price = MaxPriorityFee {};
        let response = gas_price.handle(context).await.unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, MIN_GAS_TIP);
    }
    #[tokio::test]
    async fn test_with_no_blocks_but_genesis() {
        let context = default_context().await;
        let gas_price = MaxPriorityFee {};

        let response = gas_price.handle(context).await.unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, MIN_GAS_TIP);
    }

    // FIXME: There is a problem when decoding EIP1559 transactions
    // Internal("Error decoding field 'transactions' of type alloc::vec::Vec<ethrex_common::types::transaction::Transaction>: UnexpectedString")
    // #[tokio::test]
    // async fn request_smoke_test() {
    //     let raw_json = json!(
    //     {
    //         "jsonrpc":"2.0",
    //         "method":"eth_maxPriorityFeePerGas",
    //         "id":1
    //     });
    //     let expected_response = json!("0x3b9aca00");
    //     let request: RpcRequest = serde_json::from_value(raw_json).expect("Test json is not valid");
    //     let mut context = default_context().await;
    //     context.node_data.local_p2p_node = example_p2p_node();

    //     add_eip1559_tx_blocks(&context.storage, 100, 3).await;

    //     let response = map_http_requests(&request, context).await.unwrap();
    //     assert_eq!(response, expected_response)
    // }
}
