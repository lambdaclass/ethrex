use ethrex_repl::client::RpcClient;
use ethrex_repl::repl::Repl;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A mock JSON-RPC server that returns canned responses. Aborts the server
/// task on drop, so tests don't need explicit cleanup.
struct MockServer {
    endpoint: String,
    handle: tokio::task::JoinHandle<()>,
}

impl MockServer {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());

        let handle = tokio::spawn(async move {
            loop {
                let (mut stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);

                    let response = if let Some(body_start) = request.find("\r\n\r\n") {
                        make_response(&request[body_start + 4..])
                    } else {
                        r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"bad request"},"id":1}"#
                            .to_string()
                    };

                    let http_response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        response.len(),
                        response
                    );
                    let _ = stream.write_all(http_response.as_bytes()).await;
                });
            }
        });

        Self { endpoint, handle }
    }

    fn repl(&self) -> Repl {
        Repl::new(
            RpcClient::new(self.endpoint.clone()),
            "/tmp/ethrex_repl_test_history".to_string(),
        )
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn make_response(body: &str) -> String {
    let req: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

    let result = match method {
        "eth_blockNumber" => serde_json::json!("0x10d4f"),
        "eth_chainId" => serde_json::json!("0x1"),
        "eth_getBalance" => serde_json::json!("0xde0b6b3a7640000"),
        "eth_getTransactionCount" => serde_json::json!("0x5"),
        "eth_gasPrice" => serde_json::json!("0x3b9aca00"),
        "web3_clientVersion" => serde_json::json!("ethrex/v0.1.0"),
        "net_version" => serde_json::json!("1"),
        "net_peerCount" => serde_json::json!("0x19"),
        "eth_getBlockByNumber" => serde_json::json!(null),
        _ => serde_json::json!(null),
    };

    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
    .to_string()
}

// ── RPC commands ───────────────────────────────────────────────

#[tokio::test]
async fn test_eth_block_number() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("eth.blockNumber").await;
    assert!(result.contains("68943"), "expected 68943, got: {result}");
}

#[tokio::test]
async fn test_eth_chain_id() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("eth.chainId").await;
    assert!(result.contains("1"), "expected 1, got: {result}");
}

#[tokio::test]
async fn test_eth_get_balance() {
    let server = MockServer::start().await;
    let result = server
        .repl()
        .execute_command("eth.getBalance 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045")
        .await;
    assert!(
        result.contains("1000000000000000000"),
        "expected 1 ETH in wei, got: {result}"
    );
}

#[tokio::test]
async fn test_eth_get_balance_with_block() {
    let server = MockServer::start().await;
    let result = server
        .repl()
        .execute_command("eth.getBalance 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045 latest")
        .await;
    assert!(
        result.contains("1000000000000000000"),
        "expected 1 ETH in wei, got: {result}"
    );
}

#[tokio::test]
async fn test_eth_gas_price() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("eth.gasPrice").await;
    assert!(
        result.contains("1000000000"),
        "expected 1 gwei in wei, got: {result}"
    );
}

#[tokio::test]
async fn test_web3_client_version() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("web3.clientVersion").await;
    assert!(
        result.contains("ethrex"),
        "expected 'ethrex', got: {result}"
    );
}

#[tokio::test]
async fn test_net_version() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("net.version").await;
    assert!(result.contains("1"), "expected '1', got: {result}");
}

#[tokio::test]
async fn test_net_peer_count() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("net.peerCount").await;
    assert!(result.contains("25"), "expected 25, got: {result}");
}

// ── Error handling ─────────────────────────────────────────────

#[tokio::test]
async fn test_unknown_command() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("foo.bar").await;
    assert!(
        result.contains("unknown command"),
        "expected 'unknown command', got: {result}"
    );
}

#[tokio::test]
async fn test_missing_required_arg() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("eth.getBalance").await;
    assert!(
        result.contains("Error") && result.contains("requires at least"),
        "expected error about missing args, got: {result}"
    );
}

#[tokio::test]
async fn test_invalid_address() {
    let server = MockServer::start().await;
    let result = server
        .repl()
        .execute_command("eth.getBalance 0xinvalid")
        .await;
    assert!(
        result.contains("Error"),
        "expected error for invalid address, got: {result}"
    );
}

// ── Syntax variants ────────────────────────────────────────────

#[tokio::test]
async fn test_parenthesized_syntax() {
    let server = MockServer::start().await;
    let result = server
        .repl()
        .execute_command(
            r#"eth.getBalance("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045", "latest")"#,
        )
        .await;
    assert!(
        result.contains("1000000000000000000"),
        "expected 1 ETH in wei via parens, got: {result}"
    );
}

#[tokio::test]
async fn test_block_number_decimal_conversion() {
    let server = MockServer::start().await;
    let result = server
        .repl()
        .execute_command("eth.getBlockByNumber 100 false")
        .await;
    assert!(
        result.contains("null"),
        "expected null response, got: {result}"
    );
}

// ── Utility commands ───────────────────────────────────────────

#[tokio::test]
async fn test_utility_to_wei() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("toWei 1 ether").await;
    assert!(
        result.contains("1000000000000000000"),
        "expected 1e18, got: {result}"
    );
}

#[tokio::test]
async fn test_utility_from_wei() {
    let server = MockServer::start().await;
    let result = server
        .repl()
        .execute_command("fromWei 1000000000000000000 ether")
        .await;
    assert!(result.contains("1"), "expected 1, got: {result}");
}

#[tokio::test]
async fn test_utility_to_hex() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("toHex 255").await;
    assert!(result.contains("0xff"), "expected 0xff, got: {result}");
}

#[tokio::test]
async fn test_utility_from_hex() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("fromHex 0xff").await;
    assert!(result.contains("255"), "expected 255, got: {result}");
}

#[tokio::test]
async fn test_utility_is_address() {
    let server = MockServer::start().await;
    let result = server
        .repl()
        .execute_command("isAddress 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045")
        .await;
    assert!(result.contains("true"), "expected true, got: {result}");
}

// ── Built-in commands in non-interactive mode ──────────────────

#[tokio::test]
async fn test_builtin_in_non_interactive() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command(".help").await;
    assert!(
        result.contains("not available"),
        "expected 'not available', got: {result}"
    );
}

// ── Edge cases ─────────────────────────────────────────────────

#[tokio::test]
async fn test_empty_input() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("").await;
    assert!(result.is_empty(), "expected empty string, got: {result}");
}

#[tokio::test]
async fn test_whitespace_input() {
    let server = MockServer::start().await;
    let result = server.repl().execute_command("   ").await;
    assert!(result.is_empty(), "expected empty string, got: {result}");
}

// ── Sequential commands on same Repl ───────────────────────────

#[tokio::test]
async fn test_multiple_sequential_commands() {
    let server = MockServer::start().await;
    let repl = server.repl();

    let r1 = repl.execute_command("eth.blockNumber").await;
    assert!(r1.contains("68943"), "first command failed: {r1}");

    let r2 = repl.execute_command("eth.chainId").await;
    assert!(r2.contains("1"), "second command failed: {r2}");

    let r3 = repl
        .execute_command("eth.getBalance 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045")
        .await;
    assert!(
        r3.contains("1000000000000000000"),
        "third command failed: {r3}"
    );

    let r4 = repl.execute_command("toWei 1 ether").await;
    assert!(
        r4.contains("1000000000000000000"),
        "fourth command failed: {r4}"
    );
}
