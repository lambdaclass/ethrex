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
    types::{
        Account, AccountState, BlockHeader, ChainConfig, Code, CodeMetadata, GenericTransaction,
        TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::Database;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::errors::DatabaseError;
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
        0,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_calls should succeed");

    assert_eq!(trace.len(), 1, "single top-level call frame");
    let frame = &trace[0];
    assert_eq!(frame.from, Address::from_low_u64_be(SENDER));
    assert_eq!(frame.to, Some(Address::from_low_u64_be(CONTRACT)));
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

/// Wraps [`TestDatabase`] but reports an Amsterdam-active chain config, so the
/// EIP-7778 split between block-level gas (`ctx_result.gas_used`) and the
/// post-refund gas the sender pays (`ctx_result.gas_spent`) is exercised.
/// Pre-Amsterdam the two are equal, which would make the refund regression below
/// vacuous.
struct AmsterdamDb {
    inner: TestDatabase,
}

impl Database for AmsterdamDb {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        self.inner.get_account_state(address)
    }
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        self.inner.get_storage_value(address, key)
    }
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        self.inner.get_block_hash(block_number)
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig {
            amsterdam_time: Some(0),
            ..ChainConfig::default()
        })
    }
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        self.inner.get_account_code(code_hash)
    }
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        self.inner.get_code_metadata(code_hash)
    }
}

/// Amsterdam requires a `slot_number` on L1 block headers.
fn amsterdam_header() -> BlockHeader {
    BlockHeader {
        slot_number: Some(0),
        ..default_header()
    }
}

/// Builds an Amsterdam-configured DB with `bytecode` at `CONTRACT` (storage slot
/// `0x01` pre-set to `0x2a` so an SSTORE-to-zero produces an EIP-3529 refund) and
/// a funded EOA at `SENDER`.
fn amsterdam_db_with_stored_slot(bytecode: Vec<u8>) -> GeneralizedDatabase {
    let mut accounts = FxHashMap::default();
    let mut storage = FxHashMap::default();
    storage.insert(H256::from_low_u64_be(0x01), U256::from(0x2au64));
    accounts.insert(
        Address::from_low_u64_be(CONTRACT),
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(bytecode), &NativeCrypto),
            1,
            storage,
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
    GeneralizedDatabase::new(Arc::new(AmsterdamDb {
        inner: TestDatabase { accounts },
    }))
}

/// Regression: the callTracer's top-level `gasUsed` must be the transaction's
/// post-refund gas (matching the receipt and geth's `callstack[0].GasUsed =
/// receipt.GasUsed`), NOT the pre-refund / EIP-7778 block-accounting value.
///
/// `PUSH1 0x00 PUSH1 0x01 SSTORE STOP` clears a pre-set storage slot, granting an
/// EIP-3529 refund. The opcode tracer already reports `gas_spent` (post-refund),
/// so it serves as the independent reference: the callTracer top frame must match
/// it, and both must be strictly below the pre-refund value that plain execution
/// reports for block accounting (proving the refund is actually applied).
#[test]
fn trace_call_calls_top_frame_gas_is_post_refund() {
    let bytecode = vec![0x60, 0x00, 0x60, 0x01, 0x55, 0x00];
    let header = amsterdam_header();
    let tx = call_tx();

    let mut db = amsterdam_db_with_stored_slot(bytecode.clone());
    let trace = LEVM::trace_call_calls(
        &mut db,
        &header,
        &tx,
        false,
        false,
        0,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_calls should succeed");
    let call_gas_used = trace[0].gas_used;

    // Opcode tracer reports post-refund gas (`ctx_result.gas_spent`).
    let mut db = amsterdam_db_with_stored_slot(bytecode.clone());
    let opcode_result = LEVM::trace_call_opcodes(
        &mut db,
        &header,
        &tx,
        OpcodeTracerConfig::default(),
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_opcodes should succeed");

    // Plain execution surfaces the pre-refund / block-accounting `gas_used`.
    let mut db = amsterdam_db_with_stored_slot(bytecode);
    let exec = LEVM::simulate_tx_from_generic(&tx, &header, &mut db, VMType::L1, &NativeCrypto)
        .expect("simulate_tx_from_generic should succeed");

    assert_eq!(
        call_gas_used, opcode_result.gas_used,
        "callTracer top-frame gasUsed must equal the post-refund gas the opcode tracer reports"
    );
    assert!(
        call_gas_used < exec.gas_used(),
        "a refund must be applied: post-refund {call_gas_used} should be below pre-refund {}",
        exec.gas_used()
    );
}

/// Bytecode that CODECOPYs a trailing `Error("boom")` ABI payload into memory and
/// REVERTs it — the canonical Solidity `require`/`revert("...")` shape.
fn revert_with_boom_bytecode() -> Vec<u8> {
    // 12-byte prologue: CODECOPY(dest=0, off=0x0c, len=0x64); REVERT(0, 0x64).
    let mut code = vec![
        0x60, 0x64, // PUSH1 100  (len)
        0x60, 0x0c, // PUSH1 12   (code offset where payload starts)
        0x60, 0x00, // PUSH1 0    (dest)
        0x39, // CODECOPY
        0x60, 0x64, // PUSH1 100
        0x60, 0x00, // PUSH1 0
        0xfd, // REVERT
    ];
    // ABI-encoded Error(string) with "boom": selector, offset(0x20), len(4), data.
    code.extend_from_slice(&[0x08, 0xc3, 0x79, 0xa0]); // selector
    let mut word = [0u8; 32];
    word[31] = 0x20;
    code.extend_from_slice(&word); // offset = 0x20
    word[31] = 0x04;
    code.extend_from_slice(&word); // length = 4
    let mut data = [0u8; 32];
    data[..4].copy_from_slice(b"boom");
    code.extend_from_slice(&data); // "boom" padded
    code
}

/// A reverting call must report geth's `"execution reverted"` error and the
/// ABI-decoded `Error(string)` revert reason (geth's `abi.UnpackRevert`), with the
/// raw revert data surfaced as `output`.
#[test]
fn trace_call_calls_decodes_revert_reason() {
    let mut db = db_with_contract(revert_with_boom_bytecode());
    let header = default_header();
    let tx = call_tx();

    let trace = LEVM::trace_call_calls(
        &mut db,
        &header,
        &tx,
        false,
        false,
        0,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_calls should succeed");

    let frame = &trace[0];
    assert_eq!(frame.error.as_deref(), Some("execution reverted"));
    assert_eq!(frame.revert_reason.as_deref(), Some("boom"));
    assert!(!frame.output.is_empty(), "revert data must be surfaced");
}

/// A non-revert exceptional halt maps to geth's error wording. `INVALID` (0xfe) →
/// `"invalid opcode"`, with no output/revertReason surfaced.
#[test]
fn trace_call_calls_maps_halt_error_to_geth() {
    let mut db = db_with_contract(vec![0xfe]);
    let header = default_header();
    let tx = call_tx();

    let trace = LEVM::trace_call_calls(
        &mut db,
        &header,
        &tx,
        false,
        false,
        0,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_calls should succeed");

    let frame = &trace[0];
    assert_eq!(frame.error.as_deref(), Some("invalid opcode"));
    assert!(frame.revert_reason.is_none());
    assert!(frame.output.is_empty(), "no output on a non-revert halt");
}

/// `withLog` logs carry a block-absolute `index` (geth's `log.Index`) seeded from the
/// preceding txs' log count. Two `LOG0`s traced with base 5 must get indices 5 and 6.
#[test]
fn trace_call_calls_log_index_is_block_absolute() {
    // PUSH1 0 PUSH1 0 LOG0  (x2)  STOP
    let bytecode = vec![
        0x60, 0x00, 0x60, 0x00, 0xa0, // LOG0
        0x60, 0x00, 0x60, 0x00, 0xa0, // LOG0
        0x00, // STOP
    ];
    let mut db = db_with_contract(bytecode);
    let header = default_header();
    let tx = call_tx();

    let trace = LEVM::trace_call_calls(
        &mut db,
        &header,
        &tx,
        false,
        /* with_log */ true,
        /* log_index_base */ 5,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_calls should succeed");

    let logs = &trace[0].logs;
    assert_eq!(logs.len(), 2, "both LOG0s captured");
    assert_eq!((logs[0].index, logs[0].position), (5, 0));
    assert_eq!((logs[1].index, logs[1].position), (6, 0));
}

/// The serialized top frame must omit geth's optional fields when they carry no
/// information: no `error`/`revertReason`/`calls` and no empty `output` on a clean
/// call. `to`/`value`/`input` remain present.
#[test]
fn trace_call_calls_omits_empty_fields() {
    let mut db = db_with_contract(vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00]);
    let header = default_header();
    let tx = call_tx();

    let trace = LEVM::trace_call_calls(
        &mut db,
        &header,
        &tx,
        false,
        false,
        0,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_calls should succeed");

    let json = serde_json::to_value(&trace[0]).expect("serialize");
    let obj = json.as_object().expect("frame is an object");
    for absent in ["error", "revertReason", "calls", "output"] {
        assert!(
            !obj.contains_key(absent),
            "{absent} must be omitted when empty"
        );
    }
    for present in ["type", "from", "to", "value", "gas", "gasUsed", "input"] {
        assert!(obj.contains_key(present), "{present} must be present");
    }
}

// ===========================================================================
// EIP-8037 two-dimensional gas breakdown on the callTracer top frame
// (execution-apis `CallFrame.regularGasUsed`/`stateGasUsed`/`gasRefund`).
// ===========================================================================

/// Wraps [`TestDatabase`] but reports an Osaka-active (pre-Amsterdam) chain config, to
/// serve as the "just before the fork" control for the EIP-8037 field-omission tests.
struct OsakaDb {
    inner: TestDatabase,
}

impl Database for OsakaDb {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        self.inner.get_account_state(address)
    }
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        self.inner.get_storage_value(address, key)
    }
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        self.inner.get_block_hash(block_number)
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig {
            osaka_time: Some(0),
            ..ChainConfig::default()
        })
    }
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        self.inner.get_account_code(code_hash)
    }
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        self.inner.get_code_metadata(code_hash)
    }
}

/// Emits a PUSH20 + 20 bytes of `addr`, for building CALL-opcode bytecode inline.
fn push20(addr: Address) -> Vec<u8> {
    let mut v = vec![0x73u8]; // PUSH20
    v.extend_from_slice(addr.as_bytes());
    v
}

/// Bytecode that exercises both EIP-8037 gas dimensions in one transaction:
/// `SSTORE` into a fresh slot (charges state gas, since original=0/new!=0) followed by
/// clearing a pre-existing non-zero slot to zero (grants an EIP-3529 refund). Combining
/// both keeps the accounting-identity test (point 4) non-vacuous on either dimension.
fn state_gas_and_refund_bytecode() -> Vec<u8> {
    vec![
        0x60, 0x05, // PUSH1 5   (value)
        0x60, 0x01, // PUSH1 1   (key: fresh slot)
        0x55, // SSTORE (0->5: charges state gas)
        0x60, 0x00, // PUSH1 0   (value)
        0x60, 0x02, // PUSH1 2   (key: pre-set slot)
        0x55, // SSTORE (0x2a->0: EIP-3529 refund)
        0x00, // STOP
    ]
}

/// Builds an Amsterdam-configured DB with `bytecode` at `CONTRACT` (storage slot `0x02`
/// pre-set to `0x2a`, slot `0x01` absent) and a funded EOA at `SENDER`. `extra_accounts`
/// lets a test inject additional accounts (e.g. a callee for a CALL sub-frame).
fn amsterdam_db_for_gas_breakdown(
    bytecode: Vec<u8>,
    extra_accounts: &[(Address, Account)],
) -> GeneralizedDatabase {
    let mut storage = FxHashMap::default();
    storage.insert(H256::from_low_u64_be(0x02), U256::from(0x2au64));
    let mut accounts = FxHashMap::default();
    accounts.insert(
        Address::from_low_u64_be(CONTRACT),
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(bytecode), &NativeCrypto),
            1,
            storage,
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
    for (addr, acc) in extra_accounts {
        accounts.insert(*addr, acc.clone());
    }
    GeneralizedDatabase::new(Arc::new(AmsterdamDb {
        inner: TestDatabase { accounts },
    }))
}

/// Unsigned call into `CONTRACT` like [`call_tx`], but with a caller-chosen gas limit —
/// EIP-8037 state gas (e.g. `STORAGE_SET` for a fresh `SSTORE`) is large enough that
/// `call_tx`'s default 100_000 gas is insufficient for these tests.
fn call_tx_with_gas(gas: u64) -> GenericTransaction {
    GenericTransaction {
        gas: Some(gas),
        ..call_tx()
    }
}

/// On an Amsterdam+ transaction the top-level frame carries `regularGasUsed`,
/// `stateGasUsed` and `gasRefund`, and they satisfy the EIP-8037 accounting identity
/// `regularGasUsed + stateGasUsed == gasUsed + gasRefund` (both dimensions non-zero here,
/// so the identity isn't vacuously true and the calldata floor has no calldata to bind on).
#[test]
fn trace_call_calls_amsterdam_reports_eip8037_gas_breakdown() {
    let mut db = amsterdam_db_for_gas_breakdown(state_gas_and_refund_bytecode(), &[]);
    let header = amsterdam_header();
    let tx = call_tx_with_gas(500_000);

    let trace = LEVM::trace_call_calls(
        &mut db,
        &header,
        &tx,
        false,
        false,
        0,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_calls should succeed");

    let frame = &trace[0];
    assert!(
        frame.error.is_none(),
        "call should not error: {:?}",
        frame.error
    );

    let regular = frame
        .regular_gas_used
        .expect("regularGasUsed must be set on Amsterdam+");
    let state = frame
        .state_gas_used
        .expect("stateGasUsed must be set on Amsterdam+");
    let refund = frame
        .gas_refund
        .expect("gasRefund must be set on Amsterdam+");

    assert!(state > 0, "SSTORE into a fresh slot must charge state gas");
    assert!(
        refund > 0,
        "clearing a pre-existing slot must grant an EIP-3529 refund"
    );
    assert_eq!(
        regular + state,
        frame.gas_used + refund,
        "EIP-8037 accounting identity: regularGasUsed + stateGasUsed == gasUsed + gasRefund"
    );

    let json = serde_json::to_value(frame).expect("serialize");
    assert_eq!(json["regularGasUsed"], format!("{regular:#x}"));
    assert_eq!(json["stateGasUsed"], format!("{state:#x}"));
    assert_eq!(json["gasRefund"], format!("{refund:#x}"));
}

/// Pre-Amsterdam (Osaka), the top-level frame must omit `regularGasUsed`, `stateGasUsed`
/// and `gasRefund` entirely from the serialized JSON — the fields don't exist yet.
#[test]
fn trace_call_calls_pre_amsterdam_omits_eip8037_fields() {
    let mut storage = FxHashMap::default();
    storage.insert(H256::from_low_u64_be(0x02), U256::from(0x2au64));
    let mut accounts = FxHashMap::default();
    accounts.insert(
        Address::from_low_u64_be(CONTRACT),
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(state_gas_and_refund_bytecode()), &NativeCrypto),
            1,
            storage,
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
    let mut db = GeneralizedDatabase::new(Arc::new(OsakaDb {
        inner: TestDatabase { accounts },
    }));
    let header = default_header();
    let tx = call_tx();

    let trace = LEVM::trace_call_calls(
        &mut db,
        &header,
        &tx,
        false,
        false,
        0,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_calls should succeed");

    let frame = &trace[0];
    assert!(frame.regular_gas_used.is_none());
    assert!(frame.state_gas_used.is_none());
    assert!(frame.gas_refund.is_none());

    let json = serde_json::to_value(frame).expect("serialize");
    let obj = json.as_object().expect("frame is an object");
    for absent in ["regularGasUsed", "stateGasUsed", "gasRefund"] {
        assert!(
            !obj.contains_key(absent),
            "{absent} must be omitted pre-Amsterdam"
        );
    }
}

/// Only the top-level frame is stamped with the EIP-8037 gas breakdown; a sub-frame
/// produced by a `CALL` into another contract must omit `regularGasUsed`/`stateGasUsed`/
/// `gasRefund` even on an Amsterdam+ transaction.
#[test]
fn trace_call_calls_amsterdam_subframe_omits_eip8037_fields() {
    let callee = Address::from_low_u64_be(0xD000);
    let callee_account = Account::new(U256::zero(), Code::default(), 0, FxHashMap::default());

    // CALL(gas=0xFFFF, addr=callee, value=0, argsOff=0, argsLen=0, retOff=0, retLen=0); POP; STOP
    let mut caller_code = vec![
        0x60, 0x00, // PUSH1 0  retLen
        0x60, 0x00, // PUSH1 0  retOff
        0x60, 0x00, // PUSH1 0  argsLen
        0x60, 0x00, // PUSH1 0  argsOff
        0x60, 0x00, // PUSH1 0  value
    ];
    caller_code.extend_from_slice(&push20(callee));
    caller_code.extend_from_slice(&[
        0x61, 0xFF, 0xFF, // PUSH2 0xFFFF  gas
        0xF1, // CALL
        0x50, // POP success flag
        0x00, // STOP
    ]);

    let mut db = amsterdam_db_for_gas_breakdown(caller_code, &[(callee, callee_account)]);
    let header = amsterdam_header();
    let tx = call_tx();

    let trace = LEVM::trace_call_calls(
        &mut db,
        &header,
        &tx,
        false,
        false,
        0,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace_call_calls should succeed");

    let top = &trace[0];
    assert!(
        top.error.is_none(),
        "call should not error: {:?}",
        top.error
    );
    assert!(
        top.regular_gas_used.is_some(),
        "top frame must carry regularGasUsed on Amsterdam+"
    );
    assert!(
        top.state_gas_used.is_some(),
        "top frame must carry stateGasUsed on Amsterdam+"
    );
    assert!(
        top.gas_refund.is_some(),
        "top frame must carry gasRefund on Amsterdam+"
    );
    assert_eq!(top.calls.len(), 1, "one CALL sub-frame expected");

    let sub = &top.calls[0];
    assert!(
        sub.regular_gas_used.is_none(),
        "sub-frame must not carry regularGasUsed"
    );
    assert!(
        sub.state_gas_used.is_none(),
        "sub-frame must not carry stateGasUsed"
    );
    assert!(
        sub.gas_refund.is_none(),
        "sub-frame must not carry gasRefund"
    );

    let json = serde_json::to_value(top).expect("serialize");
    let sub_json = &json["calls"][0];
    let sub_obj = sub_json.as_object().expect("sub-frame is an object");
    for absent in ["regularGasUsed", "stateGasUsed", "gasRefund"] {
        assert!(
            !sub_obj.contains_key(absent),
            "{absent} must be omitted on a sub-frame"
        );
    }
}
