//! Tests for the `debug_traceCall` VM-level entry points
//! ([`LEVM::trace_call_calls`] / [`LEVM::trace_call_opcodes`] /
//! [`LEVM::trace_call_prestate`]).
//!
//! Unlike the `trace_tx_*` family, these take an unsigned [`GenericTransaction`]
//! (the `eth_call`-shaped RPC input) and must derive the sender from its `from`
//! field rather than recovering it from a signature. These tests pin that
//! behaviour by tracing calls into a deployed contract with no signature present.

use super::test_db::TestDatabase;
use bytes::Bytes;
use ethrex_common::tracing::{PrestateResult, StructLoggerEmit, StructLoggerResult};
use ethrex_common::{
    Address, H256, U256,
    types::{Account, BlockHeader, Code, GenericTransaction, TxKind},
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::tracing::OpcodeTracerConfig;
use ethrex_levm::vm::VMType;
use ethrex_vm::backends::levm::LEVM;
use rustc_hash::FxHashMap;
use std::sync::Arc;

const CONTRACT: u64 = 0xC000;
const SENDER: u64 = 0x1000;

fn default_header() -> BlockHeader {
    BlockHeader {
        coinbase: Address::from_low_u64_be(0xCCC),
        base_fee_per_gas: Some(1),
        gas_limit: 30_000_000,
        ..Default::default()
    }
}

/// Builds a `GeneralizedDatabase` with `bytecode` deployed at `CONTRACT` and a
/// funded EOA at `SENDER`.
fn db_with_contract(bytecode: Vec<u8>) -> GeneralizedDatabase {
    let mut accounts = FxHashMap::default();
    accounts.insert(
        Address::from_low_u64_be(CONTRACT),
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(bytecode), &NativeCrypto),
            1,
            FxHashMap::default(),
        ),
    );
    accounts.insert(
        Address::from_low_u64_be(SENDER),
        Account::new(
            U256::from(10u64) * U256::from(10u64).pow(U256::from(18)),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    GeneralizedDatabase::new(Arc::new(TestDatabase { accounts }))
}

/// Unsigned call into `CONTRACT` with the sender provided only via `from`, and a
/// zero gas price so fee/balance checks are relaxed (matching `eth_call`).
fn call_tx() -> GenericTransaction {
    GenericTransaction {
        to: TxKind::Call(Address::from_low_u64_be(CONTRACT)),
        from: Address::from_low_u64_be(SENDER),
        gas: Some(100_000),
        gas_price: U256::zero(),
        ..Default::default()
    }
}

/// `PUSH1 0x01 PUSH1 0x02 ADD STOP`: the callTracer must report the top frame's
/// `from`/`to` taken from the generic tx (no signature recovery) and a clean exit.
#[test]
fn trace_call_calls_uses_from_field() {
    let mut db = db_with_contract(vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00]);
    let header = default_header();
    let tx = call_tx();

    let trace = LEVM::trace_call_calls(
        &mut db,
        &header,
        &tx,
        false,
        false,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_calls should succeed");

    assert_eq!(trace.len(), 1, "single top-level call frame");
    let frame = &trace[0];
    assert_eq!(frame.from, Address::from_low_u64_be(SENDER));
    assert_eq!(frame.to, Address::from_low_u64_be(CONTRACT));
    assert!(
        frame.error.is_none(),
        "call should not error: {:?}",
        frame.error
    );
}

/// The opcode tracer over a generic call yields the expected step sequence.
#[test]
fn trace_call_opcodes_produces_steps() {
    let mut db = db_with_contract(vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00]);
    let header = default_header();
    let tx = call_tx();

    let result = LEVM::trace_call_opcodes(
        &mut db,
        &header,
        &tx,
        OpcodeTracerConfig::default(),
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_opcodes should succeed");

    let json = serde_json::to_value(StructLoggerResult {
        result: &result,
        emit: StructLoggerEmit {
            mem_size: false,
            return_data: false,
            refund: false,
        },
    })
    .expect("serialize");
    let steps = json["structLogs"].as_array().expect("structLogs array");
    assert_eq!(steps.len(), 4, "PUSH1 PUSH1 ADD STOP");
    assert_eq!(steps[0]["op"].as_str(), Some("PUSH1"));
    assert_eq!(steps[2]["op"].as_str(), Some("ADD"));
    assert_eq!(steps[3]["op"].as_str(), Some("STOP"));
}

/// `PUSH1 0x2a PUSH1 0x01 SSTORE STOP`: the prestate tracer (diff mode) must
/// surface the storage write performed by the traced call.
#[test]
fn trace_call_prestate_diff_captures_storage_write() {
    let mut db = db_with_contract(vec![0x60, 0x2a, 0x60, 0x01, 0x55, 0x00]);
    let header = default_header();
    let tx = call_tx();

    let result = LEVM::trace_call_prestate(
        &mut db,
        &header,
        &tx,
        /* diff_mode */ true,
        /* include_empty */ false,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_prestate should succeed");

    let PrestateResult::Diff(diff) = result else {
        panic!("diff_mode must yield a Diff result");
    };
    let contract = Address::from_low_u64_be(CONTRACT);
    let post = diff
        .post
        .get(&contract)
        .expect("contract present in post state");
    let slot = H256::from_low_u64_be(0x01);
    assert_eq!(
        post.storage.get(&slot).copied(),
        Some(H256::from_low_u64_be(0x2a)),
        "slot 0x01 must be written to 0x2a"
    );
}
