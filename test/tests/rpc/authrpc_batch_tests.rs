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

    let value = call_authrpc(context, auth, body).await;
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

    let value = call_authrpc(context, auth, "[]".to_string()).await;
    let err = value
        .get("error")
        .expect("empty batch must produce an error response");
    assert_eq!(err.get("code").and_then(|v| v.as_i64()), Some(-32600));
    assert!(value.get("id").map(|v| v.is_null()).unwrap_or(false));
}

/// Auth failure on a batched body must preserve the batch shape so clients can
/// still correlate the failure with each original request id. Without this the
/// caller sees a single error with a synthetic id and has no way to tell which
/// of N requests triggered it.
#[tokio::test]
async fn authrpc_batch_auth_failure_preserves_ids() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    // No auth header at all: authenticate() returns MissingAuthentication.
    let body = r#"[
        {"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1},
        {"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":7}
    ]"#
    .to_string();

    let value = call_authrpc(context, None, body).await;
    let arr = value
        .as_array()
        .expect("batched auth failure must return a JSON array, got {value:?}");
    assert_eq!(arr.len(), 2);
    let ids: Vec<u64> = arr
        .iter()
        .map(|item| item.get("id").and_then(|v| v.as_u64()).unwrap())
        .collect();
    assert_eq!(ids, vec![1, 7], "ids should match the request batch");
    for item in arr {
        let err = item.get("error").expect("each entry must be an error");
        assert_eq!(err.get("code").and_then(|v| v.as_i64()), Some(-32000));
        assert!(
            err.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .contains("Auth"),
            "expected auth error message, got {err}"
        );
    }
}

/// Single-request auth failure still goes back as a single object (not wrapped
/// in an array) and echoes the original id.
#[tokio::test]
async fn authrpc_single_auth_failure_keeps_request_id() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = r#"{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":42}"#.to_string();
    let value = call_authrpc(context, None, body).await;
    assert_eq!(value.get("id").and_then(|v| v.as_u64()), Some(42));
    let err = value.get("error").expect("auth failure must error");
    assert_eq!(err.get("code").and_then(|v| v.as_i64()), Some(-32000));
}

/// Batches larger than `MAX_BATCH_SIZE` (1000) must be rejected before any
/// JWT-auth or dispatch work runs, to keep a 100k-request body from burning
/// crypto or memory on the engine port. Matches geth's
/// `--engine.batchitemlimit` default.
#[tokio::test]
async fn authrpc_rejects_oversize_batch() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    let context = default_context_with_storage(storage).await;
    let auth = jwt_auth_header_for(&context);

    let reqs: Vec<String> = (0..1001)
        .map(|i| format!(r#"{{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":{i}}}"#))
        .collect();
    let body = format!("[{}]", reqs.join(","));

    let value = call_authrpc(context, auth, body).await;
    let err = value
        .get("error")
        .expect("oversize batch must produce an error response");
    assert_eq!(err.get("code").and_then(|v| v.as_i64()), Some(-32600));
    assert!(value.get("id").map(|v| v.is_null()).unwrap_or(false));
}
