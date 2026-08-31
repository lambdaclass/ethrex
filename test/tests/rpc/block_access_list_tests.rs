use ethrex_common::types::block_access_list::{
    AccountChanges, BalanceChange, BlockAccessList, NonceChange, SlotChange, StorageChange,
};
use ethrex_common::types::{Block, BlockBody, BlockHeader};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::engine::payload::{
    GetPayloadBodiesByHashV2Request, GetPayloadBodiesByRangeV2Request,
};
use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::types::payload::ExecutionPayloadBodyV2;
use ethrex_rpc::utils::{RpcErrorMetadata, RpcRequest};
use ethrex_rpc::{map_debug_requests, map_eth_requests};
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

// A store whose chain config has Amsterdam active from genesis, so headers with a
// `block_access_list_hash` are recognized as post-fork.
async fn amsterdam_store() -> Store {
    let mut storage =
        Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    let mut chain_config = storage.get_chain_config();
    chain_config.amsterdam_time = Some(0);
    storage
        .set_chain_config(&chain_config)
        .await
        .expect("set chain config");
    storage
}

// A header (number 1) that commits to `bal` via `block_access_list_hash`.
// All optional header fields preceding it are set so the header round-trips
// through RLP — `encode_optional_field` is trailing-only, so a lone trailing
// optional would otherwise be misdecoded into the first optional slot.
fn header_committing_to(bal: &BlockAccessList) -> BlockHeader {
    BlockHeader {
        number: 1,
        base_fee_per_gas: Some(0),
        withdrawals_root: Some(H256::zero()),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(H256::zero()),
        block_access_list_hash: Some(bal.compute_hash(&NativeCrypto)),
        ..Default::default()
    }
}

// Mirrors the `eth_getBlockAccessList` example in
// execution-apis/src/eth/block.yaml (schema at
// src/schemas/block-access-list.yaml). If this drifts, the endpoint is no
// longer wire-compatible.
#[tokio::test]
async fn eth_get_block_access_list_matches_spec_example() {
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

    let storage = amsterdam_store().await;
    // The endpoint validates the stored BAL against the header commitment, so the
    // block's header must commit to this BAL's hash.
    let block = Block {
        header: header_committing_to(&bal),
        body: BlockBody::default(),
    };
    let block_hash = block.hash();
    storage.add_block(block).await.expect("store block");
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

// A block predating Amsterdam has no access list at all, which execution-apis
// distinguishes from an unknown block: `-32001: Resource not found`.
#[tokio::test]
async fn eth_get_block_access_list_pre_amsterdam_is_resource_not_found() {
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

    let err = map_eth_requests(&request, context)
        .await
        .expect_err("pre-Amsterdam block must be an error");
    assert_eq!(RpcErrorMetadata::from(err).code, -32001);
}

// debug_getRawBlockAccessList returns the RLP encoding, and accepts a block hash
// (execution-apis `BlockNumberOrTagOrHash`).
#[tokio::test]
async fn debug_get_raw_block_access_list_serves_rlp_by_hash() {
    let storage = amsterdam_store().await;
    let bal = sample_bal();
    let block = Block {
        header: header_committing_to(&bal),
        body: BlockBody::default(),
    };
    let block_hash = block.hash();
    storage.add_block(block).await.expect("store block");
    storage
        .store_block_access_list(block_hash, &bal)
        .expect("store BAL");

    let body = format!(
        r#"{{
            "jsonrpc": "2.0",
            "method": "debug_getRawBlockAccessList",
            "params": ["{block_hash:#x}"],
            "id": 1
        }}"#
    );
    let request: RpcRequest = serde_json::from_str(&body).unwrap();
    let context = default_context_with_storage(storage).await;

    let got = map_debug_requests(&request, context).await.expect("rpc ok");
    let expected = format!("0x{}", hex::encode(bal.encode_to_vec()));
    assert_eq!(got, serde_json::Value::String(expected));
}

// The raw getter has no `null` result: an unknown block is `-32001`.
#[tokio::test]
async fn debug_get_raw_block_access_list_unknown_hash_is_resource_not_found() {
    let storage = amsterdam_store().await;
    let context = default_context_with_storage(storage).await;

    let body = r#"{
        "jsonrpc": "2.0",
        "method": "debug_getRawBlockAccessList",
        "params": ["0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddead"],
        "id": 1
    }"#;
    let request: RpcRequest = serde_json::from_str(body).unwrap();

    let err = map_debug_requests(&request, context)
        .await
        .expect_err("unknown block must be an error");
    assert_eq!(RpcErrorMetadata::from(err).code, -32001);
}

// An Amsterdam block whose body was pruned cannot be re-executed: `4444: Pruned
// history unavailable`. This was previously `null`, so it pins the getter's
// contract change.
#[tokio::test]
async fn eth_get_block_access_list_missing_body_is_pruned_history() {
    let storage = amsterdam_store().await;
    let header = header_committing_to(&sample_bal());
    let block_hash = header.hash();
    // Header only: the body was pruned, and no BAL was stored at import.
    storage
        .add_block_header(block_hash, header)
        .await
        .expect("store header");

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

    let err = map_eth_requests(&request, context)
        .await
        .expect_err("missing body must be an error");
    assert_eq!(RpcErrorMetadata::from(err).code, 4444);
}

// An Amsterdam block whose body survives but whose parent state was pruned
// cannot be re-executed either; that must also be `4444`, not `-32603` from the
// re-execution failing on the missing state root.
#[tokio::test]
async fn eth_get_block_access_list_missing_parent_state_is_pruned_history() {
    let storage = amsterdam_store().await;
    // Full block, no stored BAL: resolution has to try re-execution. Its parent
    // (hash zero here) is absent, standing in for a pruned ancestor.
    let block = Block {
        header: header_committing_to(&sample_bal()),
        body: BlockBody::default(),
    };
    let block_hash = block.hash();
    storage.add_block(block).await.expect("store block");

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

    let err = map_eth_requests(&request, context)
        .await
        .expect_err("missing parent state must be an error");
    assert_eq!(RpcErrorMetadata::from(err).code, 4444);
}

// An Amsterdam genesis has no parent to re-execute from; its access list is
// empty by construction. `0xc0` is the exact raw encoding execution-apis gives
// for a block with no state accesses.
#[tokio::test]
async fn debug_get_raw_block_access_list_genesis_serves_empty_rlp() {
    let storage = amsterdam_store().await;
    let genesis_bal = BlockAccessList::default();
    let genesis = Block {
        header: BlockHeader {
            number: 0,
            base_fee_per_gas: Some(0),
            withdrawals_root: Some(H256::zero()),
            blob_gas_used: Some(0),
            excess_blob_gas: Some(0),
            parent_beacon_block_root: Some(H256::zero()),
            requests_hash: Some(H256::zero()),
            block_access_list_hash: Some(genesis_bal.compute_hash(&NativeCrypto)),
            ..Default::default()
        },
        body: BlockBody::default(),
    };
    let block_hash = genesis.hash();
    storage.add_block(genesis).await.expect("store genesis");

    let body = format!(
        r#"{{
            "jsonrpc": "2.0",
            "method": "debug_getRawBlockAccessList",
            "params": ["{block_hash:#x}"],
            "id": 1
        }}"#
    );
    let request: RpcRequest = serde_json::from_str(&body).unwrap();
    let context = default_context_with_storage(storage).await;

    let got = map_debug_requests(&request, context).await.expect("rpc ok");
    assert_eq!(got, serde_json::Value::String("0xc0".to_owned()));
}

// Both getters take exactly one parameter; extra arguments are a parse error,
// consistent with the rest of the `debug_getRaw*` family.
#[tokio::test]
async fn eth_get_block_access_list_rejects_extra_params() {
    let storage = amsterdam_store().await;
    let context = default_context_with_storage(storage).await;

    let body = r#"{
        "jsonrpc": "2.0",
        "method": "eth_getBlockAccessList",
        "params": ["latest", "junk"],
        "id": 1
    }"#;
    let request: RpcRequest = serde_json::from_str(body).unwrap();

    let err = map_eth_requests(&request, context)
        .await
        .expect_err("extra params must be rejected");
    assert_eq!(RpcErrorMetadata::from(err).code, -32000);
}

// engine_getPayloadBodiesByHashV2 must serve the persisted BAL straight from the
// store, without re-executing the block. We store a block and its BAL but never
// build the state trie, so a regeneration fallback would fail (or, for this
// non-Amsterdam block, return None); a response carrying the stored BAL proves it
// was read from the store. This is the path that was failing on snap-synced nodes.
#[tokio::test]
async fn payload_bodies_by_hash_v2_serves_stored_bal() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");

    // The header must commit to the stored BAL's hash (EIP-8159); the serve
    // path validates the stored BAL against this commitment before returning it.
    let bal = sample_bal();
    let block = Block {
        header: header_committing_to(&bal),
        body: BlockBody::default(),
    };
    let block_hash = block.hash();
    storage.add_block(block).await.expect("store block");

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

    // The header must commit to the stored BAL's hash (EIP-8159); the serve
    // path validates the stored BAL against this commitment before returning it.
    let bal = sample_bal();
    let block = Block {
        header: header_committing_to(&bal),
        body: BlockBody::default(),
    };
    let block_hash = block.hash();
    storage.add_block(block).await.expect("store block");
    // Make the block canonical and the latest so the range handler can find it.
    storage
        .forkchoice_update(vec![(1, block_hash)], 1, block_hash, None, None)
        .await
        .expect("forkchoice update");

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

// Regression for glamsterdam-devnet-5 block 8501: a stored BAL whose hash does
// not match the header commitment must NOT be served. Here the header commits
// to `sample_bal()` but an empty BAL is stored under the block hash; the guard
// must drop it (and regeneration short-circuits to None for this block), so the
// body carries no BAL rather than a wrong one.
#[tokio::test]
async fn payload_bodies_by_hash_v2_drops_bal_not_matching_commitment() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");

    let bal = sample_bal();
    let block = Block {
        header: header_committing_to(&bal),
        body: BlockBody::default(),
    };
    let block_hash = block.hash();
    storage.add_block(block).await.expect("store block");

    // Store a BAL that does NOT match the header commitment (the 8501 bug).
    storage
        .store_block_access_list(block_hash, &BlockAccessList::from_accounts(vec![]))
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
                None
            ))
            .unwrap()
        ]);
    assert_eq!(
        got, expected,
        "a stored BAL not matching the header commitment must not be served"
    );
}

// Fallback path: when no BAL is stored, the handler must not re-execute a block
// whose parent state is unavailable. For a pre-Amsterdam block, regeneration
// short-circuits to None, so a None-carrying body proves the response was
// produced without touching (absent) historical state. This locks in the
// snap-sync / aged-out-state safety the PR targets.
#[tokio::test]
async fn payload_bodies_by_hash_v2_pre_amsterdam_returns_none_without_re_execution() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");

    // Pre-Amsterdam block (timestamp 0); no BAL stored and no state trie built.
    let block = Block {
        header: BlockHeader {
            number: 1,
            ..Default::default()
        },
        body: BlockBody::default(),
    };
    let block_hash = block.hash();
    storage.add_block(block).await.expect("store block");

    let context = default_context_with_storage(storage).await;
    let request = GetPayloadBodiesByHashV2Request {
        hashes: vec![block_hash],
    };
    let got = request.handle(context).await.expect("rpc ok");

    let expected =
        serde_json::json!([
            serde_json::to_value(ExecutionPayloadBodyV2::from_body_with_bal(
                BlockBody::default(),
                None
            ))
            .unwrap()
        ]);
    assert_eq!(got, expected);
}
