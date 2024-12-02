use ethrex_core::{
    types::{AccountState, Block},
    Address,
};
use ethrex_rlp::decode::RLPDecode;

use serde::Deserialize;
use serde_json::json;

pub async fn get_block(rpc_url: &str, block_number: &usize) -> Result<Block, String> {
    let client = reqwest::Client::new();

    let block_number = format!("0x{block_number:x}");
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "debug_getRawBlock",
        "params": [block_number]
    });

    let response = client
        .post(rpc_url)
        .json(request)
        .send()
        .await
        .map_err(|err| err.to_string())?;

    response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| err.to_string())
        .and_then(|json| {
            json.get("result")
                .cloned()
                .ok_or("failed to get result from response".to_string())
        })
        .and_then(|result| serde_json::from_value::<String>(result).map_err(|err| err.to_string()))
        .and_then(|hex_encoded_block| {
            hex::decode(hex_encoded_block.trim_start_matches("0x")).map_err(|err| err.to_string())
        })
        .and_then(|encoded_block| {
            Block::decode_unfinished(&encoded_block)
                .map_err(|err| err.to_string())
                .map(|decoded| decoded.0)
        })
}

pub async fn get_account(
    rpc_url: &str,
    block_number: &usize,
    address: &Address,
) -> Result<AccountState, String> {
    let client = reqwest::Client::new();

    let block_number = format!("0x{block_number:x}");
    let address = format!("0x{address:x}");

    let request = &json!(
           {
               "id": 1,
               "jsonrpc": "2.0",
               "method": "eth_getProof",
               "params":[address, [], block_number]
           }
    );
    let response = client
        .post(rpc_url)
        .json(request)
        .send()
        .await
        .map_err(|err| err.to_string())?;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AccountProof {
        balance: String,
        code_hash: String,
        nonce: String,
        storage_hash: String,
    }

    let account_proof: AccountProof = response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| err.to_string())
        .and_then(|json| {
            json.get("result")
                .cloned()
                .ok_or("failed to get result from response".to_string())
        })
        .and_then(|result| serde_json::from_value(result).map_err(|err| err.to_string()))?;

    Ok(AccountState {
        nonce: u64::from_str_radix(account_proof.nonce.trim_start_matches("0x"), 16)
            .map_err(|_| "failed to parse nonce".to_string())?,
        balance: account_proof
            .balance
            .parse()
            .map_err(|_| "failed to parse balance".to_string())?,
        storage_root: account_proof
            .storage_hash
            .parse()
            .map_err(|_| "failed to parse storage root".to_string())?,
        code_hash: account_proof
            .code_hash
            .parse()
            .map_err(|_| "failed to parse code hash".to_string())?,
    })
}

#[cfg(test)]
mod test {
    use ethrex_core::Address;

    use super::*;

    const BLOCK_NUMBER: usize = 21315830;
    const RPC_URL: &str = "<to-complete>";
    const VITALIK_ADDR: &str = "d8dA6BF26964aF9D7eEd9e03E53415D37aA96045";

    #[ignore = "needs to manually set rpc url in constant"]
    #[tokio::test]
    async fn get_block_works() {
        get_block(RPC_URL, &BLOCK_NUMBER).await.unwrap();
    }

    #[ignore = "needs to manually set rpc url in constant"]
    #[tokio::test]
    async fn get_account_works() {
        get_account(
            RPC_URL,
            &BLOCK_NUMBER,
            &Address::from_slice(&hex::decode(VITALIK_ADDR).unwrap()),
        )
        .await
        .unwrap();
    }
}
