use ethrex_blockchain::constants::MIN_GAS_LIMIT;
use tracing::error;

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

// TODO: Maybe these constants should be some kind of config.
// How many transactions to take as a price sample from a block.
const TXS_SAMPLE_SIZE: usize = 3;
// How many blocks we'll go back to calculate the estimate.
const BLOCK_RANGE_LOWER_BOUND_DEC: u64 = 20;

impl RpcHandler for GasPrice {
    fn parse(_: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(GasPrice {})
    }

    // Disclaimer:
    // This estimation is somewhat based on how currently go-ethereum does it.
    // Reference: https://github.com/ethereum/go-ethereum/blob/368e16f39d6c7e5cce72a92ec289adbfbaed4854/eth/gasprice/gasprice.go#L153
    // Although it will (probably) not yield the same result.
    // The idea here is to:
    // - Take the last 20 blocks (100% arbitrary, this could be more or less blocks)
    // - For each block, take the 3 txs with the lowest gas price (100% arbitrary)
    // - Join every fetched tx into a single vec and sort it.
    // - Return the one in the middle (what is also known as the 'median sample')
    // The intuition here is that we're sampling already accepted transactions,
    // fetched from recent blocks, so they should be real, representative values.
    // This specific implementation probably is not the best way to do this
    // but it works for now for a simple estimation, in the future
    // we can look into more sophisticated estimation methods, if needed.
    /// Estimate Gas Price based on already accepted transactions,
    /// as per the spec, this will be returned in wei.
    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let latest_block_number = context.storage.get_latest_block_number()?;
        let block_range_lower_bound =
            latest_block_number.saturating_sub(BLOCK_RANGE_LOWER_BOUND_DEC);
        // These are the blocks we'll use to estimate the price.
        let block_range = block_range_lower_bound..=latest_block_number;
        if block_range.is_empty() {
            error!(
                "Calculated block range from block {} \
                    up to block {} for gas price estimation is empty",
                block_range_lower_bound, latest_block_number
            );
            return Err(RpcErr::Internal("Error calculating gas price".to_string()));
        }
        let mut results = vec![];
        // TODO: Estimating gas price involves querying multiple blocks
        // and doing some calculations with each of them, let's consider
        // caching this result, also we can have a specific DB method
        // that returns a block range to not query them one-by-one.
        for block_num in block_range {
            let Some(block_body) = context.storage.get_block_body(block_num)? else {
                error!("Block body for block number {block_num} is missing but is below the latest known block!");
                return Err(RpcErr::Internal(
                    "Error calculating gas price: missing data".to_string(),
                ));
            };
            let mut gas_price_samples = block_body
                .transactions
                .into_iter()
                .map(|tx| tx.gas_price())
                .collect::<Vec<u64>>();
            gas_price_samples.sort();
            results.extend(gas_price_samples.into_iter().take(TXS_SAMPLE_SIZE));
        }
        results.sort();

        let sample_gas = match results.get(results.len() / 2) {
            Some(gas) => *gas,
            None => {
                // If we don't have enough samples, we'll return the base fee or the min gas limit as a default.
                context
                    .storage
                    .get_block_header(latest_block_number)
                    .ok()
                    .flatten()
                    .and_then(|header| header.base_fee_per_gas)
                    .unwrap_or(MIN_GAS_LIMIT)
            }
        };

        let gas_as_hex = format!("0x{:x}", sample_gas);
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
            r: U256::from(hex!(
                "7e09e26678ed4fac08a249ebe8ed680bf9051a5e14ad223e4b2b9d26e0208f37"
            )),
            s: U256::from(hex!(
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
