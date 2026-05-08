//! Integration tests that diff ethrex's struct-log tracer output against
//! hand-constructed fixtures derived from geth's `structLogLegacy::toLegacyJSON`
//! (go-ethereum/eth/tracers/logger/logger.go).
//!
//! ## Fixture authorship
//!
//! Geth does not accept external tracer-name strings for the struct logger —
//! it is the implicit default when no tracer is specified in `debug_traceTransaction`.
//! Running geth locally requires `--dev` node setup with deterministic funding and
//! `debug_traceTransaction` curl calls (see `tooling/scripts/gen_structlog_fixtures.sh`
//! for the full regeneration procedure).
//!
//! Because that setup is heavy, the fixtures here are hand-constructed by:
//!
//! 1. Tracing each bytecode through LEVM's struct-log tracer.
//! 2. Verifying each field against the encoding rules documented in
//!    `structLogLegacy` (geth source) and the EIP-3155 spec:
//!    - `pc`, `op`, `gas`, `gasCost`, `depth` always present as decimals.
//!    - `stack` present as bottom-first array of `"0x" + stripped-leading-zeros hex`.
//!    - `memory` present only when `enableMemory=true`; chunked 32-byte `"0x" + 64 hex`.
//!    - `storage` present only at SLOAD/SSTORE when `disableStorage=false`.
//!    - `returnData` present only when `enableReturnData=true` and non-empty.
//!    - `refund` absent when zero.
//!    - `error` absent when no error.
//! 3. Confirming gas arithmetic: intrinsic cost for a no-data EIP-1559 call
//!    with gas_limit=100_000 is 21_000, leaving 79_000 for bytecode execution.
//!    The `gas_used` in the result covers execution gas spent.
//!
//! These values are stable across LEVM versions as long as the gas schedule does not
//! change (Cancun fork rules apply throughout).

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

// ── Helpers ──────────────────────────────────────────────────────────────────

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

/// Runs the struct-log tracer on `bytecode` in a fresh in-memory chain and returns
/// the serialized `StructLogResult` as a `serde_json::Value`.
fn trace_to_json(bytecode: Vec<u8>, cfg: StructLogConfig) -> serde_json::Value {
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

    let test_db = TestDatabase { accounts };
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));
    let header = default_header();
    let tx = make_tx(contract_addr, sender_addr);

    let result = LEVM::trace_tx_struct_log(&mut db, &header, &tx, cfg, VMType::L1, &NativeCrypto)
        .expect("trace should succeed");

    serde_json::to_value(&result).expect("serialize")
}

/// Loads a fixture from `cmd/ethrex/tests/fixtures/` and canonicalizes it for
/// byte-identical comparison (parse → re-serialize via serde_json).
fn load_fixture(name: &str) -> serde_json::Value {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test crate has parent dir")
        .join("cmd/ethrex/tests/fixtures")
        .join(name);
    let content = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", fixture_path.display(), e));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("fixture {} is invalid JSON: {}", name, e))
}

// ── Task 5.1: SSTORE basic ────────────────────────────────────────────────────

/// Bytecode: `PUSH1 0x2a  PUSH1 0x01  SSTORE  STOP`
///
/// This exercises the storage-capture path at the SSTORE step.  The fixture
/// records the accumulated storage map `{slot_1: 0x2a}` on that step.
///
/// Gas accounting (Cancun, EIP-2929):
///   intrinsic = 21 000; execution gas available = 79 000
///   PUSH1×2 = 3+3 = 6; SSTORE cold-new-slot = 2100+20000 = 22100; STOP = 0
///   gas_used = 6 + 22100 = 22106; result.gas = 43106 (includes refund accounting)
#[test]
fn struct_log_sstore_basic_matches_fixture() {
    // PUSH1 0x2a  PUSH1 0x01  SSTORE  STOP
    let bytecode = vec![0x60, 0x2a, 0x60, 0x01, 0x55, 0x00];
    let actual = trace_to_json(bytecode, StructLogConfig::default());
    let expected = load_fixture("eip3155_sstore_basic.json");

    assert_eq!(
        actual, expected,
        "sstore_basic trace does not match fixture"
    );
}

// ── Task 5.2: MSTORE memory ──────────────────────────────────────────────────

/// Bytecode: `PUSH1 0x20  PUSH1 0x00  MSTORE  STOP`
///
/// With `enableMemory=true`, each step emits the current 32-byte-chunked memory.
/// After MSTORE the 32-byte word `0x20` (= 32 decimal) is stored at offset 0.
/// The memory array before MSTORE is empty (`[]`); after MSTORE it contains one
/// chunk: `"0x0000...0020"`.
///
/// Geth emits `memory: []` (empty array, not null) when `enableMemory=true`
/// and memory is still empty, because the field is present whenever the config
/// flag is set (pointer-to-slice in geth, `Some(vec![])` in ethrex).
#[test]
fn struct_log_mstore_memory_matches_fixture() {
    // PUSH1 0x20  PUSH1 0x00  MSTORE  STOP
    let bytecode = vec![0x60, 0x20, 0x60, 0x00, 0x52, 0x00];
    let actual = trace_to_json(
        bytecode,
        StructLogConfig {
            enable_memory: true,
            ..Default::default()
        },
    );
    let expected = load_fixture("eip3155_mstore_memory.json");

    assert_eq!(
        actual, expected,
        "mstore_memory trace does not match fixture"
    );
}

// ── Task 5.3: STATICCALL return data ────────────────────────────────────────

/// Calls the identity precompile (address 0x04) with 1 byte of input (`0x01`).
/// The precompile echoes its input, so the STOP step's `returnData` field
/// should contain `"0x01"`.
///
/// Bytecode (18 bytes):
/// ```
/// PUSH1 0x01  PUSH1 0x00  MSTORE8          -- write 0x01 to mem[0]
/// PUSH1 0x01  PUSH1 0x00                   -- retLen=1, retOffset=0
/// PUSH1 0x01  PUSH1 0x00                   -- argsLen=1, argsOffset=0
/// PUSH1 0x04                               -- addr=identity
/// GAS         STATICCALL                   -- call
/// STOP
/// ```
///
/// With `enableReturnData=true`, the `returnData` field appears on the STOP
/// step (the step AFTER the STATICCALL returns) because geth captures the
/// sub-call's return data at the start of the next opcode's context.
///
/// Choice note: identity precompile (0x04) was chosen over ecrecover (0x01)
/// because it always succeeds and returns predictable non-empty output for
/// any non-empty input, making the fixture deterministic without needing a
/// valid secp256k1 signature.
#[test]
fn struct_log_identity_return_data_matches_fixture() {
    // See bytecode above
    let bytecode = vec![
        0x60, 0x01, 0x60, 0x00, 0x53, 0x60, 0x01, 0x60, 0x00, 0x60, 0x01, 0x60, 0x00, 0x60, 0x04,
        0x5a, 0xfa, 0x00,
    ];
    let actual = trace_to_json(
        bytecode,
        StructLogConfig {
            enable_return_data: true,
            ..Default::default()
        },
    );
    let expected = load_fixture("eip3155_identity_return_data.json");

    assert_eq!(
        actual, expected,
        "identity_return_data trace does not match fixture"
    );
}
