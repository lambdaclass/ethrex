use crate::eth::fee_calculator::estimate_gas_tip;

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

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let latest_block_number = context.storage.get_latest_block_number()?;

        let estimated_gas_tip = estimate_gas_tip(&context.storage)?;

        let base_fee = context
            .storage
            .get_block_header(latest_block_number)
            .ok()
            .flatten()
            .and_then(|header| header.base_fee_per_gas);

        // To complete the gas price, we need to add the base fee to the estimated gas.
        // If we don't have the estimated gas, we'll use the base fee as the gas price.
        // If we don't have the base fee, we'll return an Error.
        let gas_price = match (estimated_gas_tip, base_fee) {
            (Some(gas_tip), Some(base_fee)) => gas_tip + base_fee,
            (None, Some(base_fee)) => base_fee,
            (_, None) => {
                return Err(RpcErr::Internal(
                    "Error calculating gas price: missing base_fee on block".to_string(),
                ))
            }
        };

        let gas_as_hex = format!("0x{:x}", gas_price);
        Ok(serde_json::Value::String(gas_as_hex))
    }
}

#[cfg(test)]
mod tests {
    use super::GasPrice;
    use crate::eth::test_utils::{
        add_eip1559_tx_blocks, add_legacy_tx_blocks, add_mixed_tx_blocks, setup_store,
        BASE_PRICE_IN_WEI,
    };

    use crate::utils::test_utils::example_local_node_record;
    use crate::{
        rpc::{map_http_requests, RpcApiContext, RpcHandler},
        utils::{parse_json_hex, test_utils::example_p2p_node, RpcRequest},
    };
    #[cfg(feature = "based")]
    use crate::{EngineClient, EthClient};
    #[cfg(feature = "based")]
    use bytes::Bytes;
    use ethrex_blockchain::Blockchain;
    use ethrex_p2p::sync::SyncManager;
    #[cfg(feature = "l2")]
    use secp256k1::{rand, SecretKey};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn default_context() -> RpcApiContext {
        let storage = setup_store();
        let blockchain = Arc::new(Blockchain::default_with_store(storage.clone()));
        RpcApiContext {
            storage,
            blockchain,
            jwt_secret: Default::default(),
            local_p2p_node: example_p2p_node(),
            local_node_record: example_local_node_record(),
            active_filters: Default::default(),
            syncer: Arc::new(Mutex::new(SyncManager::dummy())),
            #[cfg(feature = "based")]
            gateway_eth_client: EthClient::new(""),
            #[cfg(feature = "based")]
            gateway_auth_client: EngineClient::new("", Bytes::default()),
            #[cfg(feature = "based")]
            gateway_pubkey: Default::default(),
            #[cfg(feature = "l2")]
            valid_delegation_addresses: Vec::new(),
            #[cfg(feature = "l2")]
            sponsor_pk: SecretKey::new(&mut rand::thread_rng()),
        }
    }

    #[test]
    fn test_for_legacy_txs() {
        let context = default_context();

        add_legacy_tx_blocks(&context.storage, 100, 10);

        let gas_price = GasPrice {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, 2 * BASE_PRICE_IN_WEI);
    }

    #[test]
    fn test_for_eip_1559_txs() {
        let context = default_context();

        add_eip1559_tx_blocks(&context.storage, 100, 10);

        let gas_price = GasPrice {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, 2 * BASE_PRICE_IN_WEI);
    }
    #[test]
    fn test_with_mixed_transactions() {
        let context = default_context();

        add_mixed_tx_blocks(&context.storage, 100, 10);

        let gas_price = GasPrice {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, 2 * BASE_PRICE_IN_WEI);
    }
    #[test]
    fn test_with_not_enough_blocks_or_transactions() {
        let context = default_context();

        add_mixed_tx_blocks(&context.storage, 100, 0);

        let gas_price = GasPrice {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, BASE_PRICE_IN_WEI);
    }
    #[test]
    fn test_with_no_blocks_but_genesis() {
        let context = default_context();
        let gas_price = GasPrice {};
        // genesis base fee is = BASE_PRICE_IN_WEI
        let expected_gas_price = BASE_PRICE_IN_WEI;
        let response = gas_price.handle(context).unwrap();
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
        let mut context = default_context();
        context.local_p2p_node = example_p2p_node();

        add_legacy_tx_blocks(&context.storage, 100, 1);

        let response = map_http_requests(&request, context).await.unwrap();
        assert_eq!(response, expected_response)
    }
}
