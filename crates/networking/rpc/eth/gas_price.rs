// Use fee_calculator mod in crates/networking/rpc/eth/ as gas_price

use std::cmp::max;

use crate::eth::fee_calculator::estimate_gas_tip;
use ethrex_blockchain::constants::MIN_GAS_LIMIT;

use crate::utils::RpcErr;
use crate::{RpcApiContext, RpcHandler};
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
        // If we don't have the base fee, we'll use the minimum gas limit.
        let gas_price = match (estimated_gas_tip, base_fee) {
            (Some(gas_tip), Some(base_fee)) => gas_tip + base_fee,
            (None, Some(base_fee)) => base_fee,
            // TODO: We might want to return null in this cases?
            (Some(gas_tip), None) => max(gas_tip, MIN_GAS_LIMIT),
            (None, None) => MIN_GAS_LIMIT,
        };

        let gas_as_hex = format!("0x{:x}", gas_price);
        Ok(serde_json::Value::String(gas_as_hex))
    }
}

#[cfg(test)]
mod tests {
    use super::GasPrice;
    use crate::{
        map_http_requests,
        utils::{parse_json_hex, test_utils::example_p2p_node, RpcRequest},
        RpcApiContext, RpcHandler,
    };
    use bytes::Bytes;
    use ethrex_core::{
        types::{
            Block, BlockBody, BlockHeader, EIP1559Transaction, Genesis, LegacyTransaction,
            Transaction, TxKind,
        },
        Address, Bloom, H256, U256,
    };
    use ethrex_net::{sync::SyncManager, types::Node};
    use ethrex_storage::{EngineType, Store};
    use hex_literal::hex;
    use serde_json::json;
    use std::{net::Ipv4Addr, str::FromStr, sync::Arc};
    use tokio::sync::Mutex;
    // Base price for each test transaction.
    const BASE_PRICE_IN_WEI: u64 = 10_u64.pow(9);
    fn test_header(block_num: u64) -> BlockHeader {
        BlockHeader {
            parent_hash: H256::from_str(
                "0x1ac1bf1eef97dc6b03daba5af3b89881b7ae4bc1600dc434f450a9ec34d44999",
            )
            .unwrap(),
            ommers_hash: H256::from_str(
                "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            )
            .unwrap(),
            coinbase: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
            state_root: H256::from_str(
                "0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9",
            )
            .unwrap(),
            transactions_root: H256::from_str(
                "0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d",
            )
            .unwrap(),
            receipts_root: H256::from_str(
                "0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa",
            )
            .unwrap(),
            logs_bloom: Bloom::from([0; 256]),
            difficulty: U256::zero(),
            number: block_num,
            gas_limit: 0x016345785d8a0000,
            gas_used: 0xa8de,
            timestamp: 0x03e8,
            extra_data: Bytes::new(),
            prev_randao: H256::zero(),
            nonce: 0x0000000000000000,
            base_fee_per_gas: None,
            withdrawals_root: Some(
                H256::from_str(
                    "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                )
                .unwrap(),
            ),
            blob_gas_used: Some(0x00),
            excess_blob_gas: Some(0x00),
            parent_beacon_block_root: Some(H256::zero()),
        }
    }
    fn legacy_tx_for_test(nonce: u64) -> Transaction {
        Transaction::LegacyTransaction(LegacyTransaction {
            nonce,
            gas_price: nonce * BASE_PRICE_IN_WEI,
            gas: 10000,
            to: TxKind::Create,
            value: 100.into(),
            data: Default::default(),
            v: U256::from(0x1b),
            r: U256::from_big_endian(&hex!(
                "7e09e26678ed4fac08a249ebe8ed680bf9051a5e14ad223e4b2b9d26e0208f37"
            )),
            s: U256::from_big_endian(&hex!(
                "5f6e3f188e3e6eab7d7d3b6568f5eac7d687b08d307d3154ccd8c87b4630509b"
            )),
        })
    }
    fn eip1559_tx_for_test(nonce: u64) -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id: 1,
            nonce,
            max_fee_per_gas: nonce * BASE_PRICE_IN_WEI,
            max_priority_fee_per_gas: (nonce * (10_u64.pow(9))).pow(2),
            gas_limit: 10000,
            to: TxKind::Create,
            value: 100.into(),
            data: Default::default(),
            access_list: vec![],
            signature_y_parity: true,
            signature_r: U256::default(),
            signature_s: U256::default(),
        })
    }
    fn setup_store() -> Store {
        let genesis: &str = include_str!("../../../../test_data/genesis-l1.json");
        let genesis: Genesis =
            serde_json::from_str(genesis).expect("Fatal: test config is invalid");
        let store = Store::new("test-store", EngineType::InMemory)
            .expect("Fail to create in-memory db test");
        store.add_initial_state(genesis).unwrap();
        store
    }
    #[test]
    fn test_for_legacy_txs() {
        let context = default_context();
        for block_num in 1..100 {
            let mut txs = vec![];
            for nonce in 1..=3 {
                let legacy_tx = legacy_tx_for_test(nonce);
                txs.push(legacy_tx)
            }
            let block_body = BlockBody {
                transactions: txs,
                ommers: Default::default(),
                withdrawals: Default::default(),
            };
            let block_header = test_header(block_num);
            let block = Block::new(block_header.clone(), block_body);
            context.storage.add_block(block).unwrap();
            context
                .storage
                .set_canonical_block(block_num, block_header.compute_block_hash())
                .unwrap();
            context
                .storage
                .update_latest_block_number(block_num)
                .unwrap();
        }
        let gas_price = GasPrice {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, 2000000000);
    }

    #[test]
    fn test_for_eip_1559_txs() {
        let context = default_context();
        for block_num in 1..100 {
            let mut txs = vec![];
            for nonce in 1..=3 {
                txs.push(eip1559_tx_for_test(nonce));
            }
            let block_body = BlockBody {
                transactions: txs,
                ommers: Default::default(),
                withdrawals: Default::default(),
            };
            let block_header = test_header(block_num);
            let block = Block::new(block_header.clone(), block_body);
            context.storage.add_block(block).unwrap();
            context
                .storage
                .set_canonical_block(block_num, block_header.compute_block_hash())
                .unwrap();
            context
                .storage
                .update_latest_block_number(block_num)
                .unwrap();
        }
        let gas_price = GasPrice {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, 2000000000);
    }
    #[test]
    fn test_with_mixed_transactions() {
        let context = default_context();
        for block_num in 1..100 {
            let txs = vec![
                legacy_tx_for_test(1),
                eip1559_tx_for_test(2),
                legacy_tx_for_test(3),
                eip1559_tx_for_test(4),
            ];
            let block_body = BlockBody {
                transactions: txs,
                ommers: Default::default(),
                withdrawals: Default::default(),
            };
            let block_header = test_header(block_num);
            let block = Block::new(block_header.clone(), block_body);
            context.storage.add_block(block).unwrap();
            context
                .storage
                .set_canonical_block(block_num, block_header.compute_block_hash())
                .unwrap();
            context
                .storage
                .update_latest_block_number(block_num)
                .unwrap();
        }
        let gas_price = GasPrice {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, 2000000000);
    }
    #[test]
    fn test_with_not_enough_blocks_or_transactions() {
        let context = default_context();
        for block_num in 1..10 {
            let txs = vec![legacy_tx_for_test(1)];
            let block_body = BlockBody {
                transactions: txs,
                ommers: Default::default(),
                withdrawals: Default::default(),
            };
            let block_header = test_header(block_num);
            let block = Block::new(block_header.clone(), block_body);
            context.storage.add_block(block).unwrap();
            context
                .storage
                .set_canonical_block(block_num, block_header.compute_block_hash())
                .unwrap();
            context
                .storage
                .update_latest_block_number(block_num)
                .unwrap();
        }
        let gas_price = GasPrice {};
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, 1000000000);
    }
    #[test]
    fn test_with_no_blocks_but_genesis() {
        let context = default_context();
        let gas_price = GasPrice {};
        // genesis base fee is 1_000_000_000
        let expected_gas_price = 1_000_000_000;
        let response = gas_price.handle(context).unwrap();
        let parsed_result = parse_json_hex(&response).unwrap();
        assert_eq!(parsed_result, expected_gas_price);
    }
    #[test]
    fn request_smoke_test() {
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

        for block_num in 1..100 {
            let txs = vec![legacy_tx_for_test(1)];
            let block_body = BlockBody {
                transactions: txs,
                ommers: Default::default(),
                withdrawals: Default::default(),
            };
            let block_header = test_header(block_num);
            let block = Block::new(block_header.clone(), block_body);
            context.storage.add_block(block).unwrap();
            context
                .storage
                .set_canonical_block(block_num, block_header.compute_block_hash())
                .unwrap();
            context
                .storage
                .update_latest_block_number(block_num)
                .unwrap();
        }
        let response = map_http_requests(&request, context).unwrap();
        assert_eq!(response, expected_response)
    }

    fn default_context() -> RpcApiContext {
        RpcApiContext {
            storage: setup_store(),
            jwt_secret: Default::default(),
            local_p2p_node: Node {
                ip: std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                udp_port: Default::default(),
                tcp_port: Default::default(),
                node_id: Default::default(),
            },
            active_filters: Default::default(),
            syncer: Arc::new(Mutex::new(SyncManager::dummy())),
        }
    }
}
