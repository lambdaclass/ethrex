//! EIP-8272 (`824cbc0b0e`): the two-operation RECENT_ROOT_CODE and the canonical
//! recent-root verifier frame, run by the VM the way a block runs them.
//!
//! A root written in slot `S` through the predeploy's write operation validates
//! from slot `S + 1` through the verifier frame, whose `STATICCALL` semantics run
//! the contract's validation operation; a tuple naming the wrong root, the
//! current slot, or a slot outside the usable window reverts the frame, and a
//! reverting VERIFY frame invalidates the transaction.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, Code, Fork, Frame, FrameMode, FrameTransaction, RecentRootReference,
    Transaction, frame_tx_recent_root,
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

type SeededAccount = (Address, U256, u64, Bytes);

fn seeded_db(accounts: &[SeededAccount]) -> GeneralizedDatabase {
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());
    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    for (address, balance, nonce, code) in accounts {
        cache.insert(
            *address,
            Account::new(
                *balance,
                Code::from_bytecode(code.clone(), &NativeCrypto),
                *nonce,
                FxHashMap::default(),
            ),
        );
    }
    GeneralizedDatabase::new_with_account_state(Arc::new(store), cache)
}

/// The sender, funded, and the predeploy holding RECENT_ROOT_CODE.
fn chain() -> GeneralizedDatabase {
    seeded_db(&[
        (
            SENDER,
            U256::from(10u64).pow(U256::from(18u64)),
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (
            frame_tx_recent_root(),
            U256::zero(),
            1,
            Bytes::from_static(&RECENT_ROOT_RUNTIME_BYTECODE),
        ),
    ])
}

fn run_at_slot(
    db: &mut GeneralizedDatabase,
    tx: FrameTransaction,
    slot: u64,
) -> Result<ExecutionReport, VMError> {
    let mut env = Environment {
        origin: tx.sender,
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(HARNESS_CHAIN_ID),
        base_fee_per_gas: U256::from(HARNESS_BASE_FEE),
        gas_price: U256::from(tx.max_fee_per_gas),
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    };
    // `SLOTNUM` reads `env.slot_number`; the config copy is what the block-context
    // derivation fills, and both are set so the two never disagree in a test.
    env.slot_number = U256::from(slot);
    env.config.slot_number = U256::from(slot);
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

/// The write operation: a SENDER frame calling the predeploy with `salt || root`,
/// which commits the entry for `source_id = keccak256(SENDER || salt)` at the slot
/// the transaction executes in. It needs state budget for the slot it creates.
fn write_tx(salt: H256, root: H256, nonce_seq: u64) -> FrameTransaction {
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(salt.as_bytes());
    data.extend_from_slice(root.as_bytes());
    frame_tx(
        vec![
            approve_frame(),
            Frame {
                mode: u8::from(FrameMode::Sender),
                flags: 0,
                target: Some(frame_tx_recent_root()),
                gas_limit: 200_000,
                state_gas_limit: 64 * 1530,
                value: U256::zero(),
                data: Bytes::from(data),
            },
        ],
        nonce_seq,
    )
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

/// The canonical verifier frame first, then the sender's own approval.
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

fn source_id(salt: H256) -> H256 {
    let mut preimage = Vec::with_capacity(52);
    preimage.extend_from_slice(SENDER.as_bytes());
    preimage.extend_from_slice(salt.as_bytes());
    keccak(&preimage)
}

/// Writes one root at `WRITE_SLOT` and returns the chain plus the tuple that
/// names it.
fn chain_with_a_written_root() -> (GeneralizedDatabase, RecentRootReference) {
    let salt = H256::repeat_byte(0x5A);
    let root = H256::repeat_byte(0x22);
    let mut db = chain();
    let report = run_at_slot(&mut db, write_tx(salt, root, 0), WRITE_SLOT)
        .expect("the write transaction is valid");
    let frames = report.frame_results.expect("frame results present");
    assert_eq!(
        frames[1].0,
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "the 64-byte write must succeed; got {frames:?}"
    );
    let tuple = RecentRootReference {
        source_id: source_id(salt),
        slot: WRITE_SLOT,
        root,
    };
    // The predeploy stored exactly the entry the read side derives.
    let stored = db
        .current_accounts_state
        .get(&frame_tx_recent_root())
        .and_then(|account| account.storage.get(&tuple.storage_key()))
        .copied()
        .unwrap_or_default();
    assert_eq!(
        stored,
        U256::from_big_endian(tuple.entry_hash().as_bytes()),
        "the write must commit entry_hash under storage_key"
    );
    (db, tuple)
}

#[test]
fn a_root_written_in_slot_s_validates_from_slot_s_plus_one() {
    let (mut db, tuple) = chain_with_a_written_root();

    // The slot the write landed in is not yet referenceable.
    let same_slot = run_at_slot(&mut db, verify_tx(packed(&[tuple.clone()]), 1), WRITE_SLOT);
    assert!(
        same_slot.is_err(),
        "a tuple naming the current slot must revert the verifier frame and invalidate the tx"
    );

    // From the next slot on it validates, and the frame is attributed a success.
    let report = run_at_slot(
        &mut db,
        verify_tx(packed(&[tuple.clone()]), 1),
        WRITE_SLOT + 1,
    )
    .expect("a committed tuple one slot old validates");
    let frames = report.frame_results.expect("frame results present");
    assert_eq!(
        frames[0].0,
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS
    );
    assert_eq!(frames[0].2, 0, "the verifier frame creates no state");

    // Sixteen copies of the tuple are sixteen independent checks that all pass.
    run_at_slot(
        &mut db,
        verify_tx(packed(&vec![tuple; 16]), 2),
        WRITE_SLOT + 1,
    )
    .expect("sixteen valid tuples validate");
}

#[test]
fn the_usable_window_is_8191_slots() {
    let (mut db, tuple) = chain_with_a_written_root();
    run_at_slot(
        &mut db,
        verify_tx(packed(&[tuple.clone()]), 1),
        WRITE_SLOT + 8191,
    )
    .expect("age 8191 is the last usable age");
    let expired = run_at_slot(&mut db, verify_tx(packed(&[tuple]), 2), WRITE_SLOT + 8192);
    assert!(
        expired.is_err(),
        "age 8192 reaches the ring's aliasing slot and must revert"
    );
}

#[test]
fn a_wrong_root_source_or_slot_reverts_the_verifier_frame() {
    let (mut db, tuple) = chain_with_a_written_root();
    let mut wrong_root = tuple.clone();
    wrong_root.root = H256::repeat_byte(0x33);
    let mut wrong_source = tuple.clone();
    wrong_source.source_id = H256::repeat_byte(0x44);
    let mut wrong_slot = tuple.clone();
    wrong_slot.slot = WRITE_SLOT - 1;
    for (label, bad) in [
        ("root", wrong_root),
        ("source", wrong_source),
        ("slot", wrong_slot),
    ] {
        let result = run_at_slot(&mut db, verify_tx(packed(&[bad]), 1), WRITE_SLOT + 1);
        assert!(
            result.is_err(),
            "a tuple with the wrong {label} must revert the frame"
        );
    }
    // One bad tuple among good ones fails the whole frame.
    let mut mixed = packed(&[tuple.clone(), tuple]);
    mixed[71] ^= 0x01;
    let result = run_at_slot(&mut db, verify_tx(mixed, 1), WRITE_SLOT + 1);
    assert!(result.is_err(), "any failing tuple reverts the frame");
}

#[test]
fn malformed_validation_data_reverts() {
    let (mut db, tuple) = chain_with_a_written_root();
    let good = packed(&[tuple.clone()]);
    for (label, data) in [
        ("empty", Vec::new()),
        ("71 bytes", good[..71].to_vec()),
        ("73 bytes", [good.clone(), vec![0]].concat()),
        ("17 tuples", packed(&vec![tuple.clone(); 17])),
    ] {
        // None of these is a verifier frame, so the VERIFY frame reaches the contract
        // as an ordinary call and its revert invalidates the transaction.
        let result = run_at_slot(&mut db, verify_tx(data, 1), WRITE_SLOT + 1);
        assert!(result.is_err(), "{label} of validation data must revert");
    }
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
    assert!(
        result.is_err(),
        "a static write must halt the verifier frame"
    );
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
