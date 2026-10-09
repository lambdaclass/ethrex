//! The Auth-RPC listener's guards run in front of the JSON-RPC handler: the JWT check
//! and the in-flight body budget both answer from the request headers alone, so these
//! tests drive a real server and often send a request head without its body.

use bytes::Bytes;
use ethrex_rpc::test_utils::{
    call_authrpc, default_context_with_storage, jwt_auth_header_for, post_authrpc, read_response,
    send_request_head, setup_store, spawn_authrpc_server, spawn_authrpc_server_with_timeouts,
};
use ethrex_rpc::{AUTHRPC_MAX_BODY_SIZE, DEFAULT_AUTHRPC_MAX_INFLIGHT_BODY_SIZE};
use serde_json::Value;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

const CHAIN_ID_REQUEST: &str = r#"{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}"#;

/// Budget wait for tests that expect a 503: short, so the refusal comes quickly.
const SHORT_WAIT: Duration = Duration::from_secs(1);

/// Longer than any of these tests runs, so it never fires.
const NO_TIMEOUT: Duration = Duration::from_secs(600);

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

/// Of two requests that fit the budget one at a time but not together, waits for the one
/// refused with a 503 when its budget wait runs out, and returns the other, which holds
/// its reservation while it waits for its body.
async fn request_still_in_flight(mut first: TcpStream, mut second: TcpStream) -> TcpStream {
    let first_was_refused = tokio::select! {
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
    if first_was_refused { second } else { first }
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

/// The JWT check runs before the budget, so a request without a token never reserves any
/// of it: with a budget smaller than the declared body, it gets the 401, not the 413.
#[tokio::test]
async fn authrpc_checks_the_token_before_the_budget() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server(&context, 1024).await;

    let mut stream = send_request_head(server.addr, None, Some(2048)).await;
    let (status, body) = read_response(&mut stream).await;
    assert_eq!(status, 401);
    assert_eq!(body, "missing token");

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
/// reaches the budget second waits, and gets a 503 when the first is still in flight
/// after the wait timeout. The budget is released once the first completes.
#[tokio::test]
async fn authrpc_rejects_requests_over_the_inflight_budget() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server_with_timeouts(&context, 4096, SHORT_WAIT, NO_TIMEOUT).await;
    let addr = server.addr;
    let auth = jwt_auth_header_for(&context);
    let body = padded_chain_id_request(3000);

    // Neither body is sent yet, so the request that reserved its size first is held in
    // flight waiting for it, and only the other one can be answered.
    let first = send_request_head(addr, auth.as_ref(), Some(body.len())).await;
    let second = send_request_head(addr, auth.as_ref(), Some(body.len())).await;
    let mut in_flight = request_still_in_flight(first, second).await;

    in_flight.write_all(body.as_bytes()).await.unwrap();
    let (status, response) = read_response(&mut in_flight).await;
    assert_eq!(status, 200, "in-flight request must complete: {response}");
    let value: Value = serde_json::from_str(&response).unwrap();
    assert!(value["result"].is_string(), "expected a chain id: {value}");

    let (status, response) = post_authrpc(addr, auth, body).await;
    assert_eq!(status, 200, "budget must be released: {response}");

    drop(in_flight);
    server.shutdown().await;
}

/// A request that does not fit the budget yet waits for room instead of failing: once the
/// request ahead of it completes, it is served too.
#[tokio::test]
async fn authrpc_requests_wait_for_room_in_the_budget() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server(&context, 4096).await;
    let addr = server.addr;
    let auth = jwt_auth_header_for(&context);
    let body = padded_chain_id_request(3000);

    let mut first = send_request_head(addr, auth.as_ref(), Some(body.len())).await;
    let mut second = send_request_head(addr, auth.as_ref(), Some(body.len())).await;
    // Give both heads time to reach the budget, so one of them is waiting for the other.
    tokio::time::sleep(Duration::from_millis(200)).await;
    first.write_all(body.as_bytes()).await.unwrap();
    second.write_all(body.as_bytes()).await.unwrap();

    let ((first_status, first_response), (second_status, second_response)) =
        tokio::join!(read_response(&mut first), read_response(&mut second));
    assert_eq!(first_status, 200, "first request failed: {first_response}");
    assert_eq!(
        second_status, 200,
        "second request failed: {second_response}"
    );

    drop((first, second));
    server.shutdown().await;
}

/// A client that disconnects partway through its body releases its reservation.
#[tokio::test]
async fn authrpc_disconnect_mid_body_releases_the_budget() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server_with_timeouts(&context, 4096, SHORT_WAIT, NO_TIMEOUT).await;
    let addr = server.addr;
    let auth = jwt_auth_header_for(&context);
    let body = padded_chain_id_request(3000);

    let first = send_request_head(addr, auth.as_ref(), Some(body.len())).await;
    let second = send_request_head(addr, auth.as_ref(), Some(body.len())).await;
    let mut in_flight = request_still_in_flight(first, second).await;
    in_flight
        .write_all(&body.as_bytes()[..body.len() / 2])
        .await
        .unwrap();
    drop(in_flight);

    // The server notices the disconnect asynchronously, so retry until the budget is back.
    let released = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (status, response) = post_authrpc(addr, auth.clone(), body.clone()).await;
            if status == 200 {
                break;
            }
            assert_eq!(status, 503, "unexpected response: {response}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        released.is_ok(),
        "the budget was not released after the client disconnected"
    );

    server.shutdown().await;
}

/// A body that does not arrive within the read timeout gets a 408 and its reservation is
/// released, so a client that sends slowly cannot hold the budget.
#[tokio::test]
async fn authrpc_body_not_received_in_time_is_refused() {
    let context = default_context_with_storage(setup_store().await).await;
    let server =
        spawn_authrpc_server_with_timeouts(&context, 4096, SHORT_WAIT, Duration::from_secs(1))
            .await;
    let addr = server.addr;
    let auth = jwt_auth_header_for(&context);
    let body = padded_chain_id_request(3000);

    let mut slow = send_request_head(addr, auth.as_ref(), Some(body.len())).await;
    slow.write_all(&body.as_bytes()[..body.len() / 2])
        .await
        .unwrap();
    let (status, response) = read_response(&mut slow).await;
    assert_eq!(status, 408);
    assert_null_id_error(&response, -32600);

    let (status, response) = post_authrpc(addr, auth, body).await;
    assert_eq!(status, 200, "budget must be released: {response}");

    drop(slow);
    server.shutdown().await;
}

/// A chunked body has no length to reserve up front, so it reserves the whole
/// per-request cap, which here is the whole budget: it cannot be in flight alongside even
/// a one-byte request.
#[tokio::test]
async fn authrpc_chunked_body_reserves_the_per_request_cap() {
    let context = default_context_with_storage(setup_store().await).await;
    let server = spawn_authrpc_server_with_timeouts(&context, 4096, SHORT_WAIT, NO_TIMEOUT).await;
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
