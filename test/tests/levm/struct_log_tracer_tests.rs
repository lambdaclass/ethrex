//! End-to-end smoke test for the EIP-3155 struct-log tracer.
//!
//! Wire-format rules and per-opcode capture semantics are pinned by the unit
//! tests in `ethrex-common` and `ethrex-levm`. This test only verifies that the
//! full RPC pipeline (`LEVM::trace_tx_struct_log` → `serde_json::to_value`)
//! produces a well-formed `StructLogResult` for a real transaction.

use super::test_db::TestDatabase;
use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::tracing::StructLogConfig;
use ethrex_levm::vm::VMType;
use ethrex_vm::backends::levm::LEVM;
use once_cell::sync::OnceCell;
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// `PUSH1 0x2a  PUSH1 0x01  SSTORE  STOP` — runs through the full RPC pipeline
/// and asserts the resulting JSON has the EIP-3155 strict shape.
#[test]
fn struct_log_pipeline_smoke() {
    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);
    let bytecode = Bytes::from(vec![0x60, 0x2a, 0x60, 0x01, 0x55, 0x00]);

    let mut accounts = FxHashMap::default();
    accounts.insert(
        contract_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(bytecode, &NativeCrypto),
            1,
            FxHashMap::default(),
        ),
    );
    accounts.insert(
        sender_addr,
        Account::new(
            U256::from(10u64) * U256::from(10u64).pow(U256::from(18)),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );

    let mut db = GeneralizedDatabase::new(Arc::new(TestDatabase { accounts }));
    let header = BlockHeader {
        coinbase: Address::from_low_u64_be(0xCCC),
        base_fee_per_gas: Some(1),
        gas_limit: 30_000_000,
        ..Default::default()
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 10,
        gas_limit: 100_000,
        to: TxKind::Call(contract_addr),
        value: U256::zero(),
        data: Bytes::new(),
        access_list: vec![],
        signature_y_parity: false,
        signature_r: U256::one(),
        signature_s: U256::one(),
        inner_hash: OnceCell::new(),
        sender_cache: {
            let cell = OnceCell::new();
            let _ = cell.set(sender_addr);
            cell
        },
        cached_canonical: OnceCell::new(),
    });

    let result = LEVM::trace_tx_struct_log(
        &mut db,
        &header,
        &tx,
        StructLogConfig::default(),
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace should succeed");
    let j = serde_json::to_value(&result).expect("serialize");

    // Wrapper shape: pass / gasUsed (hex) / output (hex) / structLogs.
    assert_eq!(j["pass"], serde_json::Value::Bool(true));
    let gas_used = j["gasUsed"].as_str().expect("gasUsed is hex string");
    assert!(gas_used.starts_with("0x"));
    assert_eq!(j["output"], serde_json::Value::String("0x".to_string()));

    let logs = j["structLogs"].as_array().expect("structLogs is array");
    assert_eq!(logs.len(), 4, "PUSH1 PUSH1 SSTORE STOP");

    // EIP-3155 strict per-step fields on the SSTORE entry (index 2).
    let sstore = &logs[2];
    assert_eq!(sstore["op"].as_u64(), Some(0x55), "op is numeric byte");
    assert_eq!(sstore["opName"].as_str(), Some("SSTORE"));
    assert!(sstore["gas"].as_str().is_some_and(|s| s.starts_with("0x")));
    assert!(
        sstore["gasCost"]
            .as_str()
            .is_some_and(|s| s.starts_with("0x"))
    );
    assert_eq!(sstore["refund"].as_str(), Some("0x0"));
    assert_eq!(sstore["returnData"].as_str(), Some("0x"));
    assert!(sstore["memSize"].is_number());
    assert_eq!(sstore["depth"].as_u64(), Some(1));
    assert!(sstore["stack"].is_array());
    let storage = sstore["storage"].as_object().expect("storage object");
    assert_eq!(storage.len(), 1, "single entry, no accumulation");
}
