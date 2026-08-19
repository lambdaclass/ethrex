//! `eth_estimateGas` must return the transaction's executable minimum, not an
//! upper bound near it.
//!
//! The search used to stop as soon as its bracket was within
//! `ESTIMATE_ERROR_RATIO` (1.5%) of the upper bound and then return that upper
//! bound, so every estimate came back up to ~1.5% high. Differential testing
//! against the other clients surfaced it on the EIP-2780 paths, where the
//! intrinsic cost is low enough for the relative error to be most visible: a
//! self-send whose minimum is 12000 was reported as 12156.

use ethrex_rpc::test_utils::{call_http, default_context_with_storage, setup_store};

/// A genesis-funded EOA, so the estimate is not capped by the sender's balance.
const RICH: &str = "0x3f1eae7d46d88f08fc2f8ed27fcb2ab183eb2d0e";

async fn estimate(call_object: &str) -> u64 {
    let context = default_context_with_storage(setup_store().await).await;
    let body = format!(
        r#"{{"jsonrpc":"2.0","method":"eth_estimateGas","params":[{call_object},"latest"],"id":1}}"#
    );
    let response = call_http(context, body).await;
    let hex = response["result"]
        .as_str()
        .unwrap_or_else(|| panic!("eth_estimateGas should succeed, got {response}"));
    u64::from_str_radix(hex.trim_start_matches("0x"), 16).expect("estimate is hex")
}

/// A value transfer between existing accounts costs exactly the 21000 intrinsic
/// on this genesis, with nothing to execute on top. The estimate must be that
/// number, not 21000 plus the search tolerance.
#[tokio::test]
async fn estimate_gas_returns_the_exact_minimum_for_a_transfer() {
    let got = estimate(&format!(
        r#"{{"from":"{RICH}","to":"{RICH}","value":"0x1"}}"#
    ))
    .await;
    assert_eq!(
        got, 21_000,
        "a plain transfer's estimate must be its executable minimum, got {got}"
    );
}

/// Same for a zero-value call: no execution, so the estimate is the bare
/// intrinsic cost.
#[tokio::test]
async fn estimate_gas_returns_the_exact_minimum_for_a_zero_value_call() {
    let got = estimate(&format!(r#"{{"from":"{RICH}","to":"{RICH}"}}"#)).await;
    assert_eq!(
        got, 21_000,
        "a zero-value call's estimate must be its executable minimum, got {got}"
    );
}
