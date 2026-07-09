use ethrex_common::H256;
use ethrex_storage::hash_address;
use serde_json::{Value, json};

use super::helpers::{rpc_call, rpc_call_expect_err, setup_single_transfer_block};

async fn call_range(
    store: &ethrex_storage::Store,
    block_arg: Value,
    start: H256,
    max: u64,
) -> Value {
    rpc_call(
        store,
        "debug_accountRange",
        vec![
            block_arg,
            json!(0),
            json!(format!("{start:#x}")),
            json!(max),
        ],
    )
    .await
}

#[tokio::test]
async fn account_range_returns_accounts_by_hash() {
    let env = setup_single_transfer_block().await;
    let block_hash = env.block.hash();

    let result = rpc_call(
        &env.store,
        "debug_accountRange",
        vec![
            json!(format!("{block_hash:#x}")),
            json!(0),
            json!(format!("{:#x}", H256::zero())),
            json!(1_000_u64),
        ],
    )
    .await;

    let obj = result.as_object().expect("response should be an object");
    let accounts = obj["accounts"]
        .as_object()
        .expect("accounts should be an object");
    assert!(!accounts.is_empty(), "should return accounts from state");

    // Sender appears with its hashed address as key, and `key: null` per type doc.
    let sender_hash = format!("{:#x}", H256::from_slice(&hash_address(&env.sender)));
    let entry = accounts
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&sender_hash))
        .map(|(_, v)| v)
        .expect("sender should appear in account range");
    assert!(entry["key"].is_null(), "preimage is not stored");
    assert!(entry["balance"].is_string());
    assert!(entry["nonce"].is_number());
}

#[tokio::test]
async fn account_range_by_block_number() {
    let env = setup_single_transfer_block().await;

    let result = rpc_call(
        &env.store,
        "debug_accountRange",
        vec![
            json!(format!("{:#x}", env.block.header.number)),
            json!(0),
            json!(format!("{:#x}", H256::zero())),
            json!(1_000_u64),
        ],
    )
    .await;

    let accounts = result["accounts"]
        .as_object()
        .expect("accounts should be an object");
    assert!(!accounts.is_empty(), "block-number form should resolve");
}

#[tokio::test]
async fn account_range_by_latest_tag() {
    let env = setup_single_transfer_block().await;

    let result = rpc_call(
        &env.store,
        "debug_accountRange",
        vec![
            json!("latest"),
            json!(0),
            json!(format!("{:#x}", H256::zero())),
            json!(1_000_u64),
        ],
    )
    .await;

    let accounts = result["accounts"]
        .as_object()
        .expect("accounts should be an object");
    assert!(!accounts.is_empty(), "`latest` tag should resolve");
}

#[tokio::test]
async fn account_range_completes_with_zero_next() {
    let env = setup_single_transfer_block().await;
    let block_hash = env.block.hash();

    let result = rpc_call(
        &env.store,
        "debug_accountRange",
        vec![
            json!(format!("{block_hash:#x}")),
            json!(0),
            json!(format!("{:#x}", H256::zero())),
            // Far more than any genesis state — must terminate.
            json!(1_000_000_u64),
        ],
    )
    .await;

    let next = result["next"].as_str().expect("next must be a string");
    assert_eq!(
        next,
        format!("{:#x}", H256::zero()),
        "complete iteration must report all-zero next"
    );
}

#[tokio::test]
async fn account_range_paginates_via_next() {
    let env = setup_single_transfer_block().await;
    let block_hash = env.block.hash();

    let first = call_range(
        &env.store,
        json!(format!("{block_hash:#x}")),
        H256::zero(),
        1,
    )
    .await;
    let first_accounts = first["accounts"]
        .as_object()
        .expect("accounts should be an object");
    assert_eq!(first_accounts.len(), 1, "max=1 yields exactly one entry");
    let next_str = first["next"].as_str().expect("next should be a string");
    assert_ne!(
        next_str,
        format!("{:#x}", H256::zero()),
        "truncated page must have non-zero next"
    );
    let next_hash: H256 = next_str.parse().unwrap();
    let first_key = first_accounts.keys().next().unwrap().clone();

    // Continue from the cursor.
    let second = call_range(
        &env.store,
        json!(format!("{block_hash:#x}")),
        next_hash,
        1_000_000,
    )
    .await;
    let second_accounts = second["accounts"]
        .as_object()
        .expect("accounts should be an object");
    assert!(
        !second_accounts.is_empty(),
        "continuation page must return remaining accounts"
    );
    assert!(
        !second_accounts.contains_key(&first_key),
        "continuation must not repeat the first page's account"
    );
    assert_eq!(
        second["next"].as_str().unwrap(),
        format!("{:#x}", H256::zero()),
        "second page should complete the iteration"
    );
}

#[tokio::test]
async fn account_range_invalid_block_hash_errors() {
    let env = setup_single_transfer_block().await;

    let err = rpc_call_expect_err(
        &env.store,
        "debug_accountRange",
        vec![
            json!(format!("{:#x}", H256::from_low_u64_be(0xdeadbeef))),
            json!(0),
            json!(format!("{:#x}", H256::zero())),
            json!(1_u64),
        ],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Block not found"),
        "expected block-not-found error, got: {msg}"
    );
}

#[tokio::test]
async fn account_range_unknown_block_number_errors() {
    let env = setup_single_transfer_block().await;

    let err = rpc_call_expect_err(
        &env.store,
        "debug_accountRange",
        vec![
            json!(format!("{:#x}", env.block.header.number + 1_000)),
            json!(0),
            json!(format!("{:#x}", H256::zero())),
            json!(1_u64),
        ],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Block not found"),
        "expected block-not-found error, got: {msg}"
    );
}
