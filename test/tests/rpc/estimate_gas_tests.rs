//! `eth_estimateGas` must return the transaction's executable minimum, not an
//! upper bound near it.
//!
//! The search used to stop as soon as its bracket was within
//! `ESTIMATE_ERROR_RATIO` (1.5%) of the upper bound and then return that upper
//! bound, so every estimate came back up to ~1.5% high. Differential testing
//! against the other clients surfaced it on the EIP-2780 paths, where the
//! intrinsic cost is low enough for the relative error to be most visible: a
//! self-send whose minimum is 12000 was reported as 12156.

use bytes::Bytes;
use ethrex_common::Address;
use ethrex_common::types::{Genesis, GenesisAccount};
use ethrex_rpc::test_utils::{TEST_GENESIS, call_http, default_context_with_storage, setup_store};
use ethrex_storage::{EngineType, Store};

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

/// `GAS; PUSH2 0x2710; GT; PUSH1 0x0a; JUMPI; STOP; <pad>; JUMPDEST; INVALID`
///
/// Succeeds while more than 10000 gas remains at entry and hits `INVALID`
/// otherwise, so it is a transaction whose *result* depends on the gas it is
/// given. Running it at exactly the gas an unconstrained run consumed fails,
/// which is what forces the estimate off the fast path and into the search.
const GAS_OBSERVER_CODE: [u8; 12] = [
    0x5a, 0x61, 0x27, 0x10, 0x11, 0x60, 0x0a, 0x57, 0x00, 0x00, 0x5b, 0xfe,
];

const OBSERVER: Address = Address::repeat_byte(0x0b);

async fn store_with_gas_observer() -> Store {
    let mut genesis: Genesis = serde_json::from_str(TEST_GENESIS).expect("test genesis is valid");
    genesis.alloc.insert(
        OBSERVER,
        GenesisAccount {
            code: Bytes::from_static(&GAS_OBSERVER_CODE),
            storage: Default::default(),
            balance: Default::default(),
            nonce: 0,
        },
    );
    let mut store = Store::new("estimate-gas-observer", EngineType::InMemory)
        .expect("in-memory store should build");
    store
        .add_initial_state(genesis)
        .await
        .expect("genesis should load");
    store
}

/// The fast path only answers when re-running at the consumed gas succeeds. A
/// gas-observing callee fails there, so this exercises the binary search, which
/// stops within `ESTIMATE_ERROR_RATIO` of its upper bound exactly as geth's does.
/// The contract that search must honour is one-sided: the estimate has to be
/// executable, and may sit above the true minimum but never below it.
#[tokio::test]
async fn estimate_gas_for_a_gas_observing_call_is_executable_and_within_the_tolerance() {
    /// Mirror of the crate-private constant; the `eth` module is not exported.
    const ESTIMATE_ERROR_RATIO: f64 = 0.015;

    let context = default_context_with_storage(store_with_gas_observer().await).await;
    let call = format!(r#"{{"from":"{RICH}","to":"{OBSERVER:#x}"}}"#);

    let body = format!(
        r#"{{"jsonrpc":"2.0","method":"eth_estimateGas","params":[{call},"latest"],"id":1}}"#
    );
    let response = call_http(context.clone(), body).await;
    let hex = response["result"]
        .as_str()
        .unwrap_or_else(|| panic!("eth_estimateGas should succeed, got {response}"));
    let estimate = u64::from_str_radix(hex.trim_start_matches("0x"), 16).expect("hex");

    let succeeds = |gas: u64| {
        let context = context.clone();
        async move {
            let probe = format!(
                r#"{{"jsonrpc":"2.0","method":"eth_call","params":[{{"from":"{RICH}","to":"{OBSERVER:#x}","gas":"{gas:#x}"}},"latest"],"id":1}}"#
            );
            call_http(context, probe).await.get("result").is_some()
        }
    };

    assert!(
        succeeds(estimate).await,
        "the estimate must be executable, {estimate} was not"
    );

    // Find the true minimum by bisection, then check the estimate is at or above it
    // and no further above than the tolerance allows.
    let (mut lo, mut hi) = (21_000_u64, estimate);
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if succeeds(mid).await {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let minimum = hi;
    assert!(
        estimate >= minimum,
        "the estimate must never sit below the minimum: {estimate} < {minimum}"
    );
    let over = (estimate - minimum) as f64 / estimate as f64;
    assert!(
        over < ESTIMATE_ERROR_RATIO,
        "overestimate {over:.4} (est {estimate}, min {minimum}) must stay within the tolerance"
    );
}
