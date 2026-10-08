//! EIP-8272: `RECENT_ROOT_CODE` and the recent root verifier frame, run by the VM
//! the way a block runs them.
//!
//! A root written in slot `S` through the contract's write operation validates
//! from slot `S + 1` through a `VERIFY` frame that calls the contract with the
//! packed tuples. A tuple naming the wrong root, source, the current slot, or a
//! slot outside the usable window reverts the frame, and a reverting `VERIFY`
//! frame invalidates the transaction.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, ChainConfig, Code, FRAME_RECEIPT_STATUS_SUCCESS, FRAME_TX_MAX_VERIFY_GAS,
    Fork, Frame, FrameMode, FrameTransaction, RecentRootReference, Transaction,
    frame_tx_recent_root,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH, utils::keccak};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::{ExecutionReport, TxValidationError, VMError};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use ethrex_vm::backends::FrameValidationOutcome;
use ethrex_vm::backends::levm::LEVM;
use ethrex_vm::system_contracts::RECENT_ROOT_RUNTIME_BYTECODE;
use rustc_hash::FxHashMap;
use std::sync::Arc;

const HARNESS_CHAIN_ID: u64 = 1;
const HARNESS_BASE_FEE: u64 = 1;
/// A contract sender whose runtime is `APPROVE(3)`: it approves execution and
/// payment for itself, so its transactions need no signature.
const SENDER: Address = Address::repeat_byte(0xAA);
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];
const WRITE_SLOT: u64 = 1_000;
/// EIP-8037 state gas for one zero-to-nonzero storage write.
const STORAGE_SET_STATE_GAS: u64 = 64 * 1530;

/// The sender, funded, and the contract holding `RECENT_ROOT_CODE` with the
/// given storage.
fn chain_with_storage(recent_root_storage: FxHashMap<H256, U256>) -> GeneralizedDatabase {
    // Execution takes its fork from the environment, but the admission simulation
    // reads it from the store's chain config.
    let mut in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(in_memory_db.set_chain_config(&ChainConfig {
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(0),
            osaka_time: Some(0),
            amsterdam_time: Some(0),
            hegota_time: Some(0),
            ..Default::default()
        }))
        .unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());
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
    cache.insert(
        frame_tx_recent_root(),
        Account::new(
            U256::zero(),
            Code::from_bytecode(
                Bytes::from_static(&RECENT_ROOT_RUNTIME_BYTECODE),
                &NativeCrypto,
            ),
            1,
            recent_root_storage,
        ),
    );
    GeneralizedDatabase::new_with_account_state(Arc::new(store), cache)
}

fn chain() -> GeneralizedDatabase {
    chain_with_storage(FxHashMap::default())
}

fn run_at_slot(
    db: &mut GeneralizedDatabase,
    tx: FrameTransaction,
    slot: u64,
) -> Result<ExecutionReport, VMError> {
    let env = Environment {
        origin: tx.sender,
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(HARNESS_CHAIN_ID),
        base_fee_per_gas: U256::from(HARNESS_BASE_FEE),
        gas_price: tx.max_fee_per_gas,
        tx_nonce: tx.nonce_seq,
        slot_number: U256::from(slot),
        ..Default::default()
    };
    let transaction = Transaction::FrameTransaction(tx);
    let mut vm = VM::new(
        env,
        db,
        &transaction,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new should succeed for a frame tx");
    vm.execute()
}

fn approve_frame() -> Frame {
    Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(SENDER),
        gas_limit: 100_000,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn frame_tx(frames: Vec<Frame>, nonce_seq: u64) -> FrameTransaction {
    FrameTransaction {
        chain_id: HARNESS_CHAIN_ID,
        nonce_keys: vec![U256::zero()],
        nonce_seq,
        sender: SENDER,
        frames,
        signatures: Vec::new(),
        max_priority_fee_per_gas: U256::from(1),
        max_fee_per_gas: U256::from(HARNESS_BASE_FEE + 1_000),
        ..Default::default()
    }
}

/// A `SENDER` frame calling the contract's write operation with `salt || root`.
/// It commits the entry for `source_id = keccak256(SENDER || salt)` at the slot
/// the transaction executes in, and needs state gas for the slot it creates.
fn write_frame(salt: H256, root: H256) -> Frame {
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(salt.as_bytes());
    data.extend_from_slice(root.as_bytes());
    Frame {
        mode: u8::from(FrameMode::Sender),
        flags: 0,
        target: Some(frame_tx_recent_root()),
        gas_limit: 200_000,
        state_gas_limit: STORAGE_SET_STATE_GAS,
        value: U256::zero(),
        data: Bytes::from(data),
    }
}

fn packed(tuples: &[RecentRootReference]) -> Vec<u8> {
    let mut data = Vec::with_capacity(tuples.len() * 72);
    for tuple in tuples {
        data.extend_from_slice(tuple.source_id.as_bytes());
        data.extend_from_slice(&tuple.slot.to_be_bytes());
        data.extend_from_slice(tuple.root.as_bytes());
    }
    data
}

/// The recent root verifier frame first, then the sender's own approval.
fn verify_tx(data: Vec<u8>, nonce_seq: u64) -> FrameTransaction {
    frame_tx(
        vec![
            Frame {
                mode: u8::from(FrameMode::Verify),
                flags: 0,
                target: Some(frame_tx_recent_root()),
                gas_limit: 100_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::from(data),
            },
            approve_frame(),
        ],
        nonce_seq,
    )
}

fn source_id(source: Address, salt: H256) -> H256 {
    let mut preimage = Vec::with_capacity(52);
    preimage.extend_from_slice(source.as_bytes());
    preimage.extend_from_slice(salt.as_bytes());
    keccak(&preimage)
}

fn recent_root_storage_at(db: &GeneralizedDatabase, key: H256) -> U256 {
    db.current_accounts_state
        .get(&frame_tx_recent_root())
        .and_then(|account| account.storage.get(&key))
        .copied()
        .unwrap_or_default()
}

fn assert_all_frames_succeeded(report: &ExecutionReport) {
    let frames = report
        .frame_results
        .as_ref()
        .expect("frame results present");
    assert!(
        frames
            .iter()
            .all(|frame| frame.0 == FRAME_RECEIPT_STATUS_SUCCESS),
        "every frame must succeed; got {frames:?}"
    );
}

/// A failed `VERIFY` frame invalidates the transaction.
fn assert_verify_frame_failed(result: &Result<ExecutionReport, VMError>, label: &str) {
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                TxValidationError::InvalidFrameTransaction
            ))
        ),
        "{label}: expected a failed VERIFY frame, got {result:?}"
    );
}

/// Writes one root per salt at `WRITE_SLOT`, all in one transaction, and returns
/// the chain plus the tuples that name them.
fn chain_with_written_roots(count: u8) -> (GeneralizedDatabase, Vec<RecentRootReference>) {
    let tuples: Vec<RecentRootReference> = (0..count)
        .map(|i| RecentRootReference {
            source_id: source_id(SENDER, H256::repeat_byte(0x50 + i)),
            slot: WRITE_SLOT,
            root: H256::repeat_byte(0x20 + i),
        })
        .collect();
    let mut frames = vec![approve_frame()];
    frames.extend(
        (0..count).map(|i| write_frame(H256::repeat_byte(0x50 + i), H256::repeat_byte(0x20 + i))),
    );
    let mut db = chain();
    let report = run_at_slot(&mut db, frame_tx(frames, 0), WRITE_SLOT)
        .expect("the write transaction is valid");
    assert_all_frames_succeeded(&report);
    // The contract stored exactly the entry the read side derives.
    for tuple in &tuples {
        assert_eq!(
            recent_root_storage_at(&db, tuple.storage_key()),
            U256::from_big_endian(tuple.entry_hash().as_bytes()),
            "the write must commit entry_hash under storage_key"
        );
    }
    (db, tuples)
}

fn chain_with_a_written_root() -> (GeneralizedDatabase, RecentRootReference) {
    let (db, tuples) = chain_with_written_roots(1);
    (db, tuples[0])
}

#[test]
fn reference_vector_derives_and_validates() {
    // EIP-8272 "Reference vector", with `current_slot = 2`.
    let calldata = hex::decode(
        "b9382d35273c75a50631a3e84d3c75ec9266e2b18c35a627e16cdbf26a18ca85\
         0000000000000001\
         0000000000000000000000000000000000000000000000000000000000000002",
    )
    .unwrap();
    let tuple = RecentRootReference::from_tuple_bytes(&calldata).expect("a 72-byte tuple");
    assert_eq!(
        tuple.source_id,
        source_id(Address::from_low_u64_be(1), H256::zero())
    );
    assert_eq!(tuple.slot, 1);
    assert_eq!(tuple.root, H256::from_low_u64_be(2));
    let entry_hash = H256::from_slice(
        &hex::decode("0a0d1254c851be5a133b4c9a9e300f5602fc0f43dbe65aa6a66930d4ca0a51b8").unwrap(),
    );
    let storage_key = H256::from_slice(
        &hex::decode("5f027aa1cbe2df279bf6518edd4b44ea5409fd800189ec35224e10ab05e574c3").unwrap(),
    );
    assert_eq!(tuple.entry_hash(), entry_hash);
    assert_eq!(tuple.storage_key(), storage_key);

    // With `storage[storage_key] = entry_hash`, the validation operation succeeds.
    let mut storage = FxHashMap::default();
    storage.insert(storage_key, U256::from_big_endian(entry_hash.as_bytes()));
    let mut db = chain_with_storage(storage);
    let report = run_at_slot(&mut db, verify_tx(calldata.clone(), 0), 2)
        .expect("the reference vector validates at slot 2");
    assert_all_frames_succeeded(&report);

    // Without the entry, the same frame reverts and the transaction is invalid.
    let mut empty = chain();
    assert_verify_frame_failed(
        &run_at_slot(&mut empty, verify_tx(calldata, 0), 2),
        "missing entry",
    );
}

#[test]
fn a_root_written_in_slot_s_validates_from_slot_s_plus_one() {
    let (mut db, tuple) = chain_with_a_written_root();

    // The slot the write landed in is not yet referenceable.
    let same_slot = run_at_slot(&mut db, verify_tx(packed(&[tuple]), 1), WRITE_SLOT);
    assert_verify_frame_failed(&same_slot, "current slot");

    // From the next slot on it validates, and the frame creates no state.
    let report = run_at_slot(&mut db, verify_tx(packed(&[tuple]), 1), WRITE_SLOT + 1)
        .expect("a committed tuple one slot old validates");
    assert_all_frames_succeeded(&report);
    let frames = report.frame_results.expect("frame results present");
    assert_eq!(frames[0].2, 0, "the verifier frame creates no state");

    // Sixteen copies of the tuple are sixteen independent checks that all pass.
    let report = run_at_slot(&mut db, verify_tx(packed(&[tuple; 16]), 2), WRITE_SLOT + 1)
        .expect("sixteen duplicate tuples validate");
    assert_all_frames_succeeded(&report);
}

#[test]
fn sixteen_distinct_cold_tuples_validate() {
    let (mut db, tuples) = chain_with_written_roots(16);
    let report = run_at_slot(&mut db, verify_tx(packed(&tuples), 1), WRITE_SLOT + 1)
        .expect("sixteen distinct written tuples validate");
    assert_all_frames_succeeded(&report);
}

#[test]
fn a_future_slot_reverts_the_verifier_frame() {
    let (mut db, tuple) = chain_with_a_written_root();
    let mut future = tuple;
    future.slot = WRITE_SLOT + 5;
    let result = run_at_slot(&mut db, verify_tx(packed(&[future]), 1), WRITE_SLOT + 1);
    assert_verify_frame_failed(&result, "future slot");
}

#[test]
fn the_usable_window_is_8191_slots() {
    let (mut db, tuple) = chain_with_a_written_root();
    run_at_slot(&mut db, verify_tx(packed(&[tuple]), 1), WRITE_SLOT + 8191)
        .expect("age 8191 is the last usable age");
    let expired = run_at_slot(&mut db, verify_tx(packed(&[tuple]), 2), WRITE_SLOT + 8192);
    assert_verify_frame_failed(&expired, "age 8192");
}

#[test]
fn a_wrong_root_source_or_slot_reverts_the_verifier_frame() {
    let (mut db, tuple) = chain_with_a_written_root();
    let mut wrong_root = tuple;
    wrong_root.root = H256::repeat_byte(0x33);
    let mut wrong_source = tuple;
    wrong_source.source_id = H256::repeat_byte(0x44);
    let mut wrong_slot = tuple;
    wrong_slot.slot = WRITE_SLOT - 1;
    for (label, bad) in [
        ("root", wrong_root),
        ("source", wrong_source),
        ("slot", wrong_slot),
    ] {
        let result = run_at_slot(&mut db, verify_tx(packed(&[bad]), 1), WRITE_SLOT + 1);
        assert_verify_frame_failed(&result, label);
    }
    // One bad tuple among good ones fails the whole frame.
    let mut mixed = packed(&[tuple, tuple]);
    mixed[143] ^= 0x01;
    let result = run_at_slot(&mut db, verify_tx(mixed, 1), WRITE_SLOT + 1);
    assert_verify_frame_failed(&result, "one bad tuple among good ones");
}

#[test]
fn malformed_validation_data_reverts() {
    let (mut db, tuple) = chain_with_a_written_root();
    let good = packed(&[tuple]);
    for (label, data) in [
        ("empty", Vec::new()),
        ("71 bytes", good[..71].to_vec()),
        ("73 bytes", [good.clone(), vec![0]].concat()),
        ("17 tuples", packed(&[tuple; 17])),
    ] {
        // None of these is a recent root verifier frame, so the VERIFY frame
        // reaches the contract as an ordinary call, and its revert invalidates
        // the transaction.
        let result = run_at_slot(&mut db, verify_tx(data, 1), WRITE_SLOT + 1);
        assert_verify_frame_failed(&result, label);
    }
}

#[test]
fn the_last_write_in_a_slot_wins() {
    // Two writes with the same salt in one slot target the same storage key; only
    // the second root is referenceable.
    let salt = H256::repeat_byte(0x5A);
    let first = H256::repeat_byte(0x01);
    let second = H256::repeat_byte(0x02);
    let mut db = chain();
    let report = run_at_slot(
        &mut db,
        frame_tx(
            vec![
                approve_frame(),
                write_frame(salt, first),
                write_frame(salt, second),
            ],
            0,
        ),
        WRITE_SLOT,
    )
    .expect("the write transaction is valid");
    assert_all_frames_succeeded(&report);
    let named = |root| RecentRootReference {
        source_id: source_id(SENDER, salt),
        slot: WRITE_SLOT,
        root,
    };
    assert_verify_frame_failed(
        &run_at_slot(
            &mut db,
            verify_tx(packed(&[named(first)]), 1),
            WRITE_SLOT + 1,
        ),
        "overwritten root",
    );
    run_at_slot(
        &mut db,
        verify_tx(packed(&[named(second)]), 1),
        WRITE_SLOT + 1,
    )
    .expect("the last write in the slot validates");
}

#[test]
fn a_write_through_a_static_frame_halts_and_writes_nothing() {
    // A VERIFY frame runs the contract under STATICCALL semantics: the write's
    // SSTORE halts, the frame fails, and the transaction is invalid.
    let mut db = chain();
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(H256::repeat_byte(0x5A).as_bytes());
    data.extend_from_slice(H256::repeat_byte(0x22).as_bytes());
    let result = run_at_slot(&mut db, verify_tx(data, 0), WRITE_SLOT);
    assert_verify_frame_failed(&result, "static write");
    let untouched = db
        .current_accounts_state
        .get(&frame_tx_recent_root())
        .map(|account| account.storage.is_empty())
        .unwrap_or(true);
    assert!(
        untouched,
        "no recent-root entry may be written from a static context"
    );
}

// ==================== Admission simulation agrees with execution ====================

/// The head the admission simulation runs against: the next payload's slot is the
/// head slot plus one, so a head at `WRITE_SLOT` admits tuples from `WRITE_SLOT`.
fn head_at_write_slot() -> BlockHeader {
    BlockHeader {
        gas_limit: 30_000_000,
        slot_number: Some(WRITE_SLOT),
        base_fee_per_gas: Some(HARNESS_BASE_FEE),
        ..Default::default()
    }
}

/// Runs the admission simulation over `tx`, whose approval frame is trimmed so the
/// prefix fits `MAX_VERIFY_GAS` (the execution tests above budget it generously).
fn admit(db: &mut GeneralizedDatabase, mut tx: FrameTransaction) -> FrameValidationOutcome {
    tx.frames[1].gas_limit = 10_000;
    let prefix = tx.validation_prefix().expect("a recognized prefix");
    assert_eq!(prefix.recent_root_index, Some(0));
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("a structurally valid prefix");
    LEVM::simulate_frame_validation_prefix(
        &Transaction::FrameTransaction(tx),
        &head_at_write_slot(),
        db,
        VMType::L1,
        &NativeCrypto,
        &prefix,
        None,
        FRAME_TX_MAX_VERIFY_GAS,
    )
    .expect("simulation runs")
}

#[test]
fn admission_and_execution_agree_on_the_verifier_frame_gas() {
    // EIP-8272: the mempool must reject a recent root verifier frame whose
    // `limits.execution` is one gas short of what EVM execution needs, the cold
    // access to the contract at frame entry included.
    let (mut db, tuple) = chain_with_a_written_root();
    let mut probe = db.clone();
    let report = run_at_slot(&mut probe, verify_tx(packed(&[tuple]), 1), WRITE_SLOT + 1)
        .expect("the tuple validates");
    let needed = report.frame_results.expect("frame results present")[0].1;

    let with_limit = |gas_limit: u64| {
        let mut tx = verify_tx(packed(&[tuple]), 1);
        tx.frames[0].gas_limit = gas_limit;
        tx
    };
    let exact = admit(&mut db.clone(), with_limit(needed));
    assert!(exact.passed, "exactly enough gas: {:?}", exact.violation);
    let short = admit(&mut db, with_limit(needed - 1));
    assert!(!short.passed, "one gas short must be rejected");

    // Block execution agrees: one gas short halts the frame and invalidates the tx.
    let (mut db, tuple) = chain_with_a_written_root();
    let mut tx = verify_tx(packed(&[tuple]), 1);
    tx.frames[0].gas_limit = needed - 1;
    assert_verify_frame_failed(&run_at_slot(&mut db, tx, WRITE_SLOT + 1), "one gas short");
}

#[test]
fn account_validation_can_check_the_verifier_frame_status_at_admission() {
    // EIP-8272 §Application introspection: the frame behind the verifier reads its
    // status through FRAMEPARAM before trusting the tuple. The admission simulation
    // must record completed frames the way execution does, or that read halts.
    //
    // PUSH1 0x05 (status); PUSH1 0x00 (frame 0); FRAMEPARAM; PUSH1 1; EQ;
    // PUSH1 0x10; JUMPI; PUSH1 0; PUSH1 0; REVERT; JUMPDEST; APPROVE(3).
    const CHECK_THEN_APPROVE: &[u8] = &[
        0x60, 0x05, 0x60, 0x00, 0xB3, 0x60, 0x01, 0x14, 0x60, 0x10, 0x57, 0x60, 0x00, 0x60, 0x00,
        0xFD, 0x5B, 0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA,
    ];
    let (mut db, tuple) = chain_with_a_written_root();
    let code = Code::from_bytecode(Bytes::from_static(CHECK_THEN_APPROVE), &NativeCrypto);
    db.current_accounts_state
        .get_mut(&SENDER)
        .expect("sender seeded")
        .info
        .code_hash = code.hash;
    db.codes.insert(code.hash, code);

    let mut tx = verify_tx(packed(&[tuple]), 1);
    tx.frames[0].gas_limit = 60_000;
    let outcome = admit(&mut db.clone(), tx);
    assert!(outcome.passed, "admission: {:?}", outcome.violation);
    let report = run_at_slot(&mut db, verify_tx(packed(&[tuple]), 1), WRITE_SLOT + 1)
        .expect("execution validates too");
    assert_all_frames_succeeded(&report);
}
