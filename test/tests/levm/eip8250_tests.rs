//! EIP-8250: keyed-nonce consumption and its interaction with atomic batches.
//!
//! A payment-scoped APPROVE commits five coupled effects (nonce consumption,
//! payer recording, the balance debit, first-use gas, approval flags). They
//! must all take effect together or not at all. Inside an atomic batch a
//! sibling frame's failure rolls the whole batch's state back, which would
//! unwind the balance debit while the transaction stayed authorized — minting
//! the difference at the end-of-tx refund. ethrex forbids payment approval
//! inside a batch (the mempool already bans the batch flag in the validation
//! prefix; this covers the consensus path a crafted block could otherwise
//! reach). The legitimate case — payment granted in a non-batch frame, a
//! *later* atomic batch reverting — keeps the consumption because an
//! independent frame's committed state is absorbed into the tx-level backup
//! and is not in the batch's revert scope.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, Code, Fork, Frame, FrameMode, FrameTransaction, Transaction,
    frame_tx_nonce_manager,
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

const HARNESS_CHAIN_ID: u64 = 1;
const HARNESS_BASE_FEE: u64 = 1;
const FUNDED_SENDER: Address = Address::repeat_byte(0xAA);
fn sender_balance() -> U256 {
    U256::from(10u64).pow(U256::from(18u64))
}

/// APPROVE(scope=3): sender + payment approval; frame target must be the sender.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];
/// SSTORE 1@0; REVERT — a state-writing frame that always reverts.
const SSTORE_THEN_REVERT_CODE: &[u8] =
    &[0x60, 0x01, 0x60, 0x00, 0x55, 0x60, 0x00, 0x60, 0x00, 0xFD];
/// SSTORE 1@0; STOP.
const SSTORE_THEN_STOP_CODE: &[u8] = &[0x60, 0x01, 0x60, 0x00, 0x55, 0x00];
/// NONCE_MANAGER predeploy runtime code: PUSH1 0; PUSH1 0; REVERT.
const NONCE_MANAGER_STUB_CODE: &[u8] = &[0x60, 0x00, 0x60, 0x00, 0xFD];

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

fn frame_tx_with_keys(frames: Vec<Frame>, nonce_keys: Vec<U256>) -> FrameTransaction {
    FrameTransaction {
        chain_id: HARNESS_CHAIN_ID,
        nonce_keys,
        nonce_seq: 0,
        sender: FUNDED_SENDER,
        frames,
        signatures: Vec::new(),
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: HARNESS_BASE_FEE + 1_000,
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: Vec::new(),
        inner_hash: Default::default(),
        cached_canonical: Default::default(),
    }
}

fn frame(mode: FrameMode, flags: u8, target: Address, gas_limit: u64, data: &[u8]) -> Frame {
    Frame {
        mode: u8::from(mode),
        flags,
        target: Some(target),
        gas_limit,
        value: U256::zero(),
        data: Bytes::from(data.to_vec()),
    }
}

fn run_frame_tx(
    accounts: &[SeededAccount],
    tx: FrameTransaction,
) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    let mut db = seeded_db(accounts);
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

fn nonce_of(db: &GeneralizedDatabase, addr: Address) -> u64 {
    db.current_accounts_state
        .get(&addr)
        .map(|account| account.info.nonce)
        .unwrap_or_default()
}

fn storage_slot(db: &GeneralizedDatabase, addr: Address, key: H256) -> U256 {
    db.current_accounts_state
        .get(&addr)
        .and_then(|account| account.storage.get(&key).copied())
        .unwrap_or_default()
}

/// NONCE_MANAGER slot for `(sender, key)`: keccak256(pad32(sender) || be32(key)).
fn keyed_slot(sender: Address, key: U256) -> H256 {
    let mut preimage = [0u8; 64];
    preimage[12..32].copy_from_slice(sender.as_bytes());
    preimage[32..64].copy_from_slice(&key.to_big_endian());
    H256(ethrex_crypto::keccak::keccak_hash(preimage))
}

fn nonce_manager_account() -> SeededAccount {
    (
        frame_tx_nonce_manager(),
        U256::zero(),
        1,
        Bytes::from(NONCE_MANAGER_STUB_CODE.to_vec()),
    )
}

// ==================== payment APPROVE inside a batch is rejected ====================

#[test]
fn payment_approve_inside_atomic_batch_is_rejected() {
    // frame[0] VERIFY with scope=3 AND the atomic-batch flag (0x07), so the
    // payment APPROVE commits inside the batch; frame[1] is the terminator.
    // A crafted block could reach this at consensus (the mempool bans it via
    // the prefix rules). The payment APPROVE must revert, leaving `payer`
    // unset, so the whole tx is invalid — the balance debit can never be
    // stranded outside a surviving authorization.
    let reverter = Address::from_low_u64_be(0x82_50_11);
    let accounts = [
        (
            FUNDED_SENDER,
            sender_balance(),
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (
            reverter,
            U256::zero(),
            0,
            Bytes::from(SSTORE_THEN_STOP_CODE.to_vec()),
        ),
    ];
    let tx = frame_tx_with_keys(
        vec![
            frame(FrameMode::Verify, 0x07, FUNDED_SENDER, 100_000, &[]),
            frame(FrameMode::Sender, 0x00, reverter, 100_000, &[]),
        ],
        vec![U256::zero()],
    );
    let (result, db) = run_frame_tx(&accounts, tx);
    assert!(
        result.is_err(),
        "payment approval inside an atomic batch must invalidate the tx; got {result:?}"
    );
    assert_eq!(
        nonce_of(&db, FUNDED_SENDER),
        0,
        "an invalidated tx must consume no nonce",
    );
}

// ==================== consumption survives a LATER batch revert ====================

#[test]
fn key0_consumption_survives_a_later_batch_revert() {
    // Legit shape: frame[0] VERIFY(scope=3, NO batch flag) grants payment and
    // consumes the key-0 nonce; frames[1..] are a SENDER atomic batch that
    // reverts. The payment frame is not in the batch, so its consumption is
    // outside the batch's revert scope and survives.
    let reverter = Address::from_low_u64_be(0x82_50_21);
    let accounts = [
        (
            FUNDED_SENDER,
            sender_balance(),
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (
            reverter,
            U256::zero(),
            0,
            Bytes::from(SSTORE_THEN_REVERT_CODE.to_vec()),
        ),
    ];
    let tx = frame_tx_with_keys(
        vec![
            frame(FrameMode::Verify, 0x03, FUNDED_SENDER, 100_000, &[]),
            frame(FrameMode::Sender, 0x04, reverter, 100_000, &[]),
            frame(FrameMode::Sender, 0x00, reverter, 100_000, &[]),
        ],
        vec![U256::zero()],
    );
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("payment is granted outside the batch, so the tx is valid");
    assert_eq!(report.payer_address, Some(FUNDED_SENDER));
    assert_eq!(
        nonce_of(&db, FUNDED_SENDER),
        1,
        "key-0 consumption from the non-batch payment frame survives the later batch revert",
    );
    assert!(
        storage_slot(&db, reverter, H256::zero()).is_zero(),
        "the reverted in-batch SSTORE must not survive",
    );
}

#[test]
fn keyed_nonce_consumption_survives_a_later_batch_revert() {
    // Same as above but with a non-zero nonce key, so consumption lands in
    // NONCE_MANAGER storage rather than the account nonce.
    let reverter = Address::from_low_u64_be(0x82_50_31);
    let accounts = [
        (
            FUNDED_SENDER,
            sender_balance(),
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (
            reverter,
            U256::zero(),
            0,
            Bytes::from(SSTORE_THEN_REVERT_CODE.to_vec()),
        ),
        nonce_manager_account(),
    ];
    let tx = frame_tx_with_keys(
        vec![
            frame(FrameMode::Verify, 0x03, FUNDED_SENDER, 100_000, &[]),
            frame(FrameMode::Sender, 0x04, reverter, 100_000, &[]),
            frame(FrameMode::Sender, 0x00, reverter, 100_000, &[]),
        ],
        vec![U256::one()],
    );
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("payment granted outside the batch keeps the tx valid");
    assert_eq!(report.payer_address, Some(FUNDED_SENDER));
    assert_eq!(
        storage_slot(
            &db,
            frame_tx_nonce_manager(),
            keyed_slot(FUNDED_SENDER, U256::one())
        ),
        U256::one(),
        "keyed-nonce consumption survives the later batch revert",
    );
    assert_eq!(
        nonce_of(&db, FUNDED_SENDER),
        0,
        "a non-zero key must not touch the sender's linear account nonce",
    );
}
