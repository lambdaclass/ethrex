lazy_static::lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::new();
}

use std::collections::HashMap;

use ethrex_common::{types::BlockNumber, Bytes, H256, U256};
use ethrex_rpc::authentication::generate_jwt_token;
use ethrex_rpc::clients::auth::RpcResponse;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use ethrex_common::{serde_utils, Address};


#[derive(Deserialize, Debug)]
struct Dump {
    #[serde(rename = "root")]
    state_root: H256,
    accounts: HashMap<Address, DumpAccount>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DumpAccount {
    balance: U256,
    nonce: u64,
    #[serde(rename = "root")]
    storage_root: H256,
    code_hash: H256,
    #[serde(default, with = "serde_utils::bytes")]
    code: Bytes,
    address: Address,
    #[serde(rename = "key")]
    hashed_address: H256,
}

pub async fn archive_sync_2(archive_ipc_path: &str, block_number: BlockNumber) -> eyre::Result<()> {
    let mut stream = UnixStream::connect(archive_ipc_path).await?;

    let request = &json!({
    "id": 1,
    "jsonrpc": "2.0",
    "method": "debug_dumpBlock",
    "params": [format!("{:#x}", block_number)]
    });
    let response = send_ipc_json_request(&mut stream, request).await?;
    let dump: Dump = serde_json::from_value(response)?;
    dbg!(&dump);

    Ok(())
}



async fn send_ipc_json_request(stream: &mut UnixStream, request: &Value) -> eyre::Result<Value>  {
    stream.write_all(request.to_string().as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    let mut response = Vec::new();
    while stream.read_buf(&mut response).await? != 0 {
        if response.ends_with(b"\n") {
            break;
        }
    }
    let response: RpcResponse = serde_json::from_slice(&response)?;
    match response {
        RpcResponse::Success(success_res) => Ok(success_res.result),
        RpcResponse::Error(error_res) => Err(eyre::ErrReport::msg(error_res.error.message)),
    }
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
