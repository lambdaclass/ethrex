use ethrex_common::types::block_access_list::{
    AccountChanges, BalanceChange, BlockAccessList, NonceChange, SlotChange, StorageChange,
};
use ethrex_common::types::{Block, BlockBody, BlockHeader};
use ethrex_common::{Address, H256, U256};
use ethrex_rpc::engine::payload::{
    GetPayloadBodiesByHashV2Request, GetPayloadBodiesByRangeV2Request,
};
use ethrex_rpc::map_eth_requests;
use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::types::payload::ExecutionPayloadBodyV2;
use ethrex_rpc::utils::RpcRequest;
use ethrex_storage::{EngineType, Store};
use std::str::FromStr;

// A small, structurally valid BAL used by the payload-body serving tests.
fn sample_bal() -> BlockAccessList {
    let address = Address::from_str("0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b").unwrap();
    let account = AccountChanges::new(address)
        .with_nonce_changes(vec![NonceChange::new(0, 1)])
        .with_balance_changes(vec![BalanceChange::new(0, U256::from(1u64))]);
    BlockAccessList::from_accounts(vec![account])
}

// Mirrors the `eth_getBlockAccessList` example in
// execution-apis/src/eth/block.yaml (schema at
// src/schemas/block-access-list.yaml). If this drifts, the endpoint is no
// longer wire-compatible.
#[tokio::test]
async fn eth_get_block_access_list_matches_spec_example() {
    let block_hash =
        H256::from_str("0x1111111111111111111111111111111111111111111111111111111111111111")
            .unwrap();

    let address = Address::from_str("0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b").unwrap();
    let slot = U256::zero();
    let slot_changes = vec![
        StorageChange::new(0, U256::zero()),
        StorageChange::new(1, U256::from(0x100u64)),
    ];
    let account = AccountChanges::new(address)
        .with_storage_changes(vec![SlotChange::with_changes(slot, slot_changes)])
        .with_balance_changes(vec![
            // 100 ETH and 100 ETH - 0x100000 wei, per the spec example.
            BalanceChange::new(0, U256::from_str_radix("56bc75e2d63100000", 16).unwrap()),
            BalanceChange::new(1, U256::from_str_radix("56bc75e2d63000000", 16).unwrap()),
        ])
        .with_nonce_changes(vec![NonceChange::new(0, 0), NonceChange::new(1, 1)]);
    let bal = BlockAccessList::from_accounts(vec![account]);

    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    storage
        .store_block_access_list(block_hash, &bal)
        .expect("store BAL");

    let body = format!(
        r#"{{
            "jsonrpc": "2.0",
            "method": "eth_getBlockAccessList",
            "params": ["{block_hash:#x}"],
            "id": 1
        }}"#
    );
    let request: RpcRequest = serde_json::from_str(&body).unwrap();
    let context = default_context_with_storage(storage).await;

    let got = map_eth_requests(&request, context).await.expect("rpc ok");

    let expected = serde_json::json!([{
        "address": "0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b",
        "storageChanges": [{
            "key": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "changes": [
                { "index": "0x0", "value": "0x0000000000000000000000000000000000000000000000000000000000000000" },
                { "index": "0x1", "value": "0x0000000000000000000000000000000000000000000000000000000000000100" },
            ],
        }],
        "storageReads": [],
        "balanceChanges": [
            { "index": "0x0", "value": "0x56bc75e2d63100000" },
            { "index": "0x1", "value": "0x56bc75e2d63000000" },
        ],
        "nonceChanges": [
            { "index": "0x0", "value": "0x0" },
            { "index": "0x1", "value": "0x1" },
        ],
        "codeChanges": [],
    }]);

    assert_eq!(got, expected);
}

// Unknown block hashes should return `null` per the `notFound` schema, not a
// JSON-RPC error.
#[tokio::test]
async fn eth_get_block_access_list_unknown_hash_returns_null() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    let context = default_context_with_storage(storage).await;

    let body = r#"{
        "jsonrpc": "2.0",
        "method": "eth_getBlockAccessList",
        "params": ["0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddead"],
        "id": 1
    }"#;
    let request: RpcRequest = serde_json::from_str(body).unwrap();

    let got = map_eth_requests(&request, context).await.expect("rpc ok");
    assert_eq!(got, serde_json::Value::Null);
}

// engine_getPayloadBodiesByHashV2 must serve the persisted BAL straight from the
// store, without re-executing the block. We store a block and its BAL but never
// build the state trie, so a regeneration fallback would fail (or, for this
// non-Amsterdam block, return None); a response carrying the stored BAL proves it
// was read from the store. This is the path that was failing on snap-synced nodes.
#[tokio::test]
async fn payload_bodies_by_hash_v2_serves_stored_bal() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");

    let block = Block {
        header: BlockHeader {
            number: 1,
            ..Default::default()
        },
        body: BlockBody::default(),
    };
    let block_hash = block.hash();
    storage.add_block(block).await.expect("store block");

    let bal = sample_bal();
    storage
        .store_block_access_list(block_hash, &bal)
        .expect("store BAL");

    let context = default_context_with_storage(storage).await;
    let request = GetPayloadBodiesByHashV2Request {
        hashes: vec![block_hash],
    };
    let got = request.handle(context).await.expect("rpc ok");

    let expected =
        serde_json::json!([
            serde_json::to_value(ExecutionPayloadBodyV2::from_body_with_bal(
                BlockBody::default(),
                Some(bal)
            ))
            .unwrap()
        ]);
    assert_eq!(got, expected);
}

// Same guarantee for the range variant: engine_getPayloadBodiesByRangeV2 returns
// the persisted BAL from the store without re-execution.
#[tokio::test]
async fn payload_bodies_by_range_v2_serves_stored_bal() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");

    let block = Block {
        header: BlockHeader {
            number: 1,
            ..Default::default()
        },
        body: BlockBody::default(),
    };
    let block_hash = block.hash();
    storage.add_block(block).await.expect("store block");
    // Make the block canonical and the latest so the range handler can find it.
    storage
        .forkchoice_update(vec![(1, block_hash)], 1, block_hash, None, None)
        .await
        .expect("forkchoice update");

    let bal = sample_bal();
    storage
        .store_block_access_list(block_hash, &bal)
        .expect("store BAL");

    let context = default_context_with_storage(storage).await;
    // params: [start, count] = [block 1, 1 block]
    let params = Some(vec![serde_json::json!("0x1"), serde_json::json!("0x1")]);
    let request = GetPayloadBodiesByRangeV2Request::parse(&params).expect("parse");
    let got = request.handle(context).await.expect("rpc ok");

    let expected =
        serde_json::json!([
            serde_json::to_value(ExecutionPayloadBodyV2::from_body_with_bal(
                BlockBody::default(),
                Some(bal)
            ))
            .unwrap()
        ]);
    assert_eq!(got, expected);
}
