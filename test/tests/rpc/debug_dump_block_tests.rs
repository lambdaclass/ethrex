use ethrex_common::{H256, U256};
use ethrex_storage::hash_address;
use serde_json::{Value, json};

use super::helpers::{rpc_call, rpc_call_expect_err, setup_single_transfer_block};

#[tokio::test]
async fn dump_block_returns_state_at_block() {
    let env = setup_single_transfer_block().await;

    let result = rpc_call(
        &env.store,
        "debug_dumpBlock",
        vec![json!(format!("{:#x}", env.block.header.number))],
    )
    .await;

    let obj = result.as_object().expect("response should be an object");
    let root = obj["root"].as_str().expect("root should be a string");
    assert_eq!(
        root.to_lowercase(),
        format!("{:#x}", env.block.header.state_root).to_lowercase()
    );
    assert!(
        obj.get("next").is_none() || obj["next"].is_null(),
        "small dev chain should not be truncated"
    );

    let accounts = obj["accounts"]
        .as_object()
        .expect("accounts should be an object");
    assert!(!accounts.is_empty(), "should have accounts in state dump");

    // Sender appears with the expected post-transfer state. Lookup is by
    // hashed-address since dumpBlock keys accounts on `keccak(address)`.
    let sender_key = format!("{:#x}", H256::from_slice(&hash_address(&env.sender)));
    let entry = accounts
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&sender_key))
        .map(|(_, v)| v)
        .expect("sender should appear in the dump");
    let nonce = entry["nonce"].as_u64().expect("nonce must be a number");
    assert_eq!(nonce, 1, "sender nonce after one tx should be 1");
    let balance_str = entry["balance"]
        .as_str()
        .expect("balance must be a hex string");
    let balance =
        U256::from_str_radix(balance_str.trim_start_matches("0x"), 16).expect("hex balance");
    assert!(
        balance < U256::from(10).pow(U256::from(20)),
        "sender balance should be reduced after the transfer"
    );
}

#[tokio::test]
async fn dump_block_unknown_block_errors() {
    let env = setup_single_transfer_block().await;

    let err = rpc_call_expect_err(
        &env.store,
        "debug_dumpBlock",
        vec![json!(format!("{:#x}", env.block.header.number + 1_000))],
    )
    .await;

    let msg = format!("{err:?}");
    assert!(
        msg.contains("Block not found") || msg.contains("Block header not found"),
        "expected block-not-found error, got: {msg}"
    );
}

#[tokio::test]
async fn dump_block_latest_tag_resolves() {
    let env = setup_single_transfer_block().await;

    let result = rpc_call(&env.store, "debug_dumpBlock", vec![json!("latest")]).await;

    let root = result["root"].as_str().expect("root should be a string");
    assert_eq!(
        root.to_lowercase(),
        format!("{:#x}", env.block.header.state_root).to_lowercase(),
        "`latest` should resolve to the head block's state root"
    );
}

#[tokio::test]
async fn dump_block_paginates_via_max_results_and_next() {
    let env = setup_single_transfer_block().await;

    let first: Value = rpc_call(
        &env.store,
        "debug_dumpBlock",
        vec![
            json!(format!("{:#x}", env.block.header.number)),
            json!({ "maxResults": 1_u64 }),
        ],
    )
    .await;

    let accounts = first["accounts"]
        .as_object()
        .expect("accounts should be an object");
    assert_eq!(accounts.len(), 1, "maxResults=1 should yield one account");
    let first_key = accounts.keys().next().unwrap().clone();
    let next = first["next"]
        .as_str()
        .expect("truncated response must include `next`")
        .to_owned();

    let second: Value = rpc_call(
        &env.store,
        "debug_dumpBlock",
        vec![
            json!(format!("{:#x}", env.block.header.number)),
            json!({ "start": next, "maxResults": 1_000_000_u64 }),
        ],
    )
    .await;

    let second_accounts = second["accounts"]
        .as_object()
        .expect("accounts should be an object");
    assert!(
        !second_accounts.is_empty(),
        "continuation page should return remaining accounts"
    );
    assert!(
        !second_accounts.contains_key(&first_key),
        "continuation page should not repeat the first page's account"
    );
    assert!(
        second.get("next").is_none() || second["next"].is_null(),
        "second page should complete the dump on a tiny dev chain"
    );
}
