use crate::rpc::*;

use bytes::Bytes;
use ethrex_core::{
    types::{AccountState, Block, EMPTY_KECCACK_HASH},
    Address, U256,
};
use ethrex_rlp::decode::RLPDecode;

use serde::Deserialize;
use serde_json::json;

pub async fn get_block(rpc_url: &str, block_number: usize) -> Result<Block, String> {
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
        .and_then(get_result)
        .and_then(decode_hex)
        .and_then(|encoded_block| {
            Block::decode_unfinished(&encoded_block)
                .map_err(|err| err.to_string())
                .map(|decoded| decoded.0)
        })
}

pub async fn get_account(
    rpc_url: &str,
    block_number: usize,
    address: &Address,
    storage_keys: &[U256],
) -> Result<Account, String> {
    let client = reqwest::Client::new();

    let block_number_str = format!("0x{block_number:x}");
    let address_str = format!("0x{address:x}");
    let storage_keys = storage_keys
        .iter()
        .map(|key| format!("0x{key:x}"))
        .collect::<Vec<String>>();

    let request = &json!(
           {
               "id": 1,
               "jsonrpc": "2.0",
               "method": "eth_getProof",
               "params":[address_str, storage_keys, block_number_str]
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
        storage_proof: Vec<StorageProof>,
        account_proof: Vec<String>,
    }

    #[derive(Deserialize)]
    struct StorageProof {
        key: String,
        value: String,
        proof: Vec<String>,
    }

    let AccountProof {
        balance,
        code_hash,
        nonce,
        storage_hash,
        storage_proof,
        account_proof,
    } = response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| err.to_string())
        .and_then(get_result)?;

    let (storage, storage_proofs) = storage_proof
        .into_iter()
        .map(|proof| -> Result<_, String> {
            let key: U256 = proof
                .key
                .parse()
                .map_err(|_| "failed to parse storage key".to_string())?;
            let value: U256 = proof
                .value
                .parse()
                .map_err(|_| "failed to parse storage value".to_string())?;
            let proofs = proof
                .proof
                .into_iter()
                .map(decode_hex)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(((key, value), proofs))
        })
        .collect::<Result<(HashMap<_, _>, Vec<_>), _>>()?;

    let account_state = AccountState {
        nonce: u64::from_str_radix(nonce.trim_start_matches("0x"), 16)
            .map_err(|_| "failed to parse nonce".to_string())?,
        balance: balance
            .parse()
            .map_err(|_| "failed to parse balance".to_string())?,
        storage_root: storage_hash
            .parse()
            .map_err(|_| "failed to parse storage root".to_string())?,
        code_hash: code_hash
            .parse()
            .map_err(|_| "failed to parse code hash".to_string())?,
    };

    let code = if account_state.code_hash != *EMPTY_KECCACK_HASH {
        Some(get_code(rpc_url, block_number, address).await?)
    } else {
        None
    };

    let account_proof = account_proof
        .into_iter()
        .map(decode_hex)
        .collect::<Result<Vec<_>, String>>()?;

    Ok(Account {
        account_state,
        storage,
        account_proof,
        storage_proofs,
        code,
    })
}

async fn get_code(rpc_url: &str, block_number: usize, address: &Address) -> Result<Bytes, String> {
    let client = reqwest::Client::new();

    let block_number = format!("0x{block_number:x}");
    let address = format!("0x{address:x}");
    let request = &json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "eth_getCode",
        "params": [address, block_number]
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
        .and_then(get_result)
        .and_then(decode_hex)
        .map(Bytes::from_owner)
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
        get_block(RPC_URL, BLOCK_NUMBER).await.unwrap();
    }

    #[ignore = "needs to manually set rpc url in constant"]
    #[tokio::test]
    async fn get_account_works() {
        get_account(
            RPC_URL,
            BLOCK_NUMBER,
            &Address::from_slice(&hex::decode(VITALIK_ADDR).unwrap()),
            &[],
        )
        .await
        .unwrap();
    }
}
