use ethrex_common::H256;
use serde_json::json;

use super::helpers::{
    rpc_call, rpc_call_expect_err, setup_multi_transfer_block, setup_single_transfer_block,
};

fn parse_roots(arr: &[serde_json::Value]) -> Vec<H256> {
    arr.iter()
        .map(|v| v.as_str().unwrap().parse().unwrap())
        .collect()
}

#[tokio::test]
async fn intermediate_roots_single_tx_matches_block_state_root() {
    // For a block with one tx and empty withdrawals, the only intermediate
    // root (after that single tx) must equal `block.header.state_root` — this
    // is the strong correctness check that distinguishes a real
    // implementation from one that just returns plausible-looking hashes.
    let env = setup_single_transfer_block().await;
    let block_hash = env.block.hash();
    let expected_state_root = env.block.header.state_root;

    let result = rpc_call(
        &env.store,
        "debug_intermediateRoots",
        vec![json!(format!("{block_hash:#x}"))],
    )
    .await;

    let arr = result.as_array().expect("response should be an array");
    let roots = parse_roots(arr);
    assert_eq!(roots.len(), 1, "one tx → one intermediate root");
    assert_eq!(
        roots[0], expected_state_root,
        "intermediate root after the only tx must equal the block's state_root \
         (empty withdrawals, so no post-tx changes)"
    );
}

#[tokio::test]
async fn intermediate_roots_multi_tx_progress_and_match_final() {
    // Block with three transfers from the same sender. The PR's algorithm
    // emits the *cumulative* state root after each tx (parent + system_calls
    // + txs[0..=i]). Sequential txs from the same sender mutate the sender's
    // nonce and balance every time, so each intermediate root must differ
    // from the previous one. The last root must equal `block.state_root`.
    let env = setup_multi_transfer_block(3).await;
    let block_hash = env.block.hash();
    let expected_final_root = env.block.header.state_root;

    let result = rpc_call(
        &env.store,
        "debug_intermediateRoots",
        vec![json!(format!("{block_hash:#x}"))],
    )
    .await;

    let arr = result.as_array().expect("response should be an array");
    let roots = parse_roots(arr);
    assert_eq!(roots.len(), 3, "three txs → three intermediate roots");
    assert_ne!(
        roots[0], roots[1],
        "tx[0] and tx[1] must yield different roots"
    );
    assert_ne!(
        roots[1], roots[2],
        "tx[1] and tx[2] must yield different roots"
    );
    assert_eq!(
        roots[2], expected_final_root,
        "final intermediate root must equal the block's state_root"
    );
}

#[tokio::test]
async fn intermediate_roots_accepts_config_object() {
    let env = setup_single_transfer_block().await;
    let block_hash = env.block.hash();

    let result = rpc_call(
        &env.store,
        "debug_intermediateRoots",
        vec![
            json!(format!("{block_hash:#x}")),
            json!({"timeout": "10s", "reexec": 256}),
        ],
    )
    .await;

    let arr = result.as_array().expect("response should be an array");
    assert_eq!(arr.len(), 1);
}

#[tokio::test]
async fn intermediate_roots_unknown_block_errors() {
    let env = setup_single_transfer_block().await;

    let err = rpc_call_expect_err(
        &env.store,
        "debug_intermediateRoots",
        vec![json!(format!("{:#x}", H256::from_low_u64_be(0xdeadbeef)))],
    )
    .await;
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Block not found"),
        "expected block-not-found error, got: {msg}"
    );
}
