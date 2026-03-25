//! One-shot HTTP listener for EIP-8025 proof callbacks.
//!
//! After `requestProofsV1`, the REPL spawns a temporary HTTP server that
//! receives the `GeneratedProof` POST from the proof coordinator, stores it
//! in the variable store as `$generatedProof`, and shuts down.

use crate::variables::VariableStore;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpSocket;

/// Spawn a background task that listens for exactly one HTTP POST containing
/// a `GeneratedProof` JSON body, stores it in the variable store, responds
/// with HTTP 200, and exits.
pub fn spawn_listener(port: u16, timeout_secs: u64, variables: VariableStore) {
    let timeout = Duration::from_secs(timeout_secs);
    tokio::spawn(async move {
        if let Err(e) = run_listener(port, timeout, variables).await {
            // Print to stdout so it's visible even with rustyline in raw mode.
            use std::io::Write;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            let _ = write!(out, "\r\x1b[2K");
            let _ = writeln!(out, "\x1b[1;31mProof callback error:\x1b[0m {e}");
            let _ = write!(out, "> ");
            let _ = out.flush();
        }
    });
}

async fn run_listener(
    port: u16,
    timeout: Duration,
    variables: VariableStore,
) -> Result<(), String> {
    let addr: std::net::SocketAddr = ([127, 0, 0, 1], port).into();

    let socket = TcpSocket::new_v4().map_err(|e| format!("Failed to create socket: {e}"))?;
    socket
        .set_reuseaddr(true)
        .map_err(|e| format!("Failed to set SO_REUSEADDR: {e}"))?;
    socket
        .bind(addr)
        .map_err(|e| format!("Failed to bind on port {port}: {e}"))?;
    let listener = socket
        .listen(1)
        .map_err(|e| format!("Failed to listen on port {port}: {e}"))?;

    // Wait for the callback connection with a timeout.
    let (mut stream, _peer) = tokio::time::timeout(timeout, listener.accept())
        .await
        .map_err(|_| {
            format!(
                "Timed out after {}s waiting for proof callback on port {port}.\n\
                 Check that the node was started with: --proof.callback-url http://127.0.0.1:{port}",
                timeout.as_secs()
            )
        })?
        .map_err(|e| format!("Accept failed: {e}"))?;

    // Read the full HTTP request into a buffer.
    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 4096];
    let body = loop {
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("Read failed: {e}"))?;
        if n == 0 {
            return Err("Connection closed before receiving complete request".to_string());
        }
        buf.extend_from_slice(&tmp[..n]);

        // Look for end of HTTP headers.
        let Some(header_end) = find_header_end(&buf) else {
            continue;
        };

        let headers = std::str::from_utf8(&buf[..header_end]).map_err(|e| e.to_string())?;
        let content_length = parse_content_length(headers).unwrap_or(0);
        let body_start = header_end + 4; // skip \r\n\r\n

        // Read remaining body bytes if needed.
        while buf.len() < body_start + content_length {
            let n = stream
                .read(&mut tmp)
                .await
                .map_err(|e| format!("Read failed: {e}"))?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }

        let end = body_start + content_length;
        if buf.len() < end {
            return Err(format!(
                "Incomplete body: expected {} bytes, got {}",
                content_length,
                buf.len().saturating_sub(body_start)
            ));
        }
        break buf[body_start..end].to_vec();
    };

    // Parse the JSON body as a GeneratedProof.
    let proof: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("Failed to parse proof JSON: {e}"))?;

    // Store in variable store.
    variables.insert("generatedProof".to_string(), proof);

    // Send HTTP 200 OK.
    let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;

    // Clear the readline prompt sitting on the current line, print the
    // notification, and restore a visual prompt so the REPL looks responsive.
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = write!(out, "\r\x1b[2K"); // erase the bare "> " line
    let _ = writeln!(
        out,
        "\x1b[1;32mProof received via callback!\x1b[0m Stored in \x1b[1m$generatedProof\x1b[0m"
    );
    let _ = writeln!(
        out,
        "  Verify with: engine.verifyExecutionProofV1 $generatedProof.executionProof"
    );
    let _ = write!(out, "> "); // restore visual prompt
    let _ = out.flush();

    Ok(())
}

/// Find the position of `\r\n\r\n` in the buffer (end of HTTP headers).
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Extract `Content-Length` value from HTTP headers.
fn parse_content_length(headers: &str) -> Option<usize> {
    for line in headers.lines() {
        if line.to_ascii_lowercase().starts_with("content-length:") {
            return line.split(':').nth(1)?.trim().parse().ok();
        }
    }
    None
}
