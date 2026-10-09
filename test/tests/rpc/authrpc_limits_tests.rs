//! The Auth-RPC listener's guards run in front of the JSON-RPC handler: the JWT check
//! and the in-flight body budget both answer from the request headers alone, so these
//! tests drive a real server and often send a request head without its body.

use bytes::Bytes;
use ethrex_rpc::test_utils::{
    call_authrpc, default_context_with_storage, jwt_auth_header_for, post_authrpc, read_response,
    send_request_head, setup_store, spawn_authrpc_server,
};
use ethrex_rpc::{AUTHRPC_MAX_BODY_SIZE, DEFAULT_AUTHRPC_MAX_INFLIGHT_BODY_SIZE};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

const CHAIN_ID_REQUEST: &str = r#"{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}"#;

/// `eth_chainId` padded with trailing whitespace to exactly `len` bytes.
fn padded_chain_id_request(len: usize) -> String {
    format!("{CHAIN_ID_REQUEST:<len$}")
}

/// Asserts `body` is a JSON-RPC error with a null id and the given code.
fn assert_null_id_error(body: &str, code: i64) {
    let value: Value = serde_json::from_str(body).expect("JSON-RPC error body");
    assert_eq!(value["error"]["code"], code, "unexpected error: {value}");
    assert!(value["id"].is_null(), "id must be null: {value}");
}

/// Like geth and reth, a request without a token gets a plain-text 401, and it gets it
/// from the headers alone: the declared body is never sent.
#[tokio::test]
async fn authrpc_rejects_missing_token_before_reading_body() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server(&context, DEFAULT_AUTHRPC_MAX_INFLIGHT_BODY_SIZE).await;
    let addr = server.addr;

    let mut stream = send_request_head(addr, None, Some(64 * 1024 * 1024)).await;
    let (status, body) = read_response(&mut stream).await;
    assert_eq!(status, 401);
    assert_eq!(body, "missing token");

    drop(stream);
    server.shutdown().await;
}

/// A token signed with the wrong secret is rejected the same way.
#[tokio::test]
async fn authrpc_rejects_invalid_token_before_reading_body() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server(&context, DEFAULT_AUTHRPC_MAX_INFLIGHT_BODY_SIZE).await;
    let addr = server.addr;
    let mut other_secret = context.clone();
    other_secret.node_data.jwt_secret = Bytes::from_static(b"not the node's secret");
    let wrong_token = jwt_auth_header_for(&other_secret);

    let mut stream = send_request_head(addr, wrong_token.as_ref(), Some(64 * 1024 * 1024)).await;
    let (status, body) = read_response(&mut stream).await;
    assert_eq!(status, 401);
    assert_eq!(body, "invalid token");

    drop(stream);
    server.shutdown().await;
}

/// A declared body over the per-request limit gets a 413 with a JSON-RPC error before
/// any of it is read.
#[tokio::test]
async fn authrpc_rejects_declared_oversize_body_before_reading_it() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server(&context, DEFAULT_AUTHRPC_MAX_INFLIGHT_BODY_SIZE).await;
    let addr = server.addr;
    let auth = jwt_auth_header_for(&context);

    let mut stream = send_request_head(addr, auth.as_ref(), Some(AUTHRPC_MAX_BODY_SIZE + 1)).await;
    let (status, body) = read_response(&mut stream).await;
    assert_eq!(status, 413);
    assert_null_id_error(&body, -32600);

    drop(stream);
    server.shutdown().await;
}

/// With a budget below the per-request limit, the budget becomes the per-request limit:
/// a body that could never fit gets a 413, not a 503 that invites retries.
#[tokio::test]
async fn authrpc_body_larger_than_the_budget_is_too_large() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server(&context, 1024).await;
    let addr = server.addr;
    let auth = jwt_auth_header_for(&context);

    let (status, body) = post_authrpc(addr, auth, padded_chain_id_request(2048)).await;
    assert_eq!(status, 413);
    assert_null_id_error(&body, -32600);

    server.shutdown().await;
}

/// Two requests whose bodies fit the budget one at a time but not together: the one that
/// reaches the budget second gets a 503 while the first is still in flight, and the
/// budget is released once the first completes.
#[tokio::test]
async fn authrpc_rejects_requests_over_the_inflight_budget() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server(&context, 4096).await;
    let addr = server.addr;
    let auth = jwt_auth_header_for(&context);
    let body = padded_chain_id_request(3000);

    // Neither body is sent yet, so the request that reserved its size first is held in
    // flight waiting for it, and only the other one can be answered.
    let mut first = send_request_head(addr, auth.as_ref(), Some(body.len())).await;
    let mut second = send_request_head(addr, auth.as_ref(), Some(body.len())).await;
    let first_was_rejected = tokio::select! {
        (status, response) = read_response(&mut first) => {
            assert_eq!(status, 503);
            assert_null_id_error(&response, -32603);
            true
        }
        (status, response) = read_response(&mut second) => {
            assert_eq!(status, 503);
            assert_null_id_error(&response, -32603);
            false
        }
    };
    let in_flight = if first_was_rejected {
        &mut second
    } else {
        &mut first
    };

    in_flight.write_all(body.as_bytes()).await.unwrap();
    let (status, response) = read_response(in_flight).await;
    assert_eq!(status, 200, "in-flight request must complete: {response}");
    let value: Value = serde_json::from_str(&response).unwrap();
    assert!(value["result"].is_string(), "expected a chain id: {value}");

    let (status, response) = post_authrpc(addr, auth, body).await;
    assert_eq!(status, 200, "budget must be released: {response}");

    drop((first, second));
    server.shutdown().await;
}

/// A chunked body has no length to reserve up front, so it reserves the whole
/// per-request cap, which here is the whole budget: it cannot be in flight alongside even
/// a one-byte request.
#[tokio::test]
async fn authrpc_chunked_body_reserves_the_per_request_cap() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server(&context, 4096).await;
    let addr = server.addr;
    let auth = jwt_auth_header_for(&context);

    // Whichever of the two reserves first is held in flight waiting for its body; the
    // other can only be refused if the chunked one claims the whole budget.
    let mut chunked = send_request_head(addr, auth.as_ref(), None).await;
    let mut one_byte = send_request_head(addr, auth.as_ref(), Some(1)).await;
    let chunked_was_rejected = tokio::select! {
        (status, response) = read_response(&mut chunked) => {
            assert_eq!(status, 503);
            assert_null_id_error(&response, -32603);
            true
        }
        (status, response) = read_response(&mut one_byte) => {
            assert_eq!(status, 503);
            assert_null_id_error(&response, -32603);
            false
        }
    };

    if chunked_was_rejected {
        one_byte.write_all(b"1").await.unwrap();
        let (status, _) = read_response(&mut one_byte).await;
        assert_eq!(status, 200);
    } else {
        let chunk = format!(
            "{:x}\r\n{CHAIN_ID_REQUEST}\r\n0\r\n\r\n",
            CHAIN_ID_REQUEST.len()
        );
        chunked.write_all(chunk.as_bytes()).await.unwrap();
        let (status, response) = read_response(&mut chunked).await;
        assert_eq!(status, 200, "chunked request must complete: {response}");
    }

    drop((chunked, one_byte));
    server.shutdown().await;
}

/// Params beyond the cap are skipped while parsing, but the request still reaches its
/// handler, whose arity check answers with the request's own id.
#[tokio::test]
async fn authrpc_too_many_params_get_the_method_arity_error() {
    let context = default_context_with_storage(setup_store().await).await;
    let auth = jwt_auth_header_for(&context);
    let params = vec![r#""0x0""#; 100_000].join(",");
    let body =
        format!(r#"{{"jsonrpc":"2.0","method":"eth_getBalance","params":[{params}],"id":7}}"#);

    let value = call_authrpc(&context, auth, body).await;
    assert_eq!(value["id"], 7, "the request id must be echoed: {value}");
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("Expected 2 params")),
        "expected the arity error, got {value}"
    );
}
