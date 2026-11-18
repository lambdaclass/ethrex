use ethrex_rpc::test_utils::{
    call_authrpc, default_context_with_storage, jwt_auth_header_for, setup_store,
};
use ethrex_storage::{EngineType, Store};

/// Regression test for engine-port batch parsing. Prior to the fix,
/// `handle_authrpc_request` deserialized directly into `RpcRequest` and
/// rejected JSON-RPC 2.0 batches with `Invalid request body`. Prysm's
/// `execution_payload_envelopes_by_root` handler batches `eth_getBlockByHash`
/// against the engine port (auth RPC also serves the `eth_*` namespace), which
/// caused glamsterdam-devnet-4 forking. This test uses `eth_chainId` to
/// exercise the same routing path (`map_authrpc_requests` -> `map_eth_requests`)
/// that Prysm hits.
#[tokio::test]
async fn authrpc_accepts_batched_eth_requests() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;
    let auth = jwt_auth_header_for(&context);

    let body = r#"[
        {"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1},
        {"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":2}
    ]"#
    .to_string();

    let value = call_authrpc(&context, auth, body).await;
    let arr = value
        .as_array()
        .expect("batched auth response must be a JSON array");
    assert_eq!(arr.len(), 2, "expected 2 responses, got {value}");
    for (i, item) in arr.iter().enumerate() {
        assert!(
            item.get("result").is_some(),
            "response {i} should have a result field, got {item}"
        );
        assert_eq!(
            item.get("id").and_then(|v| v.as_u64()),
            Some((i + 1) as u64)
        );
    }
}

/// JSON-RPC 2.0 §4.2: empty batch is itself an Invalid Request. Response code
/// must be -32600, and per §5.1 the id must be null when the request can't be
/// associated with a single object.
#[tokio::test]
async fn authrpc_rejects_empty_batch() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    let context = default_context_with_storage(storage).await;
    let auth = jwt_auth_header_for(&context);

    let value = call_authrpc(&context, auth, "[]".to_string()).await;
    let err = value
        .get("error")
        .expect("empty batch must produce an error response");
    assert_eq!(err.get("code").and_then(|v| v.as_i64()), Some(-32600));
    assert!(value.get("id").map(|v| v.is_null()).unwrap_or(false));
}

/// Batches larger than `MAX_BATCH_SIZE` (1000) must be rejected before any
/// dispatch work runs, to keep a 100k-request body from burning CPU or memory
/// on the engine port. Matches geth's `--engine.batchitemlimit` default.
#[tokio::test]
async fn authrpc_rejects_oversize_batch() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    let context = default_context_with_storage(storage).await;
    let auth = jwt_auth_header_for(&context);

    let reqs: Vec<String> = (0..1001)
        .map(|i| format!(r#"{{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":{i}}}"#))
        .collect();
    let body = format!("[{}]", reqs.join(","));

    let value = call_authrpc(&context, auth, body).await;
    let err = value
        .get("error")
        .expect("oversize batch must produce an error response");
    assert_eq!(err.get("code").and_then(|v| v.as_i64()), Some(-32600));
    assert!(value.get("id").map(|v| v.is_null()).unwrap_or(false));
}
