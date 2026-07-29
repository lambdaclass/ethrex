//! EIP-8272: recent-root native write + reference validity, exercised directly
//! through `execute_frame_tx` (bypassing the mempool/builder) so a failure
//! surfaces as the raw `VMError`.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, ChainConfig, Code, FRAME_TX_RECENT_ROOT_USABLE_WINDOW, Fork, Frame,
    FrameMode, FrameTransaction, RecentRootReference, Transaction, frame_tx_recent_root,
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

/// Run `tx` at `slot` against a predeploy pre-seeded with `committed`, so the
/// declared references resolve against real storage.
fn run_with_committed_roots(
    tx: FrameTransaction,
    committed: &[RecentRootReference],
    slot: u64,
) -> Result<ExecutionReport, VMError> {
    let accounts = [
        (SENDER, big(), 0, Bytes::from(APPROVE_BOTH_CODE.to_vec())),
        recent_root_predeploy(),
    ];
    let mut db = seeded_db(&accounts);
    if let Some(account) = db.current_accounts_state.get_mut(&frame_tx_recent_root()) {
        account.storage = committed
            .iter()
            .map(|entry| {
                (
                    entry.storage_key(),
                    U256::from_big_endian(entry.entry_hash().as_bytes()),
                )
            })
            .collect();
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
}

/// A data-heavy, execution-light frame tx: enough frame data that the EIP-7623
/// calldata floor exceeds what execution consumes, so the floor binds.
fn floor_bound_frame_tx() -> FrameTransaction {
    frame_tx(vec![
        frame(FrameMode::Verify, 0x03, SENDER, 100_000, &[0xFFu8; 2048]),
        frame(
            FrameMode::Sender,
            0x00,
            Address::from_low_u64_be(0xBEEF),
            30_000,
            &[],
        ),
    ])
}

/// source_id = keccak256(caller || salt) over the 20-byte address and 32-byte salt.
fn source_id(caller: Address, salt: &[u8; 32]) -> H256 {
    let mut pre = [0u8; 52];
    pre[..20].copy_from_slice(caller.as_bytes());
    pre[20..52].copy_from_slice(salt);
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
        fr[1].0,
        1,
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
    tx.recent_root_references = vec![entry];
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

/// EIP-7843 slot-derivation wiring (feedback #4): `EVMConfig::new_from_chain_config`
/// surfaces the effective beacon slot on `EVMConfig.slot_number`, which
/// `setup_env_with_config` copies into `env.slot_number`. The derivation formula
/// itself is unit-tested in `ethrex-common` (`effective_slot_number`) and the
/// write/reference logic in the tests above (with the slot set explicitly); this
/// asserts the config→env plumbing that makes 8272 functional when the CL does
/// not forward the slot (engine V3).
#[test]
fn evm_config_derives_slot_when_knob_active_and_cl_absent() {
    let config = ChainConfig {
        hegota_time: Some(0),
        derived_slot_time: Some(0),
        genesis_timestamp: Some(1000),
        seconds_per_slot: Some(6),
        ..Default::default()
    };
    // CL absent (engine V3): header carries no slot -> derived (1060-1000)/6 = 10.
    let header = BlockHeader {
        timestamp: 1060,
        slot_number: None,
        ..Default::default()
    };
    assert_eq!(
        EVMConfig::new_from_chain_config(&config, &header).slot_number,
        U256::from(10u64)
    );
    // CL present (engine V4): the supplied slot wins verbatim.
    let header_v4 = BlockHeader {
        timestamp: 1060,
        slot_number: Some(42),
        ..Default::default()
    };
    assert_eq!(
        EVMConfig::new_from_chain_config(&config, &header_v4).slot_number,
        U256::from(42u64)
    );
    // Knob absent -> 0 (unchanged behaviour on chains without the knob).
    let no_knob = ChainConfig {
        hegota_time: Some(0),
        ..Default::default()
    };
    assert_eq!(
        EVMConfig::new_from_chain_config(&no_knob, &header).slot_number,
        U256::zero()
    );
}

/// EIP-8272 reference gas is a mandatory cost, charged outside the EIP-7623
/// floored term. The floor is defined over frame and signature data only, and at
/// 64 gas per data byte it dominates `data_cost` (16 at most), so a floored
/// reference charge would be absorbed whole — silently free — even though the
/// warming it prepays happens unconditionally.
#[test]
fn reference_gas_is_charged_even_when_the_calldata_floor_binds() {
    let salt = [0x55u8; 32];
    let ref_slot = 300u64;
    let entry = RecentRootReference {
        source_id: source_id(SENDER, &salt),
        slot: ref_slot,
        root: H256::repeat_byte(0x66),
    };

    let baseline = floor_bound_frame_tx();
    let mut referencing = floor_bound_frame_tx();
    referencing.recent_root_references = vec![entry.clone()];
    let reference_gas = referencing.recent_root_reference_gas();
    assert!(reference_gas > 0, "one reference must cost something");

    // The floor must actually bind, or the test would pass for the wrong reason.
    let floor = baseline.calldata_floor_gas();
    assert!(
        floor > baseline.data_cost(),
        "floor {floor} must exceed data cost {} for this to test absorption",
        baseline.data_cost()
    );

    let without = run_with_committed_roots(baseline, &[], ref_slot + 1)
        .expect("reference-free tx must execute");
    let with = run_with_committed_roots(referencing, &[entry], ref_slot + 1)
        .expect("committed reference must validate and the tx execute");

    assert_eq!(
        with.gas_used,
        without.gas_used.saturating_add(reference_gas),
        "reference gas must survive the calldata floor"
    );
}

/// A reference that is merely not yet referenceable is transient — the next slot
/// resolves it — while an expired or uncommitted one is permanent. Block building
/// evicts a frame tx on any non-nonce-mismatch failure, so the two must not share
/// an error.
#[test]
fn unreferenceable_and_invalid_references_raise_distinct_errors() {
    let salt = [0x77u8; 32];
    let entry_slot = 400u64;
    let entry = RecentRootReference {
        source_id: source_id(SENDER, &salt),
        slot: entry_slot,
        root: H256::repeat_byte(0x88),
    };
    let referencing = || {
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
        tx
    };

    // Same slot as the write: referenceable only from the next slot on.
    let same_slot =
        run_with_committed_roots(referencing(), std::slice::from_ref(&entry), entry_slot)
            .expect_err("a reference to the current slot must fail");
    assert!(
        same_slot
            .to_string()
            .contains("not yet referenceable at this slot"),
        "transient failure must be distinguishable, got: {same_slot}"
    );

    // Ahead of the current slot: also transient.
    let future =
        run_with_committed_roots(referencing(), std::slice::from_ref(&entry), entry_slot - 1)
            .expect_err("a reference to a future slot must fail");
    assert!(
        future
            .to_string()
            .contains("not yet referenceable at this slot"),
        "a future reference must be transient, got: {future}"
    );

    // Past the usable window: permanent.
    let expired = run_with_committed_roots(
        referencing(),
        std::slice::from_ref(&entry),
        entry_slot + FRAME_TX_RECENT_ROOT_USABLE_WINDOW + 1,
    )
    .expect_err("a reference past the usable window must fail");
    assert!(
        expired.to_string().contains("expired or not committed"),
        "an expired reference must be permanent, got: {expired}"
    );

    // In window but never committed: permanent.
    let uncommitted = run_with_committed_roots(referencing(), &[], entry_slot + 1)
        .expect_err("an uncommitted reference must fail");
    assert!(
        uncommitted.to_string().contains("expired or not committed"),
        "an uncommitted reference must be permanent, got: {uncommitted}"
    );
}

/// Per EIP-7928, the reference-validity pass reads real predeploy storage, so
/// each reference's storage key belongs in the block access list as a read (the
/// pass never writes). Recorded only once the whole pass succeeds: a failed
/// reference invalidates the transaction and the block with it.
#[test]
fn valid_references_are_recorded_as_bal_storage_reads() {
    let salt = [0x99u8; 32];
    let ref_slot = 500u64;
    let entry = RecentRootReference {
        source_id: source_id(SENDER, &salt),
        slot: ref_slot,
        root: H256::repeat_byte(0xAA),
    };
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

    let mut db = seeded_db(&[
        (SENDER, big(), 0, Bytes::from(APPROVE_BOTH_CODE.to_vec())),
        recent_root_predeploy(),
    ]);
    db.enable_bal_recording();
    if let Some(account) = db.current_accounts_state.get_mut(&frame_tx_recent_root()) {
        account.storage = [(
            entry.storage_key(),
            U256::from_big_endian(entry.entry_hash().as_bytes()),
        )]
        .into_iter()
        .collect();
    }
    let env = Environment {
        origin: tx.sender,
        gas_limit: tx.total_gas_limit(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(CHAIN_ID),
        base_fee_per_gas: U256::from(1u64),
        gas_price: U256::from(tx.max_fee_per_gas),
        slot_number: U256::from(ref_slot + 1),
        tx_nonce: tx.nonce,
        ..Default::default()
    };
    let transaction = Transaction::FrameTransaction(tx);
    {
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
            .expect("committed reference must validate with BAL recording active");
    }

    let bal = db.take_bal().expect("BAL recorder was active");
    let account = bal
        .accounts()
        .iter()
        .find(|a| a.address == frame_tx_recent_root())
        .expect("RECENT_ROOT_ADDRESS must appear in the BlockAccessList");
    let key = U256::from_big_endian(entry.storage_key().as_bytes());
    assert!(
        account.storage_reads.contains(&key),
        "the reference's storage key must be recorded as a BAL read"
    );
    assert!(
        account.storage_changes.is_empty(),
        "the reference pass only reads; it must record no storage change"
    );
}

/// Pins the EIP-8272 `source_id` preimage. §Root sources defines
/// `source_id = keccak256(source_address ‖ salt)`, and the Specification
/// preamble fixes the encodings: "Addresses are 20 bytes", salts are 32. The
/// preimage is therefore 52 bytes with the address unpadded. The other tests
/// derive the expected `source_id` with the same helper the write path uses, so
/// they would pass under any self-consistent layout; this golden is what
/// actually detects a change to the layout itself.
#[test]
fn source_id_preimage_is_the_unpadded_address_and_salt() {
    let golden = H256::from_slice(
        &hex::decode("df7e44625a0cd6b99a54ec5c1c3ed8851f97629a88dcf861bf9ba2d1f13d15a9").unwrap(),
    );
    assert_eq!(source_id(SENDER, &[0x11u8; 32]), golden);
    // A 32-byte left-padded address would hash 64 bytes and give a different id.
    let mut padded = [0u8; 64];
    padded[12..32].copy_from_slice(SENDER.as_bytes());
    padded[32..64].copy_from_slice(&[0x11u8; 32]);
    assert_ne!(H256(ethrex_crypto::keccak::keccak_hash(padded)), golden);
}
