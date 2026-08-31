use ethrex_common::H256;
use ethrex_common::types::{Block, BlockBody, BlockHeader};
use ethrex_rpc::map_eth_requests;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::utils::RpcRequest;
use ethrex_storage::{EngineType, Store};

// A header at `number` whose hash is varied via the timestamp. The optional
// fields are set so the header round-trips through RLP (trailing-optional
// encoding, see `encode_optional_field`).
fn header_at(number: u64, timestamp: u64) -> BlockHeader {
    BlockHeader {
        number,
        timestamp,
        base_fee_per_gas: Some(0),
        withdrawals_root: Some(H256::zero()),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(H256::zero()),
        ..Default::default()
    }
}

async fn get_block_by_hash(storage: Store, hash: H256) -> serde_json::Value {
    let body = format!(
        r#"{{
            "jsonrpc": "2.0",
            "method": "eth_getBlockByHash",
            "params": ["{hash:#x}", false],
            "id": 1
        }}"#
    );
    let request: RpcRequest = serde_json::from_str(&body).unwrap();
    let context = default_context_with_storage(storage).await;
    map_eth_requests(&request, context).await.expect("rpc ok")
}

// eth_getBlockByHash must return the block stored FOR the requested hash. A
// non-canonical hash previously resolved hash -> number -> canonical block,
// answering with the canonical sibling at the same height: a different block
// than requested, whose `hash` field did not even echo the query. Prysm's ePBS
// envelope reconstruction fetches fork blocks by hash and rejects exactly that
// mismatch ("execution block hash mismatch").
#[tokio::test]
async fn get_block_by_hash_returns_the_requested_noncanonical_block() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");

    let canonical = Block {
        header: header_at(1, 12),
        body: BlockBody::default(),
    };
    let sibling = Block {
        header: header_at(1, 24),
        body: BlockBody::default(),
    };
    let canonical_hash = canonical.hash();
    let sibling_hash = sibling.hash();
    assert_ne!(canonical_hash, sibling_hash);

    storage.add_block(canonical).await.expect("store canonical");
    storage.add_block(sibling).await.expect("store sibling");
    storage
        .forkchoice_update(vec![], 1, canonical_hash, None, None)
        .await
        .expect("make canonical");

    let got = get_block_by_hash(storage.clone(), sibling_hash).await;
    assert_eq!(
        got["hash"],
        serde_json::json!(format!("{sibling_hash:#x}")),
        "the response must be the block the caller asked for, not its canonical sibling"
    );

    let got = get_block_by_hash(storage, canonical_hash).await;
    assert_eq!(
        got["hash"],
        serde_json::json!(format!("{canonical_hash:#x}"))
    );
}

// Unknown hashes keep returning `null` per the execution-apis `notFound` schema.
#[tokio::test]
async fn get_block_by_hash_unknown_hash_returns_null() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    let got = get_block_by_hash(storage, H256::repeat_byte(0xde)).await;
    assert_eq!(got, serde_json::Value::Null);
}
