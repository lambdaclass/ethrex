use ethrex_common::H256;
use ethrex_storage::hash_address;
use serde_json::{Value, json};

use super::helpers::{rpc_call, rpc_call_expect_err, setup_single_transfer_block};

fn parse_hashes(arr: &[Value]) -> Vec<H256> {
    arr.iter()
        .map(|v| v.as_str().unwrap().parse().unwrap())
        .collect()
}

#[tokio::test]
async fn get_modified_accounts_by_number_includes_sender() {
    let env = setup_single_transfer_block().await;
    let block_number = env.block.header.number;

    let result = rpc_call(
        &env.store,
        "debug_getModifiedAccountsByNumber",
        vec![
            json!(format!("{:#x}", block_number - 1)),
            json!(format!("{block_number:#x}")),
        ],
    )
    .await;

    let arr = result.as_array().expect("response should be an array");
    let hashes = parse_hashes(arr);
    let expected = H256::from_slice(&hash_address(&env.sender));
    assert!(
        hashes.contains(&expected),
        "diff must include sender's hashed address ({expected:?}); got: {hashes:?}"
    );
}

#[tokio::test]
async fn get_modified_accounts_by_hash_matches_by_number() {
    let env = setup_single_transfer_block().await;
    let genesis_hash = env.store.get_block_header(0).unwrap().unwrap().hash();
    let block_hash = env.block.hash();

    let by_hash = rpc_call(
        &env.store,
        "debug_getModifiedAccountsByHash",
        vec![
            json!(format!("{genesis_hash:#x}")),
            json!(format!("{block_hash:#x}")),
        ],
    )
    .await;
    let by_number = rpc_call(
        &env.store,
        "debug_getModifiedAccountsByNumber",
        vec![
            json!(format!("{:#x}", 0_u64)),
            json!(format!("{:#x}", env.block.header.number)),
        ],
    )
    .await;

    // Both variants must produce the same set of hashed addresses for the
    // same effective block range — order is implementation-defined, so compare
    // as sets.
    let mut a = parse_hashes(by_hash.as_array().unwrap());
    let mut b = parse_hashes(by_number.as_array().unwrap());
    a.sort();
    b.sort();
    assert_eq!(a, b);
    assert!(!a.is_empty(), "transfer should modify at least one account");
}

#[tokio::test]
async fn get_modified_accounts_empty_range_returns_empty() {
    let env = setup_single_transfer_block().await;
    let n = env.block.header.number;

    let result = rpc_call(
        &env.store,
        "debug_getModifiedAccountsByNumber",
        vec![json!(format!("{n:#x}")), json!(format!("{n:#x}"))],
    )
    .await;

    let arr = result.as_array().expect("response should be an array");
    assert!(
        arr.is_empty(),
        "start == end should yield empty diff, got: {arr:?}"
    );
}

#[tokio::test]
async fn get_modified_accounts_start_after_end_errors() {
    let env = setup_single_transfer_block().await;
    let n = env.block.header.number;

    let err = rpc_call_expect_err(
        &env.store,
        "debug_getModifiedAccountsByNumber",
        vec![json!(format!("{n:#x}")), json!(format!("{:#x}", 0_u64))],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(msg.contains("older"), "expected ordering error, got: {msg}");
}

#[tokio::test]
async fn get_modified_accounts_by_hash_start_after_end_errors() {
    let env = setup_single_transfer_block().await;
    let genesis_hash = env.store.get_block_header(0).unwrap().unwrap().hash();
    let block_hash = env.block.hash();

    let err = rpc_call_expect_err(
        &env.store,
        "debug_getModifiedAccountsByHash",
        vec![
            json!(format!("{block_hash:#x}")),
            json!(format!("{genesis_hash:#x}")),
        ],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(msg.contains("older"), "expected ordering error, got: {msg}");
}

#[tokio::test]
async fn get_modified_accounts_unknown_block_errors() {
    let env = setup_single_transfer_block().await;
    let n = env.block.header.number;

    let err = rpc_call_expect_err(
        &env.store,
        "debug_getModifiedAccountsByNumber",
        vec![
            json!(format!("{:#x}", n + 1_000)),
            json!(format!("{:#x}", n + 2_000)),
        ],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("not found"),
        "expected block-not-found error, got: {msg}"
    );
}
