//! EIP-8272: recent-root native write + reference validity, exercised directly
//! through `execute_frame_tx` (bypassing the mempool/builder) so a failure
//! surfaces as the raw `VMError`.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, Code, Fork, Frame, FrameMode, FrameTransaction, RecentRootReference,
    Transaction, frame_tx_recent_root,
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

const CHAIN_ID: u64 = 1;
const SENDER: Address = Address::repeat_byte(0xAA);
fn big() -> U256 {
    U256::from(10u64).pow(U256::from(18u64))
}
/// APPROVE(scope=3): sender + payment; frame target must be the sender.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];

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

fn frame_tx(frames: Vec<Frame>) -> FrameTransaction {
    FrameTransaction {
        chain_id: CHAIN_ID,
        nonce: 0,
        sender: SENDER,
        frames,
        signatures: Vec::new(),
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: Vec::new(),
        recent_root_references: Vec::new(),
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

fn run_at_slot(
    accounts: &[SeededAccount],
    tx: FrameTransaction,
    slot: u64,
) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    run_at_slot_bal(accounts, tx, slot, false)
}

fn run_at_slot_bal(
    accounts: &[SeededAccount],
    tx: FrameTransaction,
    slot: u64,
    bal: bool,
) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    let mut db = seeded_db(accounts);
    if bal {
        // Match the block builder / import path, which records the EIP-7928
        // BlockAccessList while executing.
        db.enable_bal_recording();
    }
    let env = Environment {
        origin: tx.sender,
        gas_limit: tx.total_gas_limit(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(CHAIN_ID),
        base_fee_per_gas: U256::from(1u64),
        gas_price: U256::from(tx.max_fee_per_gas),
        slot_number: U256::from(slot),
        tx_nonce: tx.nonce,
        ..Default::default()
    };
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
        .expect("VM::new");
        vm.execute()
    };
    (result, db)
}

fn recent_root_predeploy() -> SeededAccount {
    (frame_tx_recent_root(), U256::zero(), 1, Bytes::new())
}

/// source_id = keccak256(pad32(caller) || salt).
fn source_id(caller: Address, salt: &[u8; 32]) -> H256 {
    let mut pre = [0u8; 64];
    pre[12..32].copy_from_slice(caller.as_bytes());
    pre[32..64].copy_from_slice(salt);
    H256(ethrex_crypto::keccak::keccak_hash(pre))
}

fn storage_slot(db: &GeneralizedDatabase, addr: Address, key: H256) -> U256 {
    db.current_accounts_state
        .get(&addr)
        .and_then(|a| a.storage.get(&key).copied())
        .unwrap_or_default()
}

#[test]
fn recent_root_native_write_commits_the_entry() {
    let salt = [0x11u8; 32];
    let root = H256::repeat_byte(0x22);
    let write_slot = 100u64;
    let accounts = [
        (SENDER, big(), 0, Bytes::from(APPROVE_BOTH_CODE.to_vec())),
        recent_root_predeploy(),
    ];
    // [VERIFY(approve) -> sender, SENDER(write) -> RECENT_ROOT with salt||root]
    let tx = frame_tx(vec![
        frame(FrameMode::Verify, 0x03, SENDER, 100_000, &[]),
        frame(
            FrameMode::Sender,
            0x00,
            frame_tx_recent_root(),
            100_000,
            &[salt.as_slice(), root.as_bytes()].concat(),
        ),
    ]);
    let (result, db) = run_at_slot(&accounts, tx, write_slot);
    let report = result.expect("write frame tx must execute (this is where the RPC path failed)");
    let fr = report.frame_results.expect("frame results");
    assert_eq!(
        fr[1].0, 1,
        "the recent-root write frame must succeed; statuses={:?}",
        fr.iter().map(|f| f.0).collect::<Vec<_>>()
    );
    // The predeploy must now hold entry_hash at storage_key for (source_id, write_slot).
    let sid = source_id(SENDER, &salt);
    let expected = RecentRootReference {
        source_id: sid,
        slot: write_slot,
        root,
    };
    let stored = storage_slot(&db, frame_tx_recent_root(), expected.storage_key());
    assert_eq!(
        stored,
        U256::from_big_endian(expected.entry_hash().as_bytes()),
        "committed entry hash mismatch",
    );
}

#[test]
fn recent_root_native_write_with_bal_recording() {
    // The block builder / import path executes with the EIP-7928 BAL recorder
    // active. Reproduce that here: the native write records a storage change on
    // 0x8272, so a bug in that recording (or a build() inconsistency) would make
    // the builder's execute_frame_tx fail and silently skip the tx.
    let salt = [0x55u8; 32];
    let root = H256::repeat_byte(0x66);
    let write_slot = 300u64;
    let accounts = [
        (SENDER, big(), 0, Bytes::from(APPROVE_BOTH_CODE.to_vec())),
        recent_root_predeploy(),
    ];
    let tx = frame_tx(vec![
        frame(FrameMode::Verify, 0x03, SENDER, 100_000, &[]),
        frame(
            FrameMode::Sender,
            0x00,
            frame_tx_recent_root(),
            100_000,
            &[salt.as_slice(), root.as_bytes()].concat(),
        ),
    ]);
    let (result, mut db) = run_at_slot_bal(&accounts, tx, write_slot, true);
    let report = result.expect("write frame tx must execute even with BAL recording active");
    let fr = report.frame_results.expect("frame results");
    assert_eq!(fr[1].0, 1, "write frame must succeed with BAL on");
    // The BAL must build without panicking and include the 0x8272 storage write.
    let bal = db.take_bal().expect("BAL recorder was active");
    let touched = bal
        .accounts()
        .iter()
        .any(|a| a.address == frame_tx_recent_root());
    assert!(
        touched,
        "RECENT_ROOT_ADDRESS must appear in the BlockAccessList"
    );
}

#[test]
fn committed_reference_validates_and_executes() {
    // Pre-seed the predeploy with a committed root, then reference it from a
    // later slot; the tx must pass the validity check and execute.
    let salt = [0x33u8; 32];
    let root = H256::repeat_byte(0x44);
    let ref_slot = 200u64;
    let sid = source_id(SENDER, &salt);
    let entry = RecentRootReference {
        source_id: sid,
        slot: ref_slot,
        root,
    };
    let mut predeploy_storage = FxHashMap::default();
    predeploy_storage.insert(
        entry.storage_key(),
        U256::from_big_endian(entry.entry_hash().as_bytes()),
    );
    let db_accounts = [
        (SENDER, big(), 0, Bytes::from(APPROVE_BOTH_CODE.to_vec())),
        (frame_tx_recent_root(), U256::zero(), 1, Bytes::new()),
    ];
    // Manually seed the predeploy storage (seeded_db uses empty storage), so
    // build the db and inject the committed slot.
    let mut db = seeded_db(&db_accounts);
    if let Some(acc) = db.current_accounts_state.get_mut(&frame_tx_recent_root()) {
        acc.storage = predeploy_storage;
    }
    let mut tx = frame_tx(vec![
        frame(FrameMode::Verify, 0x03, SENDER, 100_000, &[]),
        frame(
            FrameMode::Sender,
            0x00,
            Address::from_low_u64_be(0xBEEF),
            30_000,
            &[],
        ),
    ]);
    tx.recent_root_references = vec![entry.clone()];
    let env = Environment {
        origin: tx.sender,
        gas_limit: tx.total_gas_limit(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(CHAIN_ID),
        base_fee_per_gas: U256::from(1u64),
        gas_price: U256::from(tx.max_fee_per_gas),
        // reference at slot ref_slot + 1: age 1 is inside the usable window.
        slot_number: U256::from(ref_slot + 1),
        tx_nonce: tx.nonce,
        ..Default::default()
    };
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
        .expect("VM::new");
        vm.execute()
    };
    let report = result.expect("committed reference must validate and the tx execute");
    assert_eq!(report.payer_address, Some(SENDER));
}
