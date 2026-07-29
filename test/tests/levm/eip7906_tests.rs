//! EIP-7906: TXTRACE (0xB6), EVENTDATACOPY (0xB7), and TXDIFF (0xB8)
//! introspection opcodes, shifted one byte up from the spec's 0xB5/0xB6/0xB7 so
//! that EIP-8272's RECENTROOTREFLOAD owns 0xB5 (see docs/hegota-devnet.md).
//!
//! Per EIP-7906 spec PR #11829, these three introspection opcodes execute ONLY
//! inside a POST_TX frame (FrameMode::PostTx) of an EIP-8141 frame transaction;
//! anywhere else (legacy/EIP-1559 txs, or any other frame mode) they
//! exceptional-halt. A POST_TX frame runs read-only (STATICCALL from
//! ENTRY_POINT) as a trailing suffix of the frame list; if its subtree REVERTs,
//! the transaction's execution BODY reverts while the transaction itself stays
//! VALID — included, with a failed (`status = 0`) receipt and the validation
//! prefix (notably the APPROVE gas payment) permanently committed.
//!
//! These integration tests therefore drive the opcodes through POST_TX frames
//! and surface results via "assert-or-revert": the POST_TX bytecode computes a
//! value, compares it to the expected word, and REVERTs on mismatch — so a
//! SUCCEEDING tx means every assertion held, and a failed-status tx means one
//! fired. The diff-computation detail (sorting,
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
    Account, BlockHeader, Code, EIP1559Transaction, FRAME_RECEIPT_STATUS_FAILURE, Fork, Frame,
    FrameMode, FrameTransaction, Transaction, TxKind,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::{ExecutionReport, VMError};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Opcode bytes ====================

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
const OTHER_ADDR: u64 = 0x7908;

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
    run_frame_tx_with_db(seeds, tx).0
}

/// As [`run_frame_tx`], but also hands back the database so a caller can inspect
/// the post-state. Needed for EIP-7906's partial revert, whose whole point is
/// which state survives — something no assertion inside the reverted frame can
/// observe.
fn run_frame_tx_with_db(
    seeds: Vec<Seed>,
    tx: FrameTransaction,
) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    let mut seeds = seeds;
    if !seeds.iter().any(|s| s.addr == tx.sender) {
        seeds.push(
            Seed::new(tx.sender, APPROVE_BOTH_CODE.to_vec()).balance(AUTO_SEED_SENDER_BALANCE),
        );
    }
    let mut db = seeded_db(&seeds);
    let env = frame_tx_env(&tx);
    let transaction = Transaction::FrameTransaction(tx);
    let result = {
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
    };
    (result, db)
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

/// A VERIFY frame declaring only `APPROVE_PAYMENT`. A frame that declares
/// `APPROVE_EXECUTION` must target `tx.sender` (EIP-8141 §APPROVE: that scope is
/// the sender authorizing execution on its own behalf), so a third-party
/// paymaster frame may declare payment scope only.
fn pay_frame(target: Address) -> Frame {
    Frame {
        flags: 0x01,
        ..verify_frame(target)
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

/// A second body contract, for the per-address TXDIFF views — which only mean
/// anything when more than one account appears in the transaction's diff.
fn other_addr() -> Address {
    Address::from_low_u64_be(OTHER_ADDR)
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

/// EIP-7906: a reverted POST_TX frame reverts the execution body and nothing
/// more. The transaction "remains valid, is included in the block, and generates
/// a receipt with a failed status (`status = 0`)", and the validation prefix —
/// including the APPROVE gas payment — stays committed. Asserts that shape:
/// `Ok`, failed top-level status, payer still set, POST_TX frame recorded failed.
fn assert_posttx_reverted(result: Result<ExecutionReport, VMError>) {
    let report = result.unwrap_or_else(|e| {
        panic!("a POST_TX revert must not invalidate the transaction, got Err({e:?})")
    });
    assert!(
        !report.is_success(),
        "expected a failed top-level status, got {report:?}"
    );
    assert!(
        report.payer_address.is_some(),
        "the prefix APPROVE must stay committed, got {report:?}"
    );
    let frame_results = report
        .frame_results
        .as_ref()
        .expect("a frame transaction must report per-frame results");
    let (status, ..) = frame_results
        .last()
        .expect("the POST_TX frame must be recorded");
    assert_eq!(
        *status, FRAME_RECEIPT_STATUS_FAILURE,
        "the POST_TX frame must be recorded as failed, got {report:?}"
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
fn posttx_revert_fails_tx_without_invalidating_it() {
    // Same trace (count == 0) but the assertion expects 1 -> REVERT. The body is
    // reverted and the receipt reports status 0; the transaction stays valid.
    let code = assert_all_eq(&[(txtrace_compute(0x01, 0x00), U256::one())]);
    assert_posttx_reverted(run_posttx(vec![], vec![], code));
}

/// The core EIP-7906 partial-revert shape, in one test: the body's storage write
/// is rewound to its prestate value, while the validation prefix's effect — the
/// APPROVE gas payment — is permanently committed and the payer is charged.
///
/// Excluding the transaction instead (rolling the payment back too) would let an
/// attacker burn a block's worth of execution and revert for free, which is why
/// the spec requires the prefix to commit (§Receipt Representation and Anti-DoS).
#[test]
fn posttx_revert_rewinds_the_body_but_commits_the_prefix() {
    const SLOT: u8 = 0x01;
    const PRESTATE_VALUE: u64 = 7;

    // Body frame overwrites SLOT; the assertion then demands a whole-tx
    // storage-change count of 0, which is false, so it reverts.
    let writer = Seed::new(writer_addr(), multi_sstore_code(&[(SLOT, U256::from(42))]))
        .storage(&[(u64::from(SLOT), PRESTATE_VALUE)]);
    let assertion = Seed::new(
        assertion_addr(),
        assert_all_eq(&[(txtrace_compute(0x01, 0x00), U256::zero())]),
    );
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        default_frame(writer_addr()),
        posttx_frame(assertion_addr()),
    ]);

    let (result, db) = run_frame_tx_with_db(vec![writer, assertion], tx);
    assert_posttx_reverted(result);

    let writer_account = db
        .current_accounts_state
        .get(&writer_addr())
        .expect("the writer account must be cached");
    assert_eq!(
        writer_account
            .storage
            .get(&H256::from_low_u64_be(u64::from(SLOT)))
            .copied(),
        Some(U256::from(PRESTATE_VALUE)),
        "the body's storage write must be rewound to its prestate value"
    );

    let sender_balance = db
        .current_accounts_state
        .get(&FUNDED_SENDER)
        .expect("the sender account must be cached")
        .info
        .balance;
    assert!(
        sender_balance < AUTO_SEED_SENDER_BALANCE,
        "the payer must stay charged for the gas consumed up to the revert; \
         balance was {sender_balance}"
    );
}

/// Logs emitted by the body go with the body's state; a POST_TX revert must leave
/// none of them in the receipt.
#[test]
fn posttx_revert_consumes_the_keyed_nonce() {
    // EIP-8250 interplay, and the reason the anti-DoS property actually holds.
    //
    // `consume_keyed_nonces` runs inside the APPROVE handler, so it happens in the
    // validation prefix — which a POST_TX revert commits. The nonce is therefore
    // SPENT even though the body was rewound, and the transaction is not
    // replayable. Under the previous exclude-the-transaction behaviour the nonce
    // was rolled back with everything else, so an attacker could resubmit the same
    // transaction indefinitely and make a builder re-execute it for free each time.
    //
    // Nonce key 0 is the account's linear nonce, so it is readable straight off the
    // sender account.
    let code = assert_all_eq(&[(txtrace_compute(0x01, 0x00), U256::one())]);
    let assertion = Seed::new(assertion_addr(), code);
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        posttx_frame(assertion_addr()),
    ]);
    let sender_nonce_before = tx.nonce_seq;

    let (result, db) = run_frame_tx_with_db(vec![assertion], tx);
    assert_posttx_reverted(result);

    let sender_nonce_after = db
        .current_accounts_state
        .get(&FUNDED_SENDER)
        .expect("the sender account must be cached")
        .info
        .nonce;
    assert_eq!(
        sender_nonce_after,
        sender_nonce_before + 1,
        "the keyed nonce consumed in the prefix must survive a POST_TX revert, \
         otherwise the transaction stays replayable"
    );
}

#[test]
fn posttx_revert_drops_body_logs() {
    // LOG0 with a zero-length payload, then STOP.
    let emitter_code = vec![PUSH1, 0x00, PUSH1, 0x00, LOG0, STOP];
    let emitter = Seed::new(writer_addr(), emitter_code);
    let assertion = Seed::new(
        assertion_addr(),
        // One event was emitted, so asserting a count of 0 reverts.
        assert_all_eq(&[(txtrace_compute(0x0C, 0x00), U256::zero())]),
    );
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        default_frame(writer_addr()),
        posttx_frame(assertion_addr()),
    ]);

    let (result, _db) = run_frame_tx_with_db(vec![emitter, assertion], tx);
    let report = result.expect("a POST_TX revert must not invalidate the transaction");
    assert!(
        !report.is_success(),
        "expected a failed status, got {report:?}"
    );
    assert!(
        report.logs.is_empty(),
        "the body's logs must be reverted with its state, got {:?}",
        report.logs
    );
}

#[test]
fn approve_halts_inside_posttx_frame() {
    // EIP-7906: APPROVE is forbidden in a POST_TX (read-only assertion) frame. It
    // must exceptional-halt, which fails the POST_TX frame and so reverts the body.
    // PUSH1 2 (APPROVE_EXECUTION scope); PUSH1 0; PUSH1 0; APPROVE.
    let code = vec![0x60, 0x02, 0x60, 0x00, 0x60, 0x00, APPROVE];
    assert_posttx_reverted(run_posttx(vec![], vec![], code));
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
    assert_posttx_reverted(run_posttx(vec![], vec![], code));
}

#[test]
fn txtrace_nonzero_in2_on_scalar_param_halts() {
    // param 0x01 (slot-change count) is scalar: a non-zero in2 must halt.
    let code = {
        let mut c = txtrace_compute(0x01, 0x01);
        c.push(STOP);
        c
    };
    assert_posttx_reverted(run_posttx(vec![], vec![], code));
}

#[test]
fn txtrace_gas_payer_and_precharge() {
    // Frame layout: VERIFY(sender)->APPROVE(2) approves the sender, then
    // VERIFY(paymaster)->APPROVE(1) sets a third-party payer; POST_TX asserts
    // gas_payer (0x15) == paymaster and gas_pre_charge (0x14) ==
    // total_gas_limit * max_fee_per_gas. Execution approval must come first:
    // EIP-8141 §APPROVE reverts an APPROVE_PAYMENT while `sender_approved` is
    // false, which is also the canonical `only_verify` + `pay` prefix order.
    let paymaster = Address::from_low_u64_be(0x9A);
    let seeds = vec![
        Seed::new(paymaster, APPROVE_PAYMENT_CODE.to_vec())
            .balance(U256::from(10u64).pow(18.into())),
        Seed::new(FUNDED_SENDER, APPROVE_EXECUTION_CODE.to_vec()).balance(AUTO_SEED_SENDER_BALANCE),
        Seed::new(assertion_addr(), Vec::new()), // filled below
    ];

    let mut frames = vec![verify_frame(FUNDED_SENDER), pay_frame(paymaster)];
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
fn txdiff_state_params_are_priced_through_the_2929_access_lists() {
    // EIP-7906 §Gas Cost: the state-reading TXDIFF params consult the EIP-2929
    // access lists for their cost and add the slot/address afterwards.
    //
    // Both variants run byte-for-byte identical bytecode shapes (two TXDIFF reads
    // through the same assert-or-revert scaffolding, differing only in a pushed
    // constant), so the gas gap between them isolates the TXDIFF charge alone. The
    // second read is warm in one variant and cold in the other, so the gap must
    // equal the cold/warm SLOAD delta — which proves the access list is both
    // consulted AND updated, neither of which a flat per-call charge would do.
    let slot_read = |slot: u64| {
        (
            txdiff_compute(0x01, writer_addr(), U256::from(slot)),
            U256::zero(),
        )
    };
    let cold_then_warm = run_posttx(vec![], vec![], assert_all_eq(&[slot_read(1), slot_read(1)]))
        .expect("reading an unmodified slot must succeed");
    let cold_then_cold = run_posttx(vec![], vec![], assert_all_eq(&[slot_read(1), slot_read(2)]))
        .expect("reading an unmodified slot must succeed");

    let cold = ethrex_levm::gas_cost::sload(true, Fork::Hegota).unwrap();
    let warm = ethrex_levm::gas_cost::sload(false, Fork::Hegota).unwrap();
    assert_eq!(
        cold_then_cold.gas_used - cold_then_warm.gas_used,
        cold - warm,
        "a repeated TXDIFF slot read must be charged the warm price"
    );

    // Same argument for the account-keyed params, against the account access list.
    let balance_read = |addr: Address| (txdiff_compute(0x03, addr, U256::zero()), U256::zero());
    let warm_second = run_posttx(
        vec![],
        vec![],
        assert_all_eq(&[balance_read(writer_addr()), balance_read(writer_addr())]),
    )
    .expect("reading an untouched balance must succeed");
    let cold_second = run_posttx(
        vec![],
        vec![],
        assert_all_eq(&[balance_read(writer_addr()), balance_read(other_addr())]),
    )
    .expect("reading an untouched balance must succeed");
    assert_eq!(
        cold_second.gas_used - warm_second.gas_used,
        ethrex_levm::gas_cost::balance(true, Fork::Hegota).unwrap()
            - ethrex_levm::gas_cost::balance(false, Fork::Hegota).unwrap(),
        "a repeated TXDIFF account read must be charged the warm price"
    );
}

#[test]
fn txdiff_per_address_view_params_do_not_warm_the_access_lists() {
    // The per-address views are answered from the transaction-local diff, so they
    // are flat-priced and must leave the access lists alone: a view call on an
    // address must NOT make a following balance read warm.
    let view = |addr: Address| (txdiff_compute(0x06, addr, U256::zero()), U256::zero());
    let balance_read = |addr: Address| (txdiff_compute(0x03, addr, U256::zero()), U256::zero());

    // Both variants are a view call followed by a balance read of `writer`; they
    // differ only in WHICH address the view was called on. If the view warmed its
    // argument, viewing `writer` first would make the balance read warm and the two
    // would diverge by the cold/warm delta. Equal totals mean it did not.
    let viewed_same = run_posttx(
        vec![],
        vec![],
        assert_all_eq(&[view(writer_addr()), balance_read(writer_addr())]),
    )
    .expect("per-address view on an untouched address must return 0");
    let viewed_other = run_posttx(
        vec![],
        vec![],
        assert_all_eq(&[view(other_addr()), balance_read(writer_addr())]),
    )
    .expect("per-address view on an untouched address must return 0");

    assert_eq!(
        viewed_same.gas_used, viewed_other.gas_used,
        "a per-address view must not add its address to the EIP-2929 access list"
    );
}

#[test]
fn txdiff_per_address_storage_view_maps_to_global_indices() {
    // Body writes two slots on the writer. The per-address view (0x06 count,
    // 0x07 local -> global) must report 2 entries, and each mapped global index
    // must address that same writer entry in TXTRACE's global slots table (0x06
    // = change_address, 0x07 = slot_key).
    let writer = Seed::new(
        writer_addr(),
        multi_sstore_code(&[(0x01, U256::from(11)), (0x02, U256::from(22))]),
    );
    let writer_word = U256::from_big_endian(writer_addr().as_bytes());
    let code = assert_all_eq(&[
        (
            txdiff_compute(0x06, writer_addr(), U256::zero()),
            U256::from(2),
        ),
        // Local 0 and 1 map to global 0 and 1 (the writer is the only account with
        // slot changes, so its entries head the table).
        (
            txdiff_compute(0x07, writer_addr(), U256::zero()),
            U256::zero(),
        ),
        (
            txdiff_compute(0x07, writer_addr(), U256::one()),
            U256::one(),
        ),
        // Cross-check the mapping against the global table it indexes into.
        (txtrace_compute(0x06, 0x00), writer_word),
        (txtrace_compute(0x07, 0x00), U256::one()),
        (txtrace_compute(0x07, 0x01), U256::from(2)),
    ]);
    assert!(
        run_posttx(vec![writer], vec![default_frame(writer_addr())], code).is_ok(),
        "TXDIFF's per-address storage view must map local indices onto the global table"
    );
}

#[test]
fn txdiff_per_address_view_count_is_zero_for_untouched_address() {
    // An address the transaction never touched has an empty view, not a halt.
    let code = assert_all_eq(&[
        (
            txdiff_compute(0x06, writer_addr(), U256::zero()),
            U256::zero(),
        ),
        (
            txdiff_compute(0x08, writer_addr(), U256::zero()),
            U256::zero(),
        ),
    ]);
    assert!(
        run_posttx(vec![], vec![], code).is_ok(),
        "per-address view counts must be 0 for an untouched address"
    );
}

#[test]
fn txdiff_per_address_view_out_of_range_local_index_halts() {
    // Spec: "If TXDIFF received an invalid local index, i.e. value greater than or
    // equal to the view's count, an exceptional halt occurs." The view is empty
    // here, so local index 0 is already out of range.
    let mut code = txdiff_compute(0x07, writer_addr(), U256::zero());
    code.push(STOP);
    assert_posttx_reverted(run_posttx(vec![], vec![], code));
}

#[test]
fn txdiff_per_address_event_view_maps_interleaved_emission_order() {
    // Two contracts each emit one event, interleaved in the body. The per-address
    // event view must pick out only the target's, mapped to its GLOBAL log index.
    let log0 = vec![PUSH1, 0x00, PUSH1, 0x00, LOG0, STOP];
    let emitter_a = Seed::new(writer_addr(), log0.clone());
    let emitter_b = Seed::new(other_addr(), log0);
    let code = assert_all_eq(&[
        // Two events total; one per emitter.
        (txtrace_compute(0x0C, 0x00), U256::from(2)),
        (
            txdiff_compute(0x08, writer_addr(), U256::zero()),
            U256::one(),
        ),
        (
            txdiff_compute(0x08, other_addr(), U256::zero()),
            U256::one(),
        ),
        // Emitter A ran first, so its single event is global index 0; B's is 1.
        (
            txdiff_compute(0x09, writer_addr(), U256::zero()),
            U256::zero(),
        ),
        (
            txdiff_compute(0x09, other_addr(), U256::zero()),
            U256::one(),
        ),
    ]);
    assert!(
        run_posttx(
            vec![emitter_a, emitter_b],
            vec![default_frame(writer_addr()), default_frame(other_addr()),],
            code
        )
        .is_ok(),
        "TXDIFF's per-address event view must map onto global emission order"
    );
}

#[test]
fn txdiff_account_change_flags_report_each_field() {
    // The writer's balance is untouched and its code unchanged, but the body
    // changes one storage slot -> only the storage bit (0b0100) is set. The
    // assertion contract itself was never modified -> mask 0.
    let writer = Seed::new(writer_addr(), sstore_code(0x01, U256::from(9))).storage(&[(1, 8)]);
    let code = assert_all_eq(&[
        (
            txdiff_compute(0x0A, writer_addr(), U256::zero()),
            U256::from(0b0100u8),
        ),
        (
            txdiff_compute(0x0A, other_addr(), U256::zero()),
            U256::zero(),
        ),
    ]);
    assert!(
        run_posttx(vec![writer], vec![default_frame(writer_addr())], code).is_ok(),
        "account_change_flags must set the storage bit and nothing else"
    );
}

#[test]
fn txdiff_account_change_flags_reject_nonzero_in3() {
    // Param 0x0A is keyed by address alone; `in3` must be 0.
    let mut code = txdiff_compute(0x0A, writer_addr(), U256::one());
    code.push(STOP);
    assert_posttx_reverted(run_posttx(vec![], vec![], code));
}

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
    assert_posttx_reverted(run_posttx(vec![acct], vec![], code));
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
