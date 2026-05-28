use ethrex_rpc::test_utils::{call_http, default_context_with_storage};
use ethrex_storage::{EngineType, Store};

/// JSON-RPC 2.0 §4.2: empty batch is itself an Invalid Request. The public
/// HTTP port must reject `[]` the same way the engine auth port does;
/// historically this returned `[]` and silently succeeded.
#[tokio::test]
async fn http_rejects_empty_batch() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    let context = default_context_with_storage(storage).await;

    let value = call_http(context, "[]".to_string()).await;
    let err = value
        .get("error")
        .expect("empty batch must produce an error response");
    assert_eq!(err.get("code").and_then(|v| v.as_i64()), Some(-32600));
    assert!(value.get("id").map(|v| v.is_null()).unwrap_or(false));
}

/// Batches larger than `MAX_BATCH_SIZE` (1000) must be rejected before any
/// dispatch work runs on the public HTTP port too. Matches geth's
/// `--rpc.batch-request-limit` default.
#[tokio::test]
async fn http_rejects_oversize_batch() {
    let storage = Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
    let context = default_context_with_storage(storage).await;

    let reqs: Vec<String> = (0..1001)
        .map(|i| format!(r#"{{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":{i}}}"#))
        .collect();
    let body = format!("[{}]", reqs.join(","));

    let value = call_http(context, body).await;
    let err = value
        .get("error")
        .expect("oversize batch must produce an error response");
    assert_eq!(err.get("code").and_then(|v| v.as_i64()), Some(-32600));
    assert!(value.get("id").map(|v| v.is_null()).unwrap_or(false));
}
