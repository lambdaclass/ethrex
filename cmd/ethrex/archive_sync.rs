lazy_static::lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::new();
}

use ethrex_common::{types::BlockNumber, Bytes};
use ethrex_rpc::authentication::generate_jwt_token;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub async fn archive_sync_2(archive_ipc_path: &str, block_number: BlockNumber) -> eyre::Result<()> {
    let mut stream = UnixStream::connect(archive_ipc_path).await?;

    let request = &json!({
    "id": 1,
    "jsonrpc": "2.0",
    "method": "debug_dumpBlock",
    "params": [block_number]
    });
    stream.write_all(request.to_string().as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let res = serde_json::from_slice(&response)?;
    dbg!(&res);
    Ok(())
}

pub async fn archive_sync(
    archive_node_url: &str,
    archive_node_jwt: &Bytes,
    block_number: BlockNumber,
) -> eyre::Result<()> {
    let token = generate_jwt_token(archive_node_jwt)?;
    let request = &json!({
    "id": 1,
    "jsonrpc": "2.0",
    "method": "debug_dumpBlock",
    "params": [block_number]
    });

    let response = CLIENT
        .post(archive_node_url)
        .bearer_auth(token)
        .json(request)
        .send()
        .await?;

    let res = response.json::<serde_json::Value>().await?;
    dbg!(&res);
    Ok(())
}
