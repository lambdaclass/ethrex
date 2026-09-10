//! End-to-end coverage for geth's State Override Set on `eth_call` and
//! `eth_estimateGas`.
//!
//! The branch's other override coverage is unit-level (parse tests plus the
//! `OverlaidVmDatabase` tests). These go through the full JSON-RPC HTTP dispatch so
//! the whole chain is exercised: parse -> `into_overrides` -> `new_overlaid_evm` ->
//! LEVM. Each asserts on an *executed* result rather than on the parsed request, so
//! an override that parses but never reaches the EVM fails the test.

use bytes::Bytes;
use ethrex_common::types::{Genesis, GenesisAccount};
use ethrex_common::{Address, U256};
use ethrex_rpc::test_utils::{call_http, default_context_with_storage, setup_store};
use ethrex_storage::{EngineType, Store};
use serde_json::{Value, json};
use std::str::FromStr;

/// Address the overrides install synthetic bytecode at.
const CALLEE: &str = "0x000000000000000000000000000000000000cafe";
/// Caller, funded through the override set so the tests don't depend on whatever
/// the genesis fixture happens to allocate.
const CALLER: &str = "0x000000000000000000000000000000000000beef";
const ONE_ETH: &str = "0xde0b6b3a7640000";

/// PUSH1 0x2a, PUSH1 0x00, MSTORE, PUSH1 0x20, PUSH1 0x00, RETURN — returns 42.
const RETURN_42: &str = "0x602a60005260206000f3";
/// SELFBALANCE, PUSH1 0x00, MSTORE, PUSH1 0x20, PUSH1 0x00, RETURN.
const RETURN_SELF_BALANCE: &str = "0x4760005260206000f3";
/// PUSH1 0x00, SLOAD, PUSH1 0x00, MSTORE, PUSH1 0x20, PUSH1 0x00, RETURN.
const RETURN_SLOT_ZERO: &str = "0x60005460005260206000f3";
/// PUSH1 0x01, PUSH1 0x00, SSTORE, STOP — a cold zero->nonzero SSTORE, so far more
/// than the 21000 a plain value transfer costs.
const STORE_ONE: &str = "0x600160005500";

const SLOT_ZERO: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

fn request(method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string()
}

/// The `result` string, or a panic naming the whole response (which carries the
/// JSON-RPC `error` object when a request was rejected).
fn expect_result(response: &Value) -> &str {
    response
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or_else(|| panic!("expected a string result, got: {response}"))
}

/// Strip the `0x` and leading zeros so a 32-byte word can be compared to the small
/// value it encodes.
fn word_value(result: &str) -> &str {
    let trimmed = result.trim_start_matches("0x").trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

/// A `code` override must make `eth_call` execute that code at an address which
/// holds none on chain.
#[tokio::test]
async fn call_executes_overridden_code() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_call",
        json!([
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                CALLEE: { "code": RETURN_42 },
                CALLER: { "balance": ONE_ETH }
            }
        ]),
    );

    let response = call_http(context, body).await;
    assert_eq!(word_value(expect_result(&response)), "2a");
}

/// Without the override the same call runs against an empty account and returns no
/// data. Control for the test above: proves the assertion tracks the override rather
/// than something the genesis fixture already provides.
#[tokio::test]
async fn call_without_overrides_hits_the_empty_account() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_call",
        json!([{ "from": CALLER, "to": CALLEE, "data": "0x" }, "latest"]),
    );

    let response = call_http(context, body).await;
    assert_eq!(expect_result(&response), "0x");
}

/// A `balance` override must be observable from inside the EVM, not just at the
/// accounting layer: SELFBALANCE reads it through `get_account_state`.
#[tokio::test]
async fn call_observes_overridden_balance() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_call",
        json!([
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                CALLEE: { "code": RETURN_SELF_BALANCE, "balance": "0x1234" },
                CALLER: { "balance": ONE_ETH }
            }
        ]),
    );

    let response = call_http(context, body).await;
    assert_eq!(word_value(expect_result(&response)), "1234");
}

/// A `state` override must be visible to SLOAD.
#[tokio::test]
async fn call_observes_overridden_storage() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_call",
        json!([
            { "from": CALLER, "to": CALLEE, "data": "0x" },
            "latest",
            {
                CALLEE: { "code": RETURN_SLOT_ZERO, "state": { SLOT_ZERO: "0x5678" } },
                CALLER: { "balance": ONE_ETH }
            }
        ]),
    );

    let response = call_http(context, body).await;
    assert_eq!(word_value(expect_result(&response)), "5678");
}

/// `eth_estimateGas` short-circuits a plain value transfer to 21000 without
/// executing anything. That shortcut is wrong when a `code` override puts code at
/// the destination, so the handler skips it whenever overrides are present — this
/// guards that skip.
#[tokio::test]
async fn estimate_gas_with_code_override_skips_the_value_transfer_short_circuit() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    // Control: no overrides, empty destination -> the 21000 short-circuit.
    let plain = request(
        "eth_estimateGas",
        json!([{ "from": CALLER, "to": CALLEE, "value": "0x0" }, "latest"]),
    );
    let response = call_http(context.clone(), plain).await;
    assert_eq!(
        expect_result(&response),
        "0x5208",
        "expected the plain-transfer short-circuit (21000)"
    );

    // Same call, but the destination now holds a cold SSTORE.
    let overridden = request(
        "eth_estimateGas",
        json!([
            { "from": CALLER, "to": CALLEE, "value": "0x0" },
            "latest",
            {
                CALLEE: { "code": STORE_ONE },
                CALLER: { "balance": ONE_ETH }
            }
        ]),
    );
    let response = call_http(context, overridden).await;
    let estimate = expect_result(&response);
    let gas = u64::from_str_radix(estimate.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("estimate {estimate} is not hex: {e}"));
    assert!(
        gas > 21_000,
        "short-circuit was not skipped: estimate {estimate} does not cover the SSTORE"
    );
}

/// GAS, PUSH3 0x0186a0, GT, PUSH1 0x0a, JUMPI, STOP, JUMPDEST, PUSH1 0, PUSH1 0, REVERT
/// — reverts unless at least ~100000 gas is still available when it runs. Its *consumed*
/// gas is tiny (~21 over intrinsic), so a gas limit derived from consumption alone is far
/// too low for it to succeed.
const NEEDS_100K_AVAILABLE: &str = "0x5a620186a011600a57005b60006000fd";

/// PUSH1 1, NUMBER, SUB, BLOCKHASH, PUSH1 0, MSTORE, PUSH1 0x20, PUSH1 0, RETURN
/// — returns BLOCKHASH(NUMBER-1), which is inside the 256-block window but past the real
/// chain tip once `number` is overridden far ahead.
const RETURN_PARENT_BLOCKHASH: &str = "0x600143034060005260206000f3";

/// `eth_estimateGas` re-runs the transaction at the gas it consumed and, if that
/// succeeds, returns it without bisecting. That re-run must carry the override sets: with
/// them dropped, a gas-observing callee installed by a `code` override is estimated from
/// a run against real state where the address holds no code at all.
#[tokio::test]
async fn estimate_gas_exact_rerun_honors_the_override_sets() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_estimateGas",
        json!([
            { "from": CALLER, "to": CALLEE, "value": "0x0" },
            "latest",
            {
                CALLEE: { "code": NEEDS_100K_AVAILABLE },
                CALLER: { "balance": ONE_ETH }
            }
        ]),
    );

    let response = call_http(context, body).await;
    let estimate = expect_result(&response);
    let gas = u64::from_str_radix(estimate.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("estimate {estimate} is not hex: {e}"));
    assert!(
        gas > 100_000,
        "estimate {estimate} would revert: the callee needs ~100000 gas still available, \
         so the exact re-run must have been made without the code override"
    );
}

/// The balance-derived gas ceiling must be computed from the *overridden* balance. Geth
/// applies the override set before capping. Funding an otherwise empty sender is the most
/// common use of a `balance` override, and it only shows up once a fee field is set,
/// because a zero fee cap skips the cap entirely.
#[tokio::test]
async fn estimate_gas_caps_against_the_overridden_balance() {
    let storage = setup_store().await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_estimateGas",
        json!([
            {
                "from": CALLER,
                "to": CALLEE,
                "value": "0x0",
                "maxFeePerGas": "0x3b9aca00"
            },
            "latest",
            { CALLER: { "balance": ONE_ETH } }
        ]),
    );

    let response = call_http(context, body).await;
    assert_eq!(
        word_value(expect_result(&response)),
        "5208",
        "the override funds the sender, so the cap must not collapse: {response}"
    );
}

/// The "BLOCKHASH past the real tip reads zero" rule belongs to the synthetic block, not
/// to the State Override Set. A Block Override Set that moves `number` past the tip must
/// get it too, otherwise the same request errors or succeeds depending on whether an
/// unrelated state override happens to be present.
///
/// The callee's code comes from the genesis `alloc` rather than a `code` override,
/// because a code override would itself build the overlay and so hide the bug.
#[tokio::test]
async fn call_with_only_block_overrides_clamps_blockhash_past_the_tip() {
    let storage = store_with_deployed_code(CALLEE, RETURN_PARENT_BLOCKHASH).await;
    let context = default_context_with_storage(storage).await;

    let body = request(
        "eth_call",
        json!([
            { "from": FUNDED_SENDER, "to": CALLEE, "data": "0x" },
            "latest",
            null,
            { "number": "0x3e8" }
        ]),
    );

    let response = call_http(context, body).await;
    assert_eq!(
        word_value(expect_result(&response)),
        "0",
        "BLOCKHASH past the real tip must read zero even with no state override: {response}"
    );
}

/// `eth_estimateGas` derives the fork and the search ceiling from the block header, and
/// must take them from the *effective* header: a `gasLimit` block override that lowers
/// the ceiling below the transaction's intrinsic gas has to make the estimate fail, not
/// be silently ignored in favour of the real block's limit.
///
/// Uses a Prague fixture because `get_max_allowed_gas_limit` returns the flat EIP-7825
/// cap on Osaka (which the default fixture is), where `block_gas_limit` is not consulted
/// at all and the override would be unobservable.
#[tokio::test]
async fn estimate_gas_ceiling_follows_the_block_gas_limit_override() {
    let storage = prague_store().await;
    let context = default_context_with_storage(storage).await;

    // Control: the real block limit (25M) is ample, so a plain transfer estimates 21000.
    let plain = request(
        "eth_estimateGas",
        json!([{ "from": FUNDED_SENDER, "to": CALLEE, "value": "0x0" }, "latest"]),
    );
    let response = call_http(context.clone(), plain).await;
    assert_eq!(expect_result(&response), "0x5208", "control: {response}");

    // Same transfer under a gasLimit override below the 21000 intrinsic cost.
    let capped = request(
        "eth_estimateGas",
        json!([
            { "from": FUNDED_SENDER, "to": CALLEE, "value": "0x0" },
            "latest",
            null,
            { "gasLimit": "0x5000" }
        ]),
    );
    let response = call_http(context, capped).await;
    assert!(
        response.get("error").is_some(),
        "a 0x5000 gasLimit override cannot cover 21000 intrinsic gas, so the estimate \
         must fail rather than fall back on the real block's limit: {response}"
    );
}

/// A genesis-funded EOA from `fixtures/genesis/l1.json`, so tests that must avoid a
/// state override still have a sender that can pay.
const FUNDED_SENDER: &str = "0x00000a8d3f37af8def18832962ee008d8dca4f7b";

fn l1_genesis() -> Genesis {
    serde_json::from_str(include_str!("../../../fixtures/genesis/l1.json"))
        .expect("l1 test genesis is invalid")
}

async fn store_from(genesis: Genesis) -> Store {
    let mut store = Store::new("test-store", EngineType::InMemory).expect("in-memory store");
    store.add_initial_state(genesis).await.unwrap();
    store
}

/// The l1 test genesis with `code` deployed at `address`, for tests that need executable
/// code without a `code` override.
async fn store_with_deployed_code(address: &str, code: &str) -> Store {
    let mut genesis = l1_genesis();
    genesis.alloc.insert(
        Address::from_str(address).unwrap(),
        GenesisAccount {
            code: Bytes::from(hex::decode(code.trim_start_matches("0x")).unwrap()),
            storage: Default::default(),
            balance: U256::zero(),
            nonce: 0,
        },
    );
    store_from(genesis).await
}

/// The l1 test genesis pinned to Prague: `get_max_allowed_gas_limit` consults
/// `block_gas_limit` there, rather than returning the flat EIP-7825 cap it returns on
/// Osaka.
async fn prague_store() -> Store {
    let mut genesis = l1_genesis();
    genesis.config.osaka_time = None;
    store_from(genesis).await
}
