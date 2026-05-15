//! End-to-end tests for the EIP-3155 `opcodeTracer`.
//!
//! Each test deploys a small bytecode through the full RPC pipeline
//! (`LEVM::trace_tx_opcodes` -> `serde_json::to_value`) and asserts on the
//! resulting JSON shape. Behaviour is verified at the wire-format boundary,
//! not on internal Rust types. Per-step content is EIP-3155: `op` is a numeric
//! opcode byte; `gas`, `gasCost`, `refund` are `"0xN"` hex strings ("Hex-Number"
//! per spec); `pc`, `memSize`, `depth` are JSON numbers; `stack` is always an
//! array (never null) of `"0xN"` hex strings. The string mnemonic is emitted
//! separately under `opName`. Steps live inside the geth-RPC-compat
//! `{failed, gas, returnValue, structLogs}` wrapper.

use super::test_db::TestDatabase;
use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::tracing::OpcodeTracerConfig;
use ethrex_levm::vm::VMType;
use ethrex_vm::backends::levm::LEVM;
use once_cell::sync::OnceCell;
use rustc_hash::FxHashMap;
use serde_json::Value;
use std::sync::Arc;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Parses an EIP-3155 "Hex-Number" field (`"0xN"`) to `u64`.
fn hex_u64(v: &Value) -> u64 {
    let s = v.as_str().expect("hex-number field must be a string");
    u64::from_str_radix(s.trim_start_matches("0x"), 16).expect("valid hex u64")
}

fn default_header() -> BlockHeader {
    BlockHeader {
        coinbase: Address::from_low_u64_be(0xCCC),
        base_fee_per_gas: Some(1),
        gas_limit: 30_000_000,
        ..Default::default()
    }
}

fn make_tx(contract: Address, sender: Address) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 10,
        gas_limit: 100_000,
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        access_list: vec![],
        signature_y_parity: false,
        signature_r: U256::one(),
        signature_s: U256::one(),
        inner_hash: OnceCell::new(),
        sender_cache: {
            let cell = OnceCell::new();
            let _ = cell.set(sender);
            cell
        },
        cached_canonical: OnceCell::new(),
    })
}

/// Runs `bytecode` under a contract account with `cfg` and returns the
/// serialized `OpcodeTraceResult` as a `serde_json::Value`.
fn trace_to_json(bytecode: Vec<u8>, cfg: OpcodeTracerConfig) -> Value {
    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);

    let mut accounts = FxHashMap::default();
    accounts.insert(
        contract_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(bytecode), &NativeCrypto),
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
    let header = default_header();
    let tx = make_tx(contract_addr, sender_addr);

    let result = LEVM::trace_tx_opcodes(&mut db, &header, &tx, cfg, VMType::L1, &NativeCrypto)
        .expect("trace should succeed");
    serde_json::to_value(&result).expect("serialize")
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// `PUSH1 0x01 PUSH1 0x02 ADD STOP`
///
/// Pins the wrapper (`failed`/`gas`/`returnValue`/`structLogs`) and the EIP-3155
/// per-step shape: numeric `op` byte, separate string `opName`, hex `gas`/
/// `gasCost`/`refund`, decimal `pc`/`memSize`/`depth`, bottom-first `stack`,
/// always-present `returnData`.
#[test]
fn opcode_tracer_basic_execution() {
    let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00];
    let j = trace_to_json(bytecode, OpcodeTracerConfig::default());

    assert_eq!(j["failed"], Value::Bool(false));
    assert!(j["gas"].is_number(), "wrapper gas is a number");
    assert_eq!(j["returnValue"], Value::String("0x".to_string()));

    let steps = j["structLogs"].as_array().expect("structLogs is array");
    assert_eq!(steps.len(), 4, "PUSH1 PUSH1 ADD STOP");

    // PUSH1 0x01 — first step, empty stack pre-execution.
    assert_eq!(steps[0]["pc"], Value::Number(0.into()));
    assert_eq!(steps[0]["op"].as_u64(), Some(0x60));
    assert_eq!(steps[0]["opName"].as_str(), Some("PUSH1"));
    assert!(steps[0]["gas"].is_string(), "gas is a hex string");
    assert_eq!(steps[0]["gasCost"].as_str(), Some("0x3"));
    assert_eq!(steps[0]["depth"].as_u64(), Some(1));
    assert_eq!(steps[0]["refund"].as_str(), Some("0x0"));
    assert_eq!(steps[0]["returnData"].as_str(), Some("0x"));
    assert_eq!(steps[0]["memSize"].as_u64(), Some(0));
    assert_eq!(steps[0]["stack"], Value::Array(vec![]));

    // ADD — third step, stack bottom-first [0x1, 0x2] pre-execution.
    assert_eq!(steps[2]["op"].as_u64(), Some(0x01));
    assert_eq!(steps[2]["opName"].as_str(), Some("ADD"));
    let add_stack = steps[2]["stack"].as_array().expect("stack array");
    assert_eq!(add_stack[0], Value::String("0x1".to_string()));
    assert_eq!(add_stack[1], Value::String("0x2".to_string()));

    // STOP — final step, stack collapsed to [0x3].
    assert_eq!(steps[3]["op"].as_u64(), Some(0x00));
    assert_eq!(steps[3]["opName"].as_str(), Some("STOP"));
    let stop_stack = steps[3]["stack"].as_array().expect("stack array");
    assert_eq!(stop_stack, &vec![Value::String("0x3".to_string())]);
}

/// `PUSH1 0x2a PUSH1 0x01 SSTORE STOP`
///
/// SSTORE step's `storage` map must be a **single-entry** object (no
/// accumulation across the transaction). Non-SLOAD/SSTORE steps omit the
/// field entirely.
#[test]
fn opcode_tracer_sstore_single_entry_storage() {
    let bytecode = vec![0x60, 0x2a, 0x60, 0x01, 0x55, 0x00];
    let j = trace_to_json(bytecode, OpcodeTracerConfig::default());
    let steps = j["structLogs"].as_array().expect("structLogs");
    assert_eq!(steps.len(), 4);

    // PUSH1 / PUSH1 — no storage field.
    assert!(steps[0].get("storage").is_none());
    assert!(steps[1].get("storage").is_none());

    // SSTORE — exactly one entry, key=0x01, value=0x2a.
    let sstore = &steps[2];
    assert_eq!(sstore["op"].as_u64(), Some(0x55));
    assert_eq!(sstore["opName"].as_str(), Some("SSTORE"));
    let storage = sstore["storage"].as_object().expect("storage object");
    assert_eq!(storage.len(), 1, "single entry, no accumulation");
    let key = format!("0x{:0>64}", "1");
    let val = format!("0x{:0>64}", "2a");
    assert_eq!(
        storage.get(&key).and_then(Value::as_str),
        Some(val.as_str())
    );

    // STOP — no storage field.
    assert!(steps[3].get("storage").is_none());
}

/// `PUSH1 0x20 PUSH1 0x00 MSTORE STOP` with `enableMemory=true`
///
/// Memory grows by one 32-byte word after MSTORE. The STOP step (captured
/// after MSTORE executes) carries `memory: ["0x000...0020"]` and `memSize: 32`.
#[test]
fn opcode_tracer_memory_capture_when_enabled() {
    let bytecode = vec![0x60, 0x20, 0x60, 0x00, 0x52, 0x00];
    let cfg = OpcodeTracerConfig {
        enable_memory: true,
        ..Default::default()
    };
    let j = trace_to_json(bytecode, cfg);
    let steps = j["structLogs"].as_array().expect("structLogs");

    let stop = steps.last().expect("at least one step");
    assert_eq!(stop["op"].as_u64(), Some(0x00));
    assert_eq!(stop["opName"].as_str(), Some("STOP"));
    assert_eq!(stop["memSize"].as_u64(), Some(32));
    let mem = stop["memory"].as_array().expect("memory array");
    assert_eq!(mem.len(), 1);
    let expected = format!("0x{:0>64}", "20");
    assert_eq!(mem[0].as_str(), Some(expected.as_str()));
}

/// `MSTORE8 + STATICCALL 0x04 (identity) + STOP` with `enableReturnData=true`
///
/// Identity precompile echoes its input. After STATICCALL returns, the
/// subsequent STOP step surfaces `returnData: "0x01"`.
#[test]
fn opcode_tracer_return_data_capture_when_enabled() {
    let bytecode = vec![
        0x60, 0x01, 0x60, 0x00, 0x53, // PUSH1 0x01 PUSH1 0x00 MSTORE8
        0x60, 0x01, 0x60, 0x00, // retLen=1 retOff=0
        0x60, 0x01, 0x60, 0x00, // argsLen=1 argsOff=0
        0x60, 0x04, // identity precompile addr
        0x5a, 0xfa, // GAS STATICCALL
        0x00, // STOP
    ];
    let cfg = OpcodeTracerConfig {
        enable_return_data: true,
        ..Default::default()
    };
    let j = trace_to_json(bytecode, cfg);
    let steps = j["structLogs"].as_array().expect("structLogs");

    let stop = steps.last().expect("at least one step");
    assert_eq!(stop["op"].as_u64(), Some(0x00));
    assert_eq!(stop["opName"].as_str(), Some("STOP"));
    assert_eq!(stop["returnData"].as_str(), Some("0x01"));
}

/// `PUSH1 0x01 PUSH1 0x02 ADD STOP` with `disableStack=true`
///
/// EIP-3155 mandates: "All array attributes (`stack`, `memory`) MUST be
/// initialized to empty arrays NOT to null". So when stack capture is disabled,
/// the field still appears as `[]` rather than `null` or being absent.
#[test]
fn opcode_tracer_stack_disabled_is_empty_array() {
    let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00];
    let cfg = OpcodeTracerConfig {
        disable_stack: true,
        ..Default::default()
    };
    let j = trace_to_json(bytecode, cfg);
    let steps = j["structLogs"].as_array().expect("structLogs");

    for step in steps {
        assert_eq!(
            step["stack"],
            Value::Array(vec![]),
            "EIP-3155: stack must serialize as [] when disabled, not null",
        );
    }
}

/// `PUSH1 0x04 JUMP JUMPDEST STOP`
///
/// Verifies the fused JUMP + JUMPDEST optimization synthesizes a JUMPDEST trace
/// entry: the JUMP step's `gasCost` is exactly 8 (not 9, which would include
/// the absorbed JUMPDEST charge), and a JUMPDEST step follows it with
/// `gasCost = 1`.
#[test]
fn opcode_tracer_jumpdest_synthesized_after_jump() {
    // pc=0: PUSH1 0x04
    // pc=2: JUMP
    // pc=3: INVALID (padding, never executed)
    // pc=4: JUMPDEST
    // pc=5: STOP
    let bytecode = vec![0x60, 0x04, 0x56, 0xfe, 0x5b, 0x00];
    let j = trace_to_json(bytecode, OpcodeTracerConfig::default());
    let steps = j["structLogs"].as_array().expect("structLogs");

    assert_eq!(steps.len(), 4, "PUSH1 / JUMP / JUMPDEST / STOP");

    assert_eq!(steps[0]["op"].as_u64(), Some(0x60));
    assert_eq!(steps[0]["opName"].as_str(), Some("PUSH1"));

    assert_eq!(steps[1]["op"].as_u64(), Some(0x56));
    assert_eq!(steps[1]["opName"].as_str(), Some("JUMP"));
    assert_eq!(
        steps[1]["gasCost"].as_str(),
        Some("0x8"),
        "JUMP gasCost must not absorb the JUMPDEST charge"
    );

    assert_eq!(steps[2]["op"].as_u64(), Some(0x5b));
    assert_eq!(steps[2]["opName"].as_str(), Some("JUMPDEST"));
    assert_eq!(steps[2]["pc"].as_u64(), Some(4));
    assert_eq!(steps[2]["gasCost"].as_str(), Some("0x1"));
    assert_eq!(steps[2]["depth"].as_u64(), Some(1));
    // Gas remaining at JUMPDEST = gas at JUMP minus JUMP's 8.
    let jump_gas = hex_u64(&steps[1]["gas"]);
    let jumpdest_gas = hex_u64(&steps[2]["gas"]);
    assert_eq!(jumpdest_gas, jump_gas - 8);

    assert_eq!(steps[3]["op"].as_u64(), Some(0x00));
    assert_eq!(steps[3]["opName"].as_str(), Some("STOP"));
    // STOP gas reflects the JUMPDEST charge having been consumed.
    let stop_gas = hex_u64(&steps[3]["gas"]);
    assert_eq!(stop_gas, jumpdest_gas - 1);
}
