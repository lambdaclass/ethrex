use ethrex_common::types::{
    block_execution_witness::{ExecutionWitnessResult, ExecutionWitnessError}, Block, ChainConfig};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rpc::admin::NodeInfo;
use serde::de::DeserializeOwned;
use serde_json::json;

use lazy_static::lazy_static;

lazy_static! {
    static ref CLIENT: reqwest::Client = reqwest::Client::new();
}

pub async fn get_latest_block_number(rpc_url: &str) -> eyre::Result<usize> {
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_blockNumber",
        "params": []
    });

    let response = CLIENT.post(rpc_url).json(request).send().await?;

    let res = response.json::<serde_json::Value>().await?;
    let mut bytes = decode_hex(get_result(res)?)?;
    bytes.reverse();
    bytes.resize(8, 0);
    let latest_bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| eyre::Error::msg("decode error".to_string()))?;

    Ok(usize::from_le_bytes(latest_bytes))
}

pub async fn get_block(rpc_url: &str, block_number: usize) -> eyre::Result<Block> {
    let block_number = format!("0x{block_number:x}");
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "debug_getRawBlock",
        "params": [block_number]
    });

    let response = CLIENT.post(rpc_url).json(request).send().await?;

    let res = response.json::<serde_json::Value>().await?;
    let encoded_block = decode_hex(get_result(res)?)?;
    let block = Block::decode_unfinished(&encoded_block)?;
    Ok(block.0)
}

pub async fn get_node_info(rpc_url: &str) -> eyre::Result<NodeInfo> {
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "admin_nodeInfo",
        "params": []
    });

    let response = CLIENT.post(rpc_url).json(request).send().await?;
    let res = response.json::<serde_json::Value>().await?;
    get_result(res)
}

pub async fn get_chain_config(rpc_url: &str) -> eyre::Result<ChainConfig> {
    let node = get_node_info(rpc_url).await?;
    node.protocols.eth.ok_or(ExecutionWitnessError::MissingChainConfig.into())
}

pub async fn get_witness(
    rpc_url: &str,
    block_number: usize,
) -> eyre::Result<ExecutionWitnessResult> {
    let block_number = format!("0x{block_number:x}");
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "debug_executionWitness",
        "params": [block_number]
    });

    let response = CLIENT.post(rpc_url).json(request).send().await?;
    let res = response.json::<serde_json::Value>().await?;
    let mut witness: ExecutionWitnessResult = get_result(res)?;
    witness.chain_config = get_chain_config(rpc_url).await?;
    return Ok(witness);
}

pub async fn get_witness_range(
    rpc_url: &str,
    from: usize,
    to: usize,
) -> eyre::Result<ExecutionWitnessResult> {
    let from = format!("0x{from:x}");
    let to = format!("0x{to:x}");
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "debug_executionWitness",
        "params": [from, to]
    });

    let response = CLIENT.post(rpc_url).json(request).send().await?;
    let res = response.json::<serde_json::Value>().await?;
    let mut witness: ExecutionWitnessResult = get_result(res)?;
    witness.chain_config = get_chain_config(rpc_url).await?;
    return Ok(witness);
}

fn get_result<T: DeserializeOwned>(response: serde_json::Value) -> eyre::Result<T> {
    match response.get("result") {
        Some(result) => Ok(serde_json::from_value(result.clone())?),
        None => Err(eyre::Error::msg(format!(
            "result not found, response is: {response}"
        ))),
    }
}

fn decode_hex(hex: String) -> eyre::Result<Vec<u8>> {
    let mut trimmed = hex.trim_start_matches("0x").to_string();
    if trimmed.len() % 2 != 0 {
        trimmed = "0".to_string() + &trimmed;
    }
    Ok(hex::decode(trimmed)?)
}

pub async fn get_tx_block(tx: &str, rpc_url: &str) -> eyre::Result<usize> {
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_getTransactionByHash",
        "params": [tx]
    });

    let response = CLIENT.post(rpc_url).json(request).send().await?;

    let res = response.json::<serde_json::Value>().await?;
    let res = res.get("result").ok_or(eyre::Error::msg("result key"))?;
    let block_number = res
        .get("blockNumber")
        .and_then(|v| v.as_str())
        .ok_or(eyre::Error::msg("bad blockNumber key"))?;
    let block_number = usize::from_str_radix(block_number.trim_start_matches("0x"), 16)?;
    Ok(block_number)
}
