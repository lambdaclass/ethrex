//! Historical-state reads must not answer from a different block's state.
//!
//! The on-disk trie is path-keyed and single-version, so opening it at a root that is no
//! longer retained does not fail — it resolves against whatever the latest committed root
//! holds. Unguarded, `eth_getBalance` and friends answer such a query with another block's
//! state and a success response. These tests pin that the guard fires (an error, not a
//! plausible wrong number) and that ordinary in-window reads are untouched.

use ethrex_common::types::{Block, BlockBody, BlockHeader};
use ethrex_common::{H256, types::Genesis};
use ethrex_rpc::map_eth_requests;
use ethrex_rpc::test_utils::default_context_with_storage;
use ethrex_rpc::utils::{RpcErr, RpcRequest};
use ethrex_storage::{EngineType, Store};

const ADDRESS: &str = "0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b";

/// A store whose canonical block 1 has a `state_root` the trie does not hold, standing in
/// for a block whose state has been committed out of the retained window.
async fn store_with_stateless_block() -> Store {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("create test DB");
    let header = BlockHeader {
        number: 1,
        // A root this store has never seen, so `has_state_root` is false for it — exactly
        // the condition of a block whose layers were committed away.
        state_root: H256::from_low_u64_be(0xdead_beef),
        base_fee_per_gas: Some(0),
        ..Default::default()
    };
    let block_hash = header.hash();
    storage
        .add_block(Block::new(header, BlockBody::default()))
        .await
        .expect("store block");
    storage
        .forkchoice_update(vec![(1, block_hash)], 1, block_hash, None, None)
        .await
        .expect("make block canonical");
    storage
}

fn request(method: &str, params: serde_json::Value) -> RpcRequest {
    serde_json::from_value(serde_json::json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    }))
    .expect("build request")
}

/// Every state read at a block whose state is gone must error rather than silently
/// resolving against the latest committed root. `eth_getProof` matters most: it would
/// otherwise return a proof that verifies against a root the caller never asked about.
#[tokio::test]
async fn state_reads_error_when_the_blocks_state_is_unavailable() {
    let cases = [
        ("eth_getBalance", serde_json::json!([ADDRESS, "0x1"])),
        ("eth_getCode", serde_json::json!([ADDRESS, "0x1"])),
        (
            "eth_getTransactionCount",
            serde_json::json!([ADDRESS, "0x1"]),
        ),
        (
            "eth_getStorageAt",
            serde_json::json!([ADDRESS, "0x0", "0x1"]),
        ),
        ("eth_getProof", serde_json::json!([ADDRESS, [], "0x1"])),
    ];

    for (method, params) in cases {
        let context = default_context_with_storage(store_with_stateless_block().await).await;
        match map_eth_requests(&request(method, params), context).await {
            Err(RpcErr::StateNotAvailable(msg)) => assert!(
                msg.contains("missing trie node"),
                "{method}: unexpected message: {msg}"
            ),
            Ok(value) => {
                panic!("{method} answered {value} from another block's state instead of erroring")
            }
            Err(other) => panic!("{method}: expected StateNotAvailable, got {other:?}"),
        }
    }
}

/// The guard must stay out of the way when the state really is there — otherwise it would
/// break every ordinary query. Genesis state is present here, so all five must succeed.
#[tokio::test]
async fn state_reads_within_the_retained_window_still_succeed() {
    let genesis: Genesis =
        serde_json::from_str(include_str!("../../../fixtures/genesis/execution-api.json"))
            .expect("parse genesis");
    let mut storage = Store::new("temp.db", EngineType::InMemory).expect("create test DB");
    storage
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");

    let cases = [
        ("eth_getBalance", serde_json::json!([ADDRESS, "0x0"])),
        ("eth_getCode", serde_json::json!([ADDRESS, "0x0"])),
        (
            "eth_getTransactionCount",
            serde_json::json!([ADDRESS, "0x0"]),
        ),
        (
            "eth_getStorageAt",
            serde_json::json!([ADDRESS, "0x0", "0x0"]),
        ),
        ("eth_getProof", serde_json::json!([ADDRESS, [], "0x0"])),
    ];

    for (method, params) in cases {
        let context = default_context_with_storage(storage.clone()).await;
        assert!(
            map_eth_requests(&request(method, params), context)
                .await
                .is_ok(),
            "{method} must still succeed when the state is present"
        );
    }
}
