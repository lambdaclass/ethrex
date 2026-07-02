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
