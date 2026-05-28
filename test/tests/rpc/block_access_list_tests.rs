use ethrex_common::types::block_access_list::{
    AccountChanges, BalanceChange, BlockAccessList, NonceChange, SlotChange, StorageChange,
};
use ethrex_common::{Address, H256, U256};
use ethrex_rpc::map_eth_requests;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::utils::RpcRequest;
use ethrex_storage::{EngineType, Store};
use std::str::FromStr;

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
