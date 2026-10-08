//! EIP-7906 transaction assertions, executed by the VM the way a block executes
//! them: `POST_TX` frames observe the transaction's diff through `TXTRACE`, `TXDIFF`
//! and `EVENTDATACOPY`, and one that fails reverts the execution body while the
//! transaction stays valid.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, Code, FRAME_RECEIPT_STATUS_FAILURE, FRAME_RECEIPT_STATUS_SKIPPED,
    FRAME_RECEIPT_STATUS_SUCCESS, Fork, Frame, FrameMode, FrameTransaction, Transaction,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH, utils::keccak};
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

const CHAIN_ID: u64 = 1;
const BASE_FEE: u64 = 1;
/// A contract sender whose runtime is `APPROVE(3)`.
const SENDER: Address = Address::repeat_byte(0xAA);
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];
/// `COUNTER` increments slot 1 and emits `LOG2(data = 0x2a, topic0 = 0x77, topic1 = 0x88)`.
const COUNTER: Address = Address::repeat_byte(0xC0);
const COUNTER_CODE: &[u8] = &[
    0x60, 0x01, 0x54, 0x60, 0x01, 0x01, 0x60, 0x01, 0x55, // SSTORE(1, SLOAD(1) + 1)
    0x60, 0x2a, 0x60, 0x00, 0x52, // MSTORE(0, 0x2a)
    0x60, 0x88, 0x60, 0x77, 0x60, 0x20, 0x60, 0x00, 0xa2, // LOG2(0, 32, 0x77, 0x88)
    0x00,
];
const COUNTER_SLOT: u64 = 1;
const TOPIC0: u64 = 0x77;
const TOPIC1: u64 = 0x88;
/// Runs `TXTRACE(0, 0)` and stops: it halts unless it runs inside a POST_TX frame.
const TXTRACE_PROBE: Address = Address::repeat_byte(0xD0);
const TXTRACE_PROBE_CODE: &[u8] = &[0x60, 0x00, 0x60, 0x00, 0xB6, 0x00];
/// Never touched by the transaction: a live-read control.
const BYSTANDER: Address = Address::repeat_byte(0xB1);
const BYSTANDER_BALANCE: u64 = 5;
const BYSTANDER_SLOT_VALUE: u64 = 9;
const ASSERTION: Address = Address::repeat_byte(0xA5);
/// EIP-8037 state gas for one zero-to-nonzero storage write.
const STORAGE_SET_STATE_GAS: u64 = 64 * 1530;

const TXTRACE: u8 = 0xB6;
const TXDIFF: u8 = 0xB7;
const EVENTDATACOPY: u8 = 0xB8;

fn push32(code: &mut Vec<u8>, value: U256) {
    code.push(0x7f);
    code.extend_from_slice(&value.to_big_endian());
}

/// Code that leaves `TXTRACE(param, index)` on the stack.
fn txtrace(param: u8, index: u64) -> Vec<u8> {
    let mut code = Vec::new();
    push32(&mut code, U256::from(index));
    code.extend_from_slice(&[0x60, param, TXTRACE]);
    code
}

/// Code that leaves `TXDIFF(param, key, index)` on the stack.
fn txdiff(param: u8, key: U256, index: u64) -> Vec<u8> {
    let mut code = Vec::new();
    push32(&mut code, U256::from(index));
    push32(&mut code, key);
    code.extend_from_slice(&[0x60, param, TXDIFF]);
    code
}

fn word(address: Address) -> U256 {
    U256::from_big_endian(H256::from(address).as_bytes())
}

/// An assertion contract: each check evaluates an expression and compares it with
/// the expected value, and any mismatch reverts.
fn assertion_code(checks: &[(Vec<u8>, U256)]) -> Bytes {
    let mut code = Vec::new();
    let mut fixups = Vec::new();
    for (expression, expected) in checks {
        code.extend_from_slice(expression);
        push32(&mut code, *expected);
        code.extend_from_slice(&[0x14, 0x15, 0x61]); // EQ ISZERO PUSH2
        fixups.push(code.len());
        code.extend_from_slice(&[0x00, 0x00, 0x57]); // <fail> JUMPI
    }
    code.push(0x00); // STOP
    let fail = u16::try_from(code.len()).unwrap().to_be_bytes();
    code.extend_from_slice(&[0x5b, 0x60, 0x00, 0x60, 0x00, 0xfd]); // JUMPDEST REVERT(0, 0)
    for at in fixups {
        code[at..at + 2].copy_from_slice(&fail);
    }
    Bytes::from(code)
}

fn chain(assertion: Bytes) -> GeneralizedDatabase {
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());
    let account = |balance: u64, code: &[u8], storage: FxHashMap<H256, U256>| {
        Account::new(
            U256::from(balance),
            Code::from_bytecode(Bytes::copy_from_slice(code), &NativeCrypto),
            1,
            storage,
        )
    };
    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    cache.insert(
        SENDER,
        Account::new(
            U256::from(10u64).pow(U256::from(18u64)),
            Code::from_bytecode(Bytes::from_static(APPROVE_BOTH_CODE), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    cache.insert(COUNTER, account(0, COUNTER_CODE, FxHashMap::default()));
    cache.insert(
        TXTRACE_PROBE,
        account(0, TXTRACE_PROBE_CODE, FxHashMap::default()),
    );
    let mut bystander_storage = FxHashMap::default();
    bystander_storage.insert(
        H256::from_low_u64_be(COUNTER_SLOT),
        U256::from(BYSTANDER_SLOT_VALUE),
    );
    cache.insert(
        BYSTANDER,
        account(BYSTANDER_BALANCE, &[0x00], bystander_storage),
    );
    cache.insert(ASSERTION, account(0, &assertion, FxHashMap::default()));
    GeneralizedDatabase::new_with_account_state(Arc::new(store), cache)
}

fn frame(mode: FrameMode, target: Address, state_gas_limit: u64) -> Frame {
    Frame {
        mode: u8::from(mode),
        flags: if mode == FrameMode::Verify { 0x03 } else { 0 },
        target: Some(target),
        gas_limit: 300_000,
        state_gas_limit,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn verify() -> Frame {
    frame(FrameMode::Verify, SENDER, 0)
}

fn bump_counter() -> Frame {
    frame(FrameMode::Sender, COUNTER, STORAGE_SET_STATE_GAS)
}

fn assert_post_tx() -> Frame {
    frame(FrameMode::PostTx, ASSERTION, 0)
}

fn frame_tx(frames: Vec<Frame>, nonce_seq: u64) -> FrameTransaction {
    FrameTransaction {
        chain_id: CHAIN_ID,
        nonce_keys: vec![U256::zero()],
        nonce_seq,
        sender: SENDER,
        frames,
        max_priority_fee_per_gas: U256::from(1),
        max_fee_per_gas: U256::from(BASE_FEE + 1_000),
        ..Default::default()
    }
}

fn execute(db: &mut GeneralizedDatabase, tx: FrameTransaction) -> Result<ExecutionReport, VMError> {
    let env = Environment {
        origin: tx.sender,
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(CHAIN_ID),
        base_fee_per_gas: U256::from(BASE_FEE),
        gas_price: tx.max_fee_per_gas,
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    };
    let transaction = Transaction::FrameTransaction(tx);
    VM::new(
        env,
        db,
        &transaction,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new succeeds for a frame tx")
    .execute()
}

fn statuses(report: &ExecutionReport) -> Vec<u8> {
    report
        .frame_results
        .as_ref()
        .expect("frame results present")
        .iter()
        .map(|result| result.0)
        .collect()
}

fn counter_value(db: &GeneralizedDatabase) -> U256 {
    db.current_accounts_state
        .get(&COUNTER)
        .and_then(|account| account.storage.get(&H256::from_low_u64_be(COUNTER_SLOT)))
        .copied()
        .unwrap_or_default()
}

/// Runs `[VERIFY, SENDER -> COUNTER, POST_TX -> assertion]` and returns the report.
fn run_assertion(checks: &[(Vec<u8>, U256)]) -> ExecutionReport {
    let mut db = chain(assertion_code(checks));
    execute(
        &mut db,
        frame_tx(vec![verify(), bump_counter(), assert_post_tx()], 0),
    )
    .expect("the transaction is valid")
}

fn assert_holds(checks: &[(Vec<u8>, U256)]) {
    let report = run_assertion(checks);
    assert_eq!(
        statuses(&report),
        vec![FRAME_RECEIPT_STATUS_SUCCESS; 3],
        "every check must hold"
    );
}

#[test]
fn txtrace_enumerates_the_transaction_diff() {
    let max_cost = {
        let tx = frame_tx(vec![verify(), bump_counter(), assert_post_tx()], 0);
        U256::from(tx.max_gas()) * tx.max_fee_per_gas
    };
    assert_holds(&[
        // The payer's escrow is the only balance change before settlement.
        (txtrace(0x00, 0), U256::from(1)),
        (txtrace(0x03, 0), word(SENDER)),
        // The counter's slot, before and after.
        (txtrace(0x01, 0), U256::from(1)),
        (txtrace(0x06, 0), word(COUNTER)),
        (txtrace(0x07, 0), U256::from(COUNTER_SLOT)),
        (txtrace(0x08, 0), U256::zero()),
        (txtrace(0x09, 0), U256::one()),
        (txtrace(0x02, 0), U256::zero()),
        // The counter's event.
        (txtrace(0x0C, 0), U256::from(1)),
        (txtrace(0x0D, 0), word(COUNTER)),
        (txtrace(0x0E, 0), U256::from(2)),
        (txtrace(0x0F, 0), U256::from(TOPIC0)),
        (txtrace(0x10, 0), U256::from(TOPIC1)),
        (txtrace(0x13, 0), U256::from(32)),
        // Gas payment.
        (txtrace(0x14, 0), max_cost),
        (txtrace(0x15, 0), word(SENDER)),
    ]);
}

#[test]
fn txdiff_looks_up_keys_and_views() {
    let counter = word(COUNTER);
    let slot = U256::from(COUNTER_SLOT);
    assert_holds(&[
        (txdiff_slot(0x00, counter, slot), U256::zero()),
        (txdiff_slot(0x01, counter, slot), U256::one()),
        // An untouched key reads the live value before and after.
        (
            txdiff_slot(0x00, word(BYSTANDER), slot),
            U256::from(BYSTANDER_SLOT_VALUE),
        ),
        (
            txdiff_slot(0x01, word(BYSTANDER), slot),
            U256::from(BYSTANDER_SLOT_VALUE),
        ),
        (
            txdiff(0x02, word(BYSTANDER), 0),
            U256::from(BYSTANDER_BALANCE),
        ),
        (
            txdiff(0x03, word(BYSTANDER), 0),
            U256::from(BYSTANDER_BALANCE),
        ),
        (
            txdiff(0x04, counter, 0),
            U256::from_big_endian(keccak(COUNTER_CODE).as_bytes()),
        ),
        // Per-address and per-topic views map to the global tables.
        (txdiff(0x06, counter, 0), U256::one()),
        (txdiff(0x07, counter, 0), U256::zero()),
        (txdiff(0x08, counter, 0), U256::one()),
        (txdiff(0x09, counter, 0), U256::zero()),
        (txdiff(0x0B, U256::from(TOPIC1), 0), U256::one()),
        (txdiff(0x0C, U256::from(TOPIC1), 0), U256::zero()),
        // `topic0` is the event signature and is not a participant.
        (txdiff(0x0B, U256::from(TOPIC0), 0), U256::zero()),
        // Change flags: storage for the counter, nonce and balance for the sender.
        (txdiff(0x0A, counter, 0), U256::from(0b0100)),
        (txdiff(0x0A, word(SENDER), 0), U256::from(0b0011)),
        (txdiff(0x0A, word(BYSTANDER), 0), U256::zero()),
    ]);
}

/// `TXDIFF(param, address, slot)`: the slot is the third operand.
fn txdiff_slot(param: u8, address: U256, slot: U256) -> Vec<u8> {
    let mut code = Vec::new();
    push32(&mut code, slot);
    push32(&mut code, address);
    code.extend_from_slice(&[0x60, param, TXDIFF]);
    code
}

#[test]
fn eventdatacopy_copies_event_data_and_halts_past_its_end() {
    // EVENTDATACOPY(event 0, memory 0, data 0, 32 bytes), then MLOAD(0).
    let copy = vec![
        0x60,
        0x20,
        0x60,
        0x00,
        0x60,
        0x00,
        0x60,
        0x00,
        EVENTDATACOPY,
        0x60,
        0x00,
        0x51,
    ];
    assert_holds(&[(copy, U256::from(0x2a))]);

    // One byte past the end of the data halts the frame.
    let past_end = vec![
        0x60,
        0x21,
        0x60,
        0x00,
        0x60,
        0x00,
        0x60,
        0x00,
        EVENTDATACOPY,
        0x60,
        0x01,
    ];
    let report = run_assertion(&[(past_end, U256::one())]);
    assert_eq!(statuses(&report)[2], FRAME_RECEIPT_STATUS_FAILURE);
}

#[test]
fn a_failing_assertion_reverts_the_execution_body_and_keeps_the_transaction() {
    let sender_balance_before = U256::from(10u64).pow(U256::from(18u64));
    let mut db = chain(assertion_code(&[(txtrace(0x01, 0), U256::from(2))]));
    let report = execute(
        &mut db,
        frame_tx(vec![verify(), bump_counter(), assert_post_tx()], 0),
    )
    .expect("a failed assertion leaves the transaction valid");

    assert_eq!(
        statuses(&report),
        vec![
            FRAME_RECEIPT_STATUS_SUCCESS,
            FRAME_RECEIPT_STATUS_SUCCESS,
            FRAME_RECEIPT_STATUS_FAILURE,
        ]
    );
    let results = report.frame_results.as_ref().unwrap();
    // The body frame keeps its status and execution gas, with no logs and no state gas.
    assert!(results[1].1 > 0);
    assert_eq!(results[1].2, 0);
    assert!(results[1].3.is_empty());
    assert!(report.logs.is_empty());
    assert_eq!(report.state_gas_used, 0);
    // The body's write is gone; the prefix's nonce consumption and payment stay.
    assert_eq!(counter_value(&db), U256::zero());
    let sender = db.current_accounts_state.get(&SENDER).unwrap();
    assert_eq!(sender.info.nonce, 1);
    assert!(sender.info.balance < sender_balance_before);
}

#[test]
fn frames_after_a_failed_assertion_are_skipped() {
    let mut db = chain(assertion_code(&[(txtrace(0x01, 0), U256::from(2))]));
    let report = execute(
        &mut db,
        frame_tx(
            vec![verify(), bump_counter(), assert_post_tx(), assert_post_tx()],
            0,
        ),
    )
    .expect("the transaction is valid");
    assert_eq!(
        statuses(&report),
        vec![
            FRAME_RECEIPT_STATUS_SUCCESS,
            FRAME_RECEIPT_STATUS_SUCCESS,
            FRAME_RECEIPT_STATUS_FAILURE,
            FRAME_RECEIPT_STATUS_SKIPPED,
        ]
    );
    assert_eq!(report.frame_results.as_ref().unwrap()[3].1, 0);
}

#[test]
fn the_opcodes_halt_outside_a_post_tx_frame() {
    let mut db = chain(assertion_code(&[]));
    let report = execute(
        &mut db,
        frame_tx(
            vec![verify(), frame(FrameMode::Sender, TXTRACE_PROBE, 0)],
            0,
        ),
    )
    .expect("the transaction is valid");
    assert_eq!(
        statuses(&report),
        vec![FRAME_RECEIPT_STATUS_SUCCESS, FRAME_RECEIPT_STATUS_FAILURE]
    );
}

#[test]
fn approve_in_a_post_tx_frame_halts_and_reverts_the_body() {
    let mut db = chain(assertion_code(&[]));
    let report = execute(
        &mut db,
        frame_tx(
            vec![
                verify(),
                bump_counter(),
                frame(FrameMode::PostTx, SENDER, 0),
            ],
            0,
        ),
    )
    .expect("the transaction is valid");
    assert_eq!(statuses(&report)[2], FRAME_RECEIPT_STATUS_FAILURE);
    assert_eq!(counter_value(&db), U256::zero());
}

#[test]
fn the_prestate_includes_earlier_transactions() {
    // The first transaction bumps the counter to 1 in the same database; the second
    // bumps it to 2 and must see 1 as the value before it ran.
    let slot = U256::from(COUNTER_SLOT);
    let mut db = chain(assertion_code(&[
        (txdiff_slot(0x00, word(COUNTER), slot), U256::one()),
        (txdiff_slot(0x01, word(COUNTER), slot), U256::from(2)),
        (txtrace(0x08, 0), U256::one()),
    ]));
    execute(&mut db, frame_tx(vec![verify(), bump_counter()], 0)).expect("first tx is valid");
    let report = execute(
        &mut db,
        frame_tx(vec![verify(), bump_counter(), assert_post_tx()], 1),
    )
    .expect("second tx is valid");
    assert_eq!(statuses(&report), vec![FRAME_RECEIPT_STATUS_SUCCESS; 3]);
    assert_eq!(counter_value(&db), U256::from(2));
}
