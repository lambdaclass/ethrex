lazy_static::lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::new();
}


use ethrex_common::types::BlockNumber;
use serde_json::json;

pub async fn archive_sync(archive_node_url: &str, block_number: BlockNumber) -> eyre::Result<()>{

    let request = &json!({
    "id": 1,
    "jsonrpc": "2.0",
    "method": "debug_dumpBlock",
    "params": [block_number]
    });

    let response = CLIENT.post(archive_node_url).json(request).send().await?;

    let res = response.json::<serde_json::Value>().await?;
    dbg!(&res); 
    Ok(())
}
