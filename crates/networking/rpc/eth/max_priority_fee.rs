use crate::eth::fee_calculator::estimate_gas_tip;

use crate::utils::RpcErr;
use crate::{RpcApiContext, RpcHandler};
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

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let estimated_gas_tip = estimate_gas_tip(&context.storage)?;

        let gas_tip = match estimated_gas_tip {
            Some(gas_tip) => gas_tip,
            None => return Ok(serde_json::Value::Null),
        };

        let gas_as_hex = format!("0x{:x}", gas_tip);
        Ok(serde_json::Value::String(gas_as_hex))
    }
}

#[cfg(test)]
mod tests {
    use super::MaxPriorityFee;
    use crate::eth::test_utils::{
        add_eip1559_tx_blocks, add_legacy_tx_blocks, add_mixed_tx_blocks, setup_store,
        BASE_PRICE_IN_WEI,
    };

    use crate::utils::test_utils::example_local_node_record;
    use crate::{
        map_http_requests,
        utils::{parse_json_hex, test_utils::example_p2p_node, RpcRequest},
        RpcApiContext, RpcHandler,
    };
    use ethrex_net::sync::SyncManager;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn default_context() -> RpcApiContext {
        let mut context = crate::utils::test_utils::default_context();
        context.storage = setup_store();
        context
    }

    #[test]
    fn test_for_legacy_txs() {
        let context = default_context();

        add_legacy_tx_blocks(&context.storage, 100, 10);

        let gas_price = MaxPriorityFee {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, BASE_PRICE_IN_WEI);
    }

    #[test]
    fn test_for_eip_1559_txs() {
        let context = default_context();

        add_eip1559_tx_blocks(&context.storage, 100, 10);

        let gas_price = MaxPriorityFee {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, BASE_PRICE_IN_WEI);
    }
    #[test]
    fn test_with_mixed_transactions() {
        let context = default_context();

        add_mixed_tx_blocks(&context.storage, 100, 10);

        let gas_price = MaxPriorityFee {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, BASE_PRICE_IN_WEI);
    }
    #[test]
    fn test_with_not_enough_blocks_or_transactions() {
        let context = default_context();

        add_mixed_tx_blocks(&context.storage, 100, 0);

        let gas_price = MaxPriorityFee {};
        let response = gas_price.handle(context).unwrap();
        assert_eq!(response, Value::Null);
    }
    #[test]
    fn test_with_no_blocks_but_genesis() {
        let context = default_context();
        let gas_price = MaxPriorityFee {};

        let response = gas_price.handle(context).unwrap();
        assert_eq!(response, Value::Null);
    }
    #[test]
    fn request_smoke_test() {
        let raw_json = json!(
        {
            "jsonrpc":"2.0",
            "method":"eth_maxPriorityFeePerGas",
            "id":1
        });
        let expected_response = json!("0x3b9aca00");
        let request: RpcRequest = serde_json::from_value(raw_json).expect("Test json is not valid");
        let mut context = default_context();
        context.local_p2p_node = example_p2p_node();

        add_eip1559_tx_blocks(&context.storage, 100, 3);

        let response = map_http_requests(&request, context).unwrap();
        assert_eq!(response, expected_response)
    }
}
