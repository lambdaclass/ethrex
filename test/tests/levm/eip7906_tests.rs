//! EIP-7906: TXTRACE (0xB6), EVENTDATACOPY (0xB7), and TXDIFF (0xB8) on
//! hegota-devnet (renumbered above EIP-8272's RECENTROOTREFLOAD at 0xB5).
//!
//! Per EIP-7906 spec PR #11829, these three introspection opcodes execute ONLY
//! inside a POST_TX frame (FrameMode::PostTx) of an EIP-8141 frame transaction;
//! anywhere else (legacy/EIP-1559 txs, or any other frame mode) they
//! exceptional-halt. A POST_TX frame runs read-only (STATICCALL from
//! ENTRY_POINT) as a trailing suffix of the frame list; if its subtree REVERTs,
//! the whole transaction body reverts and the tx is invalid (#11829).
//!
//! These integration tests therefore drive the opcodes through POST_TX frames
//! and surface results via "assert-or-revert": the POST_TX bytecode computes a
//! value, compares it to the expected word, and REVERTs on mismatch — so a
//! VALID tx means every assertion held. The diff-computation detail (sorting,
//! before/after values, exclusions) is unit-tested directly against the pure
//! functions in `crates/vm/levm/src/opcode_handlers/tx_trace.rs`.
//!
//! Stack orders (operand popped FIRST listed first):
//!   TXTRACE        [in2, param]               -> push param, then in2
//!   EVENTDATACOPY  [event_index, memOff, dataOff, length]
//!   TXDIFF         [param, address, in3]      -> push in3, then address, then param

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, Code, EIP1559Transaction, Fork, Frame, FrameMode, FrameTransaction,
    Transaction, TxKind,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::{ExecutionReport, TxValidationError, VMError};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Opcode bytes ====================

// Renumbered to 0xB6/0xB7/0xB8 on hegota-devnet so EIP-8272 owns 0xB5.
const TXTRACE: u8 = 0xB6;
const EVENTDATACOPY: u8 = 0xB7;
const TXDIFF: u8 = 0xB8;

// EVM opcodes used by the bytecode builders.
const PUSH1: u8 = 0x60;
const PUSH2: u8 = 0x61;
const PUSH20: u8 = 0x73;
const PUSH32: u8 = 0x7f;
const MSTORE: u8 = 0x52;
const MLOAD: u8 = 0x51;
const SSTORE: u8 = 0x55;
const STOP: u8 = 0x00;
const LOG0: u8 = 0xa0;
const REVERT: u8 = 0xfd;
const JUMPDEST: u8 = 0x5b;
const JUMPI: u8 = 0x57;
const EQ: u8 = 0x14;
const APPROVE: u8 = 0xAA;

// ==================== Harness constants ====================

const HARNESS_CHAIN_ID: u64 = 1;
/// Funded sender for frame txs. Must be non-zero (static-constraint check).
const FUNDED_SENDER: Address = Address::repeat_byte(0xAA);
const AUTO_SEED_SENDER_BALANCE: U256 = U256::MAX;
const HARNESS_BASE_FEE: u64 = 1;

/// APPROVE(scope=3): sets payer AND sender_approved when run in a VERIFY frame
/// whose target is the tx sender. Mints a minimal valid frame tx.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, APPROVE];
/// APPROVE(scope=1): the frame's target becomes the gas payer.
const APPROVE_PAYMENT_CODE: &[u8] = &[0x60, 0x01, 0x60, 0x00, 0x60, 0x00, APPROVE];
/// APPROVE(scope=2): sender (execution) approval; the frame target must be the sender.
const APPROVE_EXECUTION_CODE: &[u8] = &[0x60, 0x02, 0x60, 0x00, 0x60, 0x00, APPROVE];

const ASSERTION_ADDR: u64 = 0x7906;
const WRITER_ADDR: u64 = 0x7907;

// ==================== Account seeding ====================

/// A seeded account: address, balance, nonce, code, and prestate storage.
struct Seed {
    addr: Address,
    balance: U256,
    nonce: u64,
    code: Vec<u8>,
    storage: Vec<(u64, u64)>,
}

impl Seed {
    fn new(addr: Address, code: Vec<u8>) -> Self {
        Self {
            addr,
            balance: U256::zero(),
            nonce: 0,
            code,
            storage: Vec::new(),
        }
    }
    fn balance(mut self, b: U256) -> Self {
        self.balance = b;
        self
    }
    fn storage(mut self, slots: &[(u64, u64)]) -> Self {
        self.storage = slots.to_vec();
        self
    }
}

fn seeded_db(seeds: &[Seed]) -> GeneralizedDatabase {
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());

    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    for seed in seeds {
        let storage: FxHashMap<H256, U256> = seed
            .storage
            .iter()
            .map(|(k, v)| (H256::from_low_u64_be(*k), U256::from(*v)))
            .collect();
        cache.insert(
            seed.addr,
            Account::new(
                seed.balance,
                Code::from_bytecode(Bytes::from(seed.code.clone()), &NativeCrypto),
                seed.nonce,
                storage,
            ),
        );
    }
    GeneralizedDatabase::new_with_account_state(Arc::new(store), cache)
}

// ==================== Frame-tx execution ====================

fn frame_tx_env(tx: &FrameTransaction) -> Environment {
    Environment {
        origin: tx.sender,
        gas_limit: tx.total_gas_limit(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(HARNESS_CHAIN_ID),
        base_fee_per_gas: U256::from(HARNESS_BASE_FEE),
        gas_price: U256::from(tx.max_fee_per_gas),
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    }
}

fn frame_tx_with_frames(frames: Vec<Frame>) -> FrameTransaction {
    FrameTransaction {
        chain_id: HARNESS_CHAIN_ID,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender: FUNDED_SENDER,
        frames,
        signatures: Vec::new(),
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: HARNESS_BASE_FEE + 1_000,
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: Vec::new(),
        recent_root_references: Vec::new(),
        inner_hash: Default::default(),
        cached_canonical: Default::default(),
    }
}

/// Run `tx` against `seeds`, auto-seeding the sender with `APPROVE_BOTH_CODE` if
/// the caller did not provide it. Returns the execution result.
fn run_frame_tx(seeds: Vec<Seed>, tx: FrameTransaction) -> Result<ExecutionReport, VMError> {
    let mut seeds = seeds;
    if !seeds.iter().any(|s| s.addr == tx.sender) {
        seeds.push(
            Seed::new(tx.sender, APPROVE_BOTH_CODE.to_vec()).balance(AUTO_SEED_SENDER_BALANCE),
        );
    }
    let mut db = seeded_db(&seeds);
    let env = frame_tx_env(&tx);
    let transaction = Transaction::FrameTransaction(tx);
    let mut vm = VM::new(
        env,
        &mut db,
        &transaction,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
    )
    .expect("VM::new should succeed for a frame tx");
    vm.execute()
}

fn verify_frame(target: Address) -> Frame {
    Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(target),
        gas_limit: 200_000,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn default_frame(target: Address) -> Frame {
    Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0x00,
        target: Some(target),
        gas_limit: 2_000_000,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

/// A POST_TX frame (read-only; STATICCALL from ENTRY_POINT). A revert in its
/// subtree reverts the whole tx body and invalidates the tx.
fn posttx_frame(target: Address) -> Frame {
    Frame {
        mode: u8::from(FrameMode::PostTx),
        flags: 0x00,
        target: Some(target),
        gas_limit: 400_000,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

// ==================== Bytecode builders ====================

fn push1(v: u8) -> Vec<u8> {
    vec![PUSH1, v]
}

fn push2(v: usize) -> Vec<u8> {
    let v = u16::try_from(v).expect("push2 operand fits in u16");
    vec![PUSH2, (v >> 8) as u8, (v & 0xff) as u8]
}

fn push20(addr: Address) -> Vec<u8> {
    let mut out = vec![PUSH20];
    out.extend_from_slice(addr.as_bytes());
    out
}

fn push32(v: U256) -> Vec<u8> {
    let mut out = vec![PUSH32];
    out.extend_from_slice(&v.to_big_endian());
    out
}

fn word(addr: Address) -> U256 {
    let mut buf = [0u8; 32];
    buf[12..].copy_from_slice(addr.as_bytes());
    U256::from_big_endian(&buf)
}

/// `PUSH1 param ; PUSH1 in2 ; TXTRACE` — leaves the trace word on the stack.
fn txtrace_compute(param: u8, in2: u8) -> Vec<u8> {
    let mut c = push1(param);
    c.extend(push1(in2));
    c.push(TXTRACE);
    c
}

/// `PUSH32 in3 ; PUSH20 address ; PUSH1 param ; TXDIFF` — leaves the diff word
/// on the stack (param popped first, then address, then in3).
fn txdiff_compute(param: u8, addr: Address, in3: U256) -> Vec<u8> {
    let mut c = push32(in3);
    c.extend(push20(addr));
    c.extend(push1(param));
    c.push(TXDIFF);
    c
}

/// Assemble a sequence of `(compute, expected)` checks into POST_TX assertion
/// bytecode: each check computes a word, compares it to `expected`, and REVERTs
/// on mismatch; if all match, the code STOPs (assertion holds). Jump targets are
/// absolute offsets patched with `PUSH2`.
fn assert_all_eq(checks: &[(Vec<u8>, U256)]) -> Vec<u8> {
    let mut code = Vec::new();
    for (compute, expected) in checks {
        code.extend_from_slice(compute); // [val]
        code.extend(push32(*expected)); // [val, exp]
        code.push(EQ); // [val == exp]
        // PUSH2 skip(3) ; JUMPI(1) ; PUSH1 0(2) ; PUSH1 0(2) ; REVERT(1) ; JUMPDEST(1)
        let skip = code.len() + 3 + 1 + 2 + 2 + 1; // offset of the JUMPDEST below
        code.extend(push2(skip));
        code.push(JUMPI);
        code.extend(push1(0));
        code.extend(push1(0));
        code.push(REVERT);
        code.push(JUMPDEST);
    }
    code.push(STOP);
    code
}

/// Writer body: `SSTORE val@slot ; STOP`.
fn sstore_code(slot: u8, val: U256) -> Vec<u8> {
    let mut c = push32(val);
    c.extend(push1(slot));
    c.push(SSTORE);
    c.push(STOP);
    c
}

/// Writer body: one `SSTORE val@slot` per entry, then `STOP`.
fn multi_sstore_code(writes: &[(u8, U256)]) -> Vec<u8> {
    let mut c = Vec::new();
    for (slot, val) in writes {
        c.extend(push32(*val));
        c.extend(push1(*slot));
        c.push(SSTORE);
    }
    c.push(STOP);
    c
}

// ==================== Test helpers ====================

fn assertion_addr() -> Address {
    Address::from_low_u64_be(ASSERTION_ADDR)
}

fn writer_addr() -> Address {
    Address::from_low_u64_be(WRITER_ADDR)
}

/// Run `[VERIFY(sender)->APPROVE(3), <body frames>, POST_TX(assertion)]`.
/// `seeds` must seed every body/assertion contract; the sender is auto-seeded.
fn run_posttx(
    mut seeds: Vec<Seed>,
    body_frames: Vec<Frame>,
    assertion_code: Vec<u8>,
) -> Result<ExecutionReport, VMError> {
    seeds.push(Seed::new(assertion_addr(), assertion_code));
    let mut frames = vec![verify_frame(FUNDED_SENDER)];
    frames.extend(body_frames);
    frames.push(posttx_frame(assertion_addr()));
    run_frame_tx(seeds, frame_tx_with_frames(frames))
}

fn assert_invalid(result: Result<ExecutionReport, VMError>) {
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                TxValidationError::InvalidFrameTransaction
            ))
        ),
        "expected InvalidFrameTransaction, got {result:?}"
    );
}

// ==================== POST_TX gating + whole-body revert ====================

#[test]
fn txtrace_passes_inside_posttx_frame() {
    // No body writes, so the whole-tx storage-change count (TXTRACE 0x01) is 0.
    let code = assert_all_eq(&[(txtrace_compute(0x01, 0x00), U256::zero())]);
    assert!(
        run_posttx(vec![], vec![], code).is_ok(),
        "a matching POST_TX assertion must keep the tx valid"
    );
}

#[test]
fn posttx_revert_invalidates_whole_tx() {
    // Same trace (count == 0) but the assertion expects 1 -> REVERT -> tx invalid.
    let code = assert_all_eq(&[(txtrace_compute(0x01, 0x00), U256::one())]);
    assert_invalid(run_posttx(vec![], vec![], code));
}

#[test]
fn txtrace_halts_in_default_frame() {
    // TXTRACE in a DEFAULT (non-POST_TX) frame must halt. Frame 0 approves a
    // payer, so the tx stays valid; the DEFAULT frame reverting is the gating
    // proof. `txtrace_passes_inside_posttx_frame` runs the SAME opcode in a
    // POST_TX frame successfully, so the pair establishes the gating.
    let seeds = vec![Seed::new(
        writer_addr(),
        vec![0x60, 0x01, 0x60, 0x00, TXTRACE, STOP], // PUSH1 1; PUSH1 0; TXTRACE; STOP
    )];
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        default_frame(writer_addr()),
    ]);
    let report = run_frame_tx(seeds, tx).expect("tx valid: payer approved in frame 0");
    assert!(
        !report.is_success(),
        "TXTRACE outside a POST_TX frame must halt, reverting the DEFAULT frame: {:?}",
        report.result
    );
}

#[test]
fn txtrace_halts_in_normal_tx() {
    // The introspection opcodes are not valid in a normal EIP-1559 tx (no frame
    // context); TXTRACE must exceptional-halt there.
    let code = vec![0x60, 0x01, 0x60, 0x00, TXTRACE, STOP];
    let contract = Address::from_low_u64_be(0x3000);
    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    cache.insert(
        contract,
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(code), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    let sender = Address::from_low_u64_be(0x1000);
    cache.insert(
        sender,
        Account::new(
            U256::from(10u64).pow(18.into()),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(store), cache);
    let env = Environment {
        origin: sender,
        gas_limit: 1_000_000,
        block_gas_limit: 2_000_000,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(HARNESS_CHAIN_ID),
        gas_price: U256::from(10u64),
        base_fee_per_gas: U256::from(1u64),
        tx_max_fee_per_gas: Some(U256::from(10u64)),
        ..Default::default()
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        gas_limit: 1_000_000,
        max_fee_per_gas: 10,
        ..Default::default()
    });
    let report = {
        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &NativeCrypto,
        )
        .expect("VM::new");
        vm.execute().expect("execute returns Ok even on halt")
    };
    assert!(
        !report.is_success(),
        "TXTRACE in a normal tx must halt: {:?}",
        report.result
    );
}

// ==================== TXTRACE through POST_TX ====================

#[test]
fn txtrace_observes_body_storage_writes() {
    // Body frame writes two distinct slots; the POST_TX assertion observes the
    // whole-tx storage-change count == 2.
    let writer = Seed::new(
        writer_addr(),
        multi_sstore_code(&[(0x01, U256::from(42)), (0x02, U256::from(43))]),
    );
    let code = assert_all_eq(&[(txtrace_compute(0x01, 0x00), U256::from(2))]);
    assert!(
        run_posttx(vec![writer], vec![default_frame(writer_addr())], code).is_ok(),
        "POST_TX must observe both committed body writes"
    );
}

#[test]
fn txtrace_undefined_param_halts() {
    // An undefined TXTRACE param halts -> the POST_TX frame reverts -> tx invalid.
    let code = {
        let mut c = txtrace_compute(0x7F, 0x00); // 0x7F is not a defined param
        c.push(STOP);
        c
    };
    assert_invalid(run_posttx(vec![], vec![], code));
}

#[test]
fn txtrace_nonzero_in2_on_scalar_param_halts() {
    // param 0x01 (slot-change count) is scalar: a non-zero in2 must halt.
    let code = {
        let mut c = txtrace_compute(0x01, 0x01);
        c.push(STOP);
        c
    };
    assert_invalid(run_posttx(vec![], vec![], code));
}

#[test]
fn txtrace_gas_payer_and_precharge() {
    // Frame layout: VERIFY(paymaster)->APPROVE(1) sets payer; VERIFY(sender)->
    // APPROVE(2) approves the sender; POST_TX asserts gas_payer (0x15) == paymaster
    // and gas_pre_charge (0x14) == total_gas_limit * max_fee_per_gas.
    let paymaster = Address::from_low_u64_be(0x9A);
    let seeds = vec![
        Seed::new(paymaster, APPROVE_PAYMENT_CODE.to_vec())
            .balance(U256::from(10u64).pow(18.into())),
        Seed::new(FUNDED_SENDER, APPROVE_EXECUTION_CODE.to_vec()).balance(AUTO_SEED_SENDER_BALANCE),
        Seed::new(assertion_addr(), Vec::new()), // filled below
    ];

    let mut frames = vec![verify_frame(paymaster), verify_frame(FUNDED_SENDER)];
    frames.push(posttx_frame(assertion_addr()));
    let tx = frame_tx_with_frames(frames);

    let expected_precharge = U256::from(tx.total_gas_limit()) * U256::from(tx.max_fee_per_gas);
    let assertion = assert_all_eq(&[
        (txtrace_compute(0x15, 0x00), word(paymaster)),
        (txtrace_compute(0x14, 0x00), expected_precharge),
    ]);

    // Rebuild seeds with the assertion code now known.
    let seeds = seeds
        .into_iter()
        .map(|s| {
            if s.addr == assertion_addr() {
                Seed::new(assertion_addr(), assertion.clone())
            } else {
                s
            }
        })
        .collect();

    assert!(
        run_frame_tx(seeds, tx).is_ok(),
        "gas_payer must be the paymaster and gas_pre_charge must match the formula"
    );
}

// ==================== EVENTDATACOPY through POST_TX ====================

/// Body that emits LOG0 with one 32-byte data word: `MSTORE w@0 ; LOG0(0,32) ; STOP`.
fn log_word_code(w: U256) -> Vec<u8> {
    let mut c = push32(w);
    c.extend(push1(0));
    c.push(MSTORE);
    c.extend(push1(32)); // size
    c.extend(push1(0)); // offset
    c.push(LOG0);
    c.push(STOP);
    c
}

#[test]
fn eventdatacopy_copies_event_data_in_posttx() {
    let data = U256::from(0xABCDEFu64);
    let emitter = Seed::new(writer_addr(), log_word_code(data));

    // POST_TX: EVENTDATACOPY(event 0, mem 0, dataOff 0, len 32); MLOAD(0); compare.
    // Stack push order (bottom-up): length, dataOffset, memOffset, event_index.
    let mut compute = push1(32); // length
    compute.extend(push1(0)); // dataOffset
    compute.extend(push1(0)); // memOffset
    compute.extend(push1(0)); // event_index
    compute.push(EVENTDATACOPY);
    compute.extend(push1(0)); // MLOAD offset
    compute.push(MLOAD); // -> copied word
    let code = assert_all_eq(&[(compute, data)]);

    assert!(
        run_posttx(vec![emitter], vec![default_frame(writer_addr())], code).is_ok(),
        "EVENTDATACOPY in POST_TX must copy the body's emitted event data"
    );
}

#[test]
fn eventdatacopy_halts_in_default_frame() {
    // EVENTDATACOPY in a DEFAULT frame must halt (gating).
    let mut body = log_word_code(U256::one());
    body.truncate(body.len() - 1); // drop STOP
    // EVENTDATACOPY(0,0,0,0) then STOP.
    body.extend(push1(0));
    body.extend(push1(0));
    body.extend(push1(0));
    body.extend(push1(0));
    body.push(EVENTDATACOPY);
    body.push(STOP);
    let seeds = vec![Seed::new(writer_addr(), body)];
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        default_frame(writer_addr()),
    ]);
    let report = run_frame_tx(seeds, tx).expect("tx valid: payer approved");
    assert!(
        !report.is_success(),
        "EVENTDATACOPY outside POST_TX must halt: {:?}",
        report.result
    );
}

// ==================== TXDIFF through POST_TX ====================

#[test]
fn txdiff_slot_before_after_for_modified_slot() {
    // Writer's prestate slot 5 = 10; body changes it to 99. POST_TX asserts
    // slot_before (0x00) == 10 and slot_after (0x01) == 99.
    let writer = Seed::new(writer_addr(), sstore_code(0x05, U256::from(99))).storage(&[(5, 10)]);
    let slot5 = U256::from(5);
    let code = assert_all_eq(&[
        (txdiff_compute(0x00, writer_addr(), slot5), U256::from(10)),
        (txdiff_compute(0x01, writer_addr(), slot5), U256::from(99)),
    ]);
    assert!(
        run_posttx(vec![writer], vec![default_frame(writer_addr())], code).is_ok(),
        "TXDIFF must report the prestate (before) and live (after) slot values"
    );
}

#[test]
fn txdiff_unmodified_slot_reads_live_value_both_ways() {
    // Writer's prestate slot 7 = 123, never touched by the body. TXDIFF before
    // and after must both equal the live value (123).
    let writer = Seed::new(writer_addr(), vec![STOP]).storage(&[(7, 123)]);
    let slot7 = U256::from(7);
    let code = assert_all_eq(&[
        (txdiff_compute(0x00, writer_addr(), slot7), U256::from(123)),
        (txdiff_compute(0x01, writer_addr(), slot7), U256::from(123)),
    ]);
    assert!(
        run_posttx(vec![writer], vec![default_frame(writer_addr())], code).is_ok(),
        "TXDIFF on an unmodified slot must read the live value for both before and after"
    );
}

#[test]
fn txdiff_codehash_for_deployed_and_undeployed() {
    // A seeded contract (untouched) -> codehash_before == codehash_after == its
    // code hash. An undeployed address -> empty Keccak hash both ways.
    let seeded_code = vec![0x60, 0x00, STOP];
    let contract = Seed::new(writer_addr(), seeded_code.clone());
    let code_hash = Code::from_bytecode(Bytes::from(seeded_code), &NativeCrypto).hash;
    let empty_hash = *ethrex_common::constants::EMPTY_KECCAK_HASH;
    let undeployed = Address::from_low_u64_be(0xDEAD);

    let code = assert_all_eq(&[
        (
            txdiff_compute(0x04, writer_addr(), U256::zero()),
            U256::from_big_endian(code_hash.as_bytes()),
        ),
        (
            txdiff_compute(0x05, writer_addr(), U256::zero()),
            U256::from_big_endian(code_hash.as_bytes()),
        ),
        (
            txdiff_compute(0x04, undeployed, U256::zero()),
            U256::from_big_endian(empty_hash.as_bytes()),
        ),
    ]);
    assert!(
        run_posttx(vec![contract], vec![], code).is_ok(),
        "TXDIFF codehash must report a deployed account's hash and the empty hash for undeployed"
    );
}

#[test]
fn txdiff_balance_unmodified_reads_live_value() {
    // A seeded, untouched account -> balance_before == balance_after == seeded.
    let acct = Seed::new(writer_addr(), vec![STOP]).balance(U256::from(777u64));
    let code = assert_all_eq(&[
        (
            txdiff_compute(0x02, writer_addr(), U256::zero()),
            U256::from(777),
        ),
        (
            txdiff_compute(0x03, writer_addr(), U256::zero()),
            U256::from(777),
        ),
    ]);
    assert!(
        run_posttx(vec![acct], vec![], code).is_ok(),
        "TXDIFF balance on an unmodified account must read the live balance both ways"
    );
}

#[test]
fn txdiff_nonzero_in3_on_balance_param_halts() {
    // Balance params are scalar: a non-zero in3 (slot key) must halt -> tx invalid.
    let code = {
        let mut c = txdiff_compute(0x02, writer_addr(), U256::one());
        c.push(STOP);
        c
    };
    let acct = Seed::new(writer_addr(), vec![STOP]).balance(U256::from(5u64));
    assert_invalid(run_posttx(vec![acct], vec![], code));
}

#[test]
fn txdiff_halts_in_default_frame() {
    // TXDIFF in a DEFAULT frame must halt (gating).
    let mut body = txdiff_compute(0x03, writer_addr(), U256::zero());
    body.push(STOP);
    let seeds = vec![Seed::new(writer_addr(), body)];
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        default_frame(writer_addr()),
    ]);
    let report = run_frame_tx(seeds, tx).expect("tx valid: payer approved");
    assert!(
        !report.is_success(),
        "TXDIFF outside POST_TX must halt: {:?}",
        report.result
    );
}

// ==================== Fork gating ====================

/// Execute `code` as a normal EIP-1559 tx at `fork` and report success.
fn run_normal_tx(code: Vec<u8>, fork: Fork) -> bool {
    let contract = Address::from_low_u64_be(0x3000);
    let sender = Address::from_low_u64_be(0x1000);
    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    cache.insert(
        contract,
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(code), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    cache.insert(
        sender,
        Account::new(
            U256::from(10u64).pow(18.into()),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(store), cache);
    let env = Environment {
        origin: sender,
        gas_limit: 1_000_000,
        block_gas_limit: 2_000_000,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        chain_id: U256::from(HARNESS_CHAIN_ID),
        gas_price: U256::from(10u64),
        base_fee_per_gas: U256::from(1u64),
        tx_max_fee_per_gas: Some(U256::from(10u64)),
        ..Default::default()
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        gas_limit: 1_000_000,
        max_fee_per_gas: 10,
        ..Default::default()
    });
    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
    )
    .expect("VM::new");
    vm.execute()
        .expect("execute returns Ok even on halt")
        .is_success()
}

#[test]
fn txtrace_invalid_before_hegota() {
    // 0xB6 is not installed before Hegota -> undefined opcode -> halt.
    let code = vec![0x60, 0x15, 0x60, 0x00, TXTRACE, STOP];
    assert!(
        !run_normal_tx(code, Fork::Amsterdam),
        "TXTRACE must be invalid before Hegota"
    );
}

#[test]
fn eventdatacopy_invalid_before_hegota() {
    let mut code = log_word_code(U256::one());
    code.truncate(code.len() - 1); // drop STOP
    code.extend(push1(0));
    code.extend(push1(0));
    code.extend(push1(0));
    code.extend(push1(0));
    code.push(EVENTDATACOPY);
    code.push(STOP);
    assert!(
        !run_normal_tx(code, Fork::Amsterdam),
        "EVENTDATACOPY must be invalid before Hegota"
    );
}

#[test]
fn txdiff_invalid_before_hegota() {
    let mut code = txdiff_compute(0x03, writer_addr(), U256::zero());
    code.push(STOP);
    assert!(
        !run_normal_tx(code, Fork::Amsterdam),
        "TXDIFF must be invalid before Hegota"
    );
}
