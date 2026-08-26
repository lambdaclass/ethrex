//! EIP-8272: the recent-root predeploy and reference validity, exercised
//! directly through `execute_frame_tx` and plain calls (bypassing the
//! mempool/builder) so a failure surfaces as the raw `VMError`.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, AccountState, BlockHeader, ChainConfig, Code, CodeMetadata, EIP1559Transaction,
    FRAME_TX_RECENT_ROOT_USABLE_WINDOW, Fork, Frame, FrameMode, FrameTransaction,
    RecentRootReference, Transaction, TxKind, frame_tx_recent_root,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::Database as LevmDatabase;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::DatabaseError;
use ethrex_levm::errors::{ExecutionReport, VMError};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use ethrex_vm::backends::levm::LEVM;
use ethrex_vm::system_contracts::RECENT_ROOT_RUNTIME_BYTECODE;
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
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
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

/// A state budget covering one `RECENT_ROOT_CODE` entry write (a new storage slot in
/// the predeploy, ~126k of EIP-8037 state gas) with room to spare. EIP-8141 v2 frames
/// declare `limits.state` and a charge past it halts the frame, so a write frame that
/// declared nothing would revert; read-only frames spend none of it and it is refunded.
const STATE_BUDGET: u64 = 1_000_000;

fn frame(mode: FrameMode, flags: u8, target: Address, gas_limit: u64, data: &[u8]) -> Frame {
    Frame {
        mode: u8::from(mode),
        flags,
        target: Some(target),
        gas_limit,
        state_limit: STATE_BUDGET,
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
        tx_nonce: tx.nonce_seq,
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
            None,
        )
        .expect("VM::new");
        vm.execute()
    };
    (result, db)
}

/// The predeploy carries `RECENT_ROOT_CODE`, so a frame targeting it runs that bytecode. Seeding it
/// codeless would instead take the EIP-8141 default-code path, which returns success without writing.
fn recent_root_predeploy() -> SeededAccount {
    (
        frame_tx_recent_root(),
        U256::zero(),
        1,
        Bytes::from_static(&RECENT_ROOT_RUNTIME_BYTECODE),
    )
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
        tx_nonce: tx.nonce_seq,
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
        None,
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
fn a_frame_targeting_the_predeploy_commits_the_entry() {
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
            300_000,
            &[salt.as_slice(), root.as_bytes()].concat(),
        ),
    ]);
    let (result, db) = run_at_slot(&accounts, tx, write_slot);
    let report = result.expect("write frame tx must execute (this is where the RPC path failed)");
    let fr = report.frame_results.expect("frame results");
    assert_eq!(
        fr[1].status,
        1,
        "the recent-root write frame must succeed; statuses={:?}",
        fr.iter().map(|f| f.status).collect::<Vec<_>>()
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
fn a_frame_write_is_recorded_in_the_block_access_list() {
    // The block builder / import path executes with the EIP-7928 BAL recorder
    // active. Reproduce that here: the write records a storage change on
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
            300_000,
            &[salt.as_slice(), root.as_bytes()].concat(),
        ),
    ]);
    let (result, mut db) = run_at_slot_bal(&accounts, tx, write_slot, true);
    let report = result.expect("write frame tx must execute even with BAL recording active");
    let fr = report.frame_results.expect("frame results");
    assert_eq!(fr[1].status, 1, "write frame must succeed with BAL on");
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

/// EIP-8141 v2: "since the signature validation does not happen in EVM execution, the
/// related precompiles `ecrecover` and `P256VERIFY` must not be added to the block-level
/// access list."
///
/// ethrex satisfies this by construction — outer signatures are checked by a direct call
/// to `validate_frame_signatures`, never through EVM dispatch, so the BAL recorder never
/// sees the precompile addresses. That is exactly the kind of property that holds until
/// someone routes the check through the EVM for convenience, and an extra BAL entry
/// changes the list's hash and invalidates the block. Pinned here, alongside the positive
/// case above, so the two are read together: 0x8272 *must* appear, 0x01 and 0x100 must
/// not.
#[test]
fn signature_precompiles_stay_out_of_the_block_access_list() {
    let salt = [0x77u8; 32];
    let root = H256::repeat_byte(0x88);
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
            300_000,
            &[salt.as_slice(), root.as_bytes()].concat(),
        ),
    ]);
    let (result, mut db) = run_at_slot_bal(&accounts, tx, 400u64, true);
    result.expect("the transaction executes with BAL recording active");
    let bal = db.take_bal().expect("BAL recorder was active");

    let ecrecover = Address::from_low_u64_be(0x01);
    let p256verify = Address::from_low_u64_be(0x100);
    for (address, name) in [(ecrecover, "ecrecover"), (p256verify, "P256VERIFY")] {
        assert!(
            !bal.accounts().iter().any(|a| a.address == address),
            "{name} must not appear in the BlockAccessList: outer signature validation \
             happens outside EVM execution"
        );
    }
    assert!(
        bal.accounts()
            .iter()
            .any(|a| a.address == frame_tx_recent_root()),
        "the EIP-8272 predeploy is a deliberate BAL entry and must still be recorded"
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
        tx_nonce: tx.nonce_seq,
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
            None,
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

/// EIP-8272 splits its gas in two: `recent_root_reference_intrinsic_gas` is a
/// mandatory cost that enters both `standard_gas_limit` and `calldata_floor_gas`,
/// while `rlp(recent_root_references)` is ordinary transaction data whose tokens
/// enter `calldata_tokens`. With the floor binding, the charge is therefore the
/// intrinsic gas plus the reference bytes at the floor rate, never absorbed and
/// never billed twice.
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
    assert!(
        referencing.recent_root_reference_intrinsic_gas() > 0,
        "one reference must cost something"
    );

    // The floor must actually bind, or the test would pass for the wrong reason.
    let floor = baseline.calldata_floor_gas();
    assert!(
        floor > baseline.data_cost(),
        "floor {floor} must exceed data cost {} for this to test absorption",
        baseline.data_cost()
    );

    // Under a binding floor `gas_used` is `calldata_floor_total()`, so the whole
    // EIP-8272 delta is the difference between the two transactions' floors.
    let expected_delta = referencing
        .calldata_floor_total()
        .saturating_sub(baseline.calldata_floor_total());
    assert!(
        expected_delta > referencing.recent_root_reference_intrinsic_gas(),
        "the reference bytes must add to the floor on top of the intrinsic gas"
    );

    let without = run_with_committed_roots(baseline, &[], ref_slot + 1)
        .expect("reference-free tx must execute");
    let with = run_with_committed_roots(referencing, &[entry], ref_slot + 1)
        .expect("committed reference must validate and the tx execute");

    assert_eq!(
        with.gas_used,
        without.gas_used.saturating_add(expected_delta),
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
        tx_nonce: tx.nonce_seq,
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
            None,
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

// ==================== Predeploy call behaviour ====================
//
// EIP-8272 §"Recent root contract". The predeploy carries real runtime code, so
// these are ordinary EVM calls rather than frame transactions: the write is
// reachable from a plain EOA transaction, and the spec's two prohibitions
// (static context, `DELEGATECALL`/`CALLCODE`) fall out of the EVM rather than
// from an explicit check in the code.

const CALLER: Address = Address::repeat_byte(0xC0);
/// Slot the wrapper contracts below store the inner call's success flag in.
/// A keccak-derived recent-root storage key cannot collide with it.
fn success_slot() -> H256 {
    let mut key = H256::zero();
    key.0[30..].copy_from_slice(&0xFFFFu16.to_be_bytes());
    key
}

/// `calldatacopy(0, 0, 64)` — stage the 64-byte `salt ‖ root` argument for the
/// wrappers below.
const STAGE_ARGS: &[u8] = &[0x60, 0x40, 0x60, 0x00, 0x60, 0x00, 0x37];

/// `sstore(0xffff, success); stop`
const RECORD_SUCCESS: &[u8] = &[0x61, 0xff, 0xff, 0x55, 0x00];

/// Forward the call to the predeploy with `DELEGATECALL`, then record whether it
/// succeeded.
fn delegatecall_wrapper() -> Bytes {
    let call = [
        0x60, 0x00, // retSize
        0x60, 0x00, // retOffset
        0x60, 0x40, // argsSize
        0x60, 0x00, // argsOffset
        0x61, 0x82, 0x72, // address
        0x5a, // gas
        0xf4, // DELEGATECALL
    ];
    Bytes::from([STAGE_ARGS, &call, RECORD_SUCCESS].concat())
}

/// Forward the call to the predeploy with `STATICCALL`, then record whether it
/// succeeded.
fn staticcall_wrapper() -> Bytes {
    let call = [
        0x60, 0x00, // retSize
        0x60, 0x00, // retOffset
        0x60, 0x40, // argsSize
        0x60, 0x00, // argsOffset
        0x61, 0x82, 0x72, // address
        0x5a, // gas
        0xfa, // STATICCALL
    ];
    Bytes::from([STAGE_ARGS, &call, RECORD_SUCCESS].concat())
}

/// `CALL` the predeploy, then revert the whole frame.
fn reverting_wrapper() -> Bytes {
    let call = [
        0x60, 0x00, // retSize
        0x60, 0x00, // retOffset
        0x60, 0x40, // argsSize
        0x60, 0x00, // argsOffset
        0x60, 0x00, // value
        0x61, 0x82, 0x72, // address
        0x5a, // gas
        0xf1, // CALL
        0x50, // POP success
        0x60, 0x00, 0x60, 0x00, 0xfd, // revert(0, 0)
    ];
    Bytes::from([STAGE_ARGS, &call].concat())
}

/// Run a plain EIP-1559 transaction — no frame transaction, no EIP-8141 path.
fn run_plain_call(
    accounts: &[SeededAccount],
    to: Address,
    data: &[u8],
    slot: u64,
) -> (ExecutionReport, GeneralizedDatabase) {
    let mut db = seeded_db(accounts);
    let env = Environment {
        origin: SENDER,
        gas_limit: 1_000_000,
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(CHAIN_ID),
        base_fee_per_gas: U256::from(1u64),
        gas_price: U256::from(1_000u64),
        tx_max_fee_per_gas: Some(U256::from(1_000u64)),
        tx_max_priority_fee_per_gas: Some(U256::from(1u64)),
        slot_number: U256::from(slot),
        ..Default::default()
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(to),
        value: U256::zero(),
        data: Bytes::from(data.to_vec()),
        gas_limit: 1_000_000,
        max_fee_per_gas: 1_000,
        max_priority_fee_per_gas: 1,
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
            None,
        )
        .expect("VM::new");
        vm.execute().expect("plain call must execute")
    };
    (report, db)
}

/// The entry a successful write by `caller` at `slot` commits.
fn committed_entry(caller: Address, salt: &[u8; 32], root: H256, slot: u64) -> RecentRootReference {
    RecentRootReference {
        source_id: source_id(caller, salt),
        slot,
        root,
    }
}

#[test]
fn plain_eoa_call_writes_the_entry() {
    // The predeploy is callable like any other contract: `to = 0x…8272` with
    // `salt ‖ root` as calldata commits the entry. Before the predeploy carried
    // real code this call was a silent no-op, which is the divergence the swap
    // removes.
    let salt = [0x11u8; 32];
    let root = H256::repeat_byte(0x22);
    let write_slot = 100u64;
    let accounts = [(SENDER, big(), 0, Bytes::new()), recent_root_predeploy()];
    let calldata = [salt.as_slice(), root.as_bytes()].concat();

    let (report, db) = run_plain_call(&accounts, frame_tx_recent_root(), &calldata, write_slot);

    assert!(report.is_success(), "the write call must succeed");
    let entry = committed_entry(SENDER, &salt, root, write_slot);
    assert_eq!(
        storage_slot(&db, frame_tx_recent_root(), entry.storage_key()),
        U256::from_big_endian(entry.entry_hash().as_bytes()),
        "committed entry hash mismatch",
    );
    // All 64 calldata bytes are non-zero here, the most expensive shape. Each
    // zero byte in `salt ‖ root` takes 12 gas off (EIP-7623 charges a zero byte
    // one token against a non-zero byte's four), and execution is far above the
    // calldata floor, so the floor never binds. Pinned so a repricing that moves
    // the write shows up here rather than at bring-up.
    //
    // 126_356 on the glamsterdam-devnet-8 base, 127_256 before it: the EIP-8038 v8.1.0
    // schedule prices a cold storage access at 2100 where the earlier draft charged 3000,
    // and the write touches one cold slot. This pin caught that reprice, which is exactly
    // what it exists for -- see the divergence ledger for the consensus consequence.
    assert_eq!(report.gas_used, 126_356, "measured write gas");
}

#[test]
fn delegatecall_writes_the_callers_storage_and_leaves_the_predeploy_untouched() {
    // "Only a direct call to RECENT_ROOT_ADDRESS can write recent-root storage.
    // DELEGATECALL and CALLCODE MUST NOT write recent-root storage." No explicit
    // guard is needed: the SSTORE runs, but in the delegating account's storage,
    // so recent-root storage is unchanged and the entry it produces is not
    // referenceable.
    let salt = [0x33u8; 32];
    let root = H256::repeat_byte(0x44);
    let write_slot = 150u64;
    let accounts = [
        (SENDER, big(), 0, Bytes::new()),
        (CALLER, U256::zero(), 1, delegatecall_wrapper()),
        recent_root_predeploy(),
    ];
    let calldata = [salt.as_slice(), root.as_bytes()].concat();

    let (report, db) = run_plain_call(&accounts, CALLER, &calldata, write_slot);
    assert!(report.is_success(), "the wrapper must not revert");
    assert_eq!(
        storage_slot(&db, CALLER, success_slot()),
        U256::one(),
        "the delegated call itself succeeds"
    );

    // `CALLER` inside delegated code is the EOA, so the entry is the one a
    // direct call would have committed — it just lands in the wrong account.
    let entry = committed_entry(SENDER, &salt, root, write_slot);
    assert_eq!(
        storage_slot(&db, frame_tx_recent_root(), entry.storage_key()),
        U256::zero(),
        "recent-root storage must be untouched by a DELEGATECALL"
    );
    assert_eq!(
        storage_slot(&db, CALLER, entry.storage_key()),
        U256::from_big_endian(entry.entry_hash().as_bytes()),
        "the write lands in the delegating account's storage"
    );
}

#[test]
fn staticcall_fails_and_writes_nothing() {
    // "In static context, the write MUST fail and storage MUST remain
    // unchanged." The SSTORE raises, so the STATICCALL returns 0.
    let salt = [0x55u8; 32];
    let root = H256::repeat_byte(0x66);
    let write_slot = 175u64;
    let accounts = [
        (SENDER, big(), 0, Bytes::new()),
        (CALLER, U256::zero(), 1, staticcall_wrapper()),
        recent_root_predeploy(),
    ];
    let calldata = [salt.as_slice(), root.as_bytes()].concat();

    let (report, db) = run_plain_call(&accounts, CALLER, &calldata, write_slot);
    assert!(report.is_success(), "the wrapper itself must not revert");
    assert_eq!(
        storage_slot(&db, CALLER, success_slot()),
        U256::zero(),
        "the static call must fail"
    );
    let entry = committed_entry(SENDER, &salt, root, write_slot);
    assert_eq!(
        storage_slot(&db, frame_tx_recent_root(), entry.storage_key()),
        U256::zero(),
        "storage must remain unchanged"
    );
}

#[test]
fn a_write_inside_a_reverting_frame_rolls_back() {
    // The write is ordinary EVM state, so it is subject to ordinary revert
    // semantics. A reference to a rolled-back entry must not validate, or a
    // reverted block would leave referenceable roots behind.
    let salt = [0x77u8; 32];
    let root = H256::repeat_byte(0x88);
    let write_slot = 200u64;
    let accounts = [
        (SENDER, big(), 0, Bytes::new()),
        (CALLER, U256::zero(), 1, reverting_wrapper()),
        recent_root_predeploy(),
    ];
    let calldata = [salt.as_slice(), root.as_bytes()].concat();

    let (report, db) = run_plain_call(&accounts, CALLER, &calldata, write_slot);
    assert!(!report.is_success(), "the outer frame reverts");
    let entry = committed_entry(SENDER, &salt, root, write_slot);
    assert_eq!(
        storage_slot(&db, frame_tx_recent_root(), entry.storage_key()),
        U256::zero(),
        "the write must roll back with the frame that made it"
    );
}

// ==================== Predeploy activation ====================
//
// EIP-8272 §Activation. `install_recent_root_code` runs on both the
// payload-build path (`Evm::apply_system_calls`) and the block-import path
// (`LEVM::prepare_block`); the two must agree or builder and importer compute
// different state roots. The three cases the spec distinguishes are pinned
// here, along with idempotence and BAL recording.

/// A store that answers every account except `RECENT_ROOT_ADDRESS` as absent,
/// so the parent state of the predeploy can be set per test.
struct ParentState(AccountState);

impl LevmDatabase for ParentState {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        Ok(if address == frame_tx_recent_root() {
            self.0
        } else {
            AccountState::default()
        })
    }
    fn get_storage_value(&self, _address: Address, _key: H256) -> Result<U256, DatabaseError> {
        Ok(U256::zero())
    }
    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig::default())
    }
    fn get_account_code(&self, _code_hash: H256) -> Result<Code, DatabaseError> {
        Ok(Code::default())
    }
    fn get_code_metadata(&self, _code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        Ok(CodeMetadata { length: 0 })
    }
}

fn db_with_parent_state(state: AccountState) -> GeneralizedDatabase {
    GeneralizedDatabase::new(Arc::new(ParentState(state)))
}

fn installed_predeploy(db: &GeneralizedDatabase) -> ethrex_levm::account::LevmAccount {
    db.current_accounts_state
        .get(&frame_tx_recent_root())
        .cloned()
        .expect("the predeploy account must exist after install")
}

fn installed_code(db: &GeneralizedDatabase) -> Bytes {
    let hash = installed_predeploy(db).info.code_hash;
    db.codes
        .get(&hash)
        .expect("the installed code must be registered")
        .code_bytes()
}

#[test]
fn install_creates_an_absent_predeploy_with_nonce_one() {
    let mut db = db_with_parent_state(AccountState::default());
    LEVM::install_recent_root_code(&mut db, &NativeCrypto).expect("install");

    let account = installed_predeploy(&db);
    assert_eq!(account.info.nonce, 1);
    assert_eq!(account.info.balance, U256::zero());
    assert_eq!(installed_code(&db), RECENT_ROOT_RUNTIME_BYTECODE.as_slice());
}

#[test]
fn install_adopts_an_empty_account_and_preserves_its_balance() {
    // An EOA may have sent value to 0x8272 before the fork, and may have sent
    // transactions from it. The spec adopts such an account rather than
    // rejecting it: set the code, take nonce = max(existing, 1), preserve the
    // balance. Destroying either would burn user funds or re-open already-used
    // CREATE addresses.
    let squatted = U256::from(12_345u64);
    let mut db = db_with_parent_state(AccountState {
        balance: squatted,
        nonce: 9,
        ..AccountState::default()
    });
    LEVM::install_recent_root_code(&mut db, &NativeCrypto).expect("install");

    let account = installed_predeploy(&db);
    assert_eq!(account.info.balance, squatted, "balance must be preserved");
    assert_eq!(account.info.nonce, 9, "nonce must never be lowered");
    assert_eq!(installed_code(&db), RECENT_ROOT_RUNTIME_BYTECODE.as_slice());
}

#[test]
fn install_rejects_a_parent_state_with_code_or_storage() {
    // "The fork configuration MUST choose a RECENT_ROOT_ADDRESS with empty code
    // and empty storage in the parent state of the first post-fork payload. If
    // this condition is false at activation, the payload is invalid." Silently
    // overwriting would destroy the squatter's state and diverge from a client
    // that rejects.
    let foreign_code = Code::from_bytecode(Bytes::from_static(&[0x00]), &NativeCrypto);
    let mut with_code = db_with_parent_state(AccountState {
        code_hash: foreign_code.hash,
        ..AccountState::default()
    });
    LEVM::install_recent_root_code(&mut with_code, &NativeCrypto)
        .expect_err("non-empty code in the parent state must invalidate the payload");

    let mut with_storage = db_with_parent_state(AccountState {
        storage_root: H256::repeat_byte(0x77),
        ..AccountState::default()
    });
    LEVM::install_recent_root_code(&mut with_storage, &NativeCrypto)
        .expect_err("non-empty storage in the parent state must invalidate the payload");
}

#[test]
fn install_is_idempotent() {
    // Exactly one account update, at the first Hegota block and none afterwards.
    // A second install that touched the account would put a spurious predeploy
    // entry in every block's access list.
    let mut db = db_with_parent_state(AccountState::default());
    LEVM::install_recent_root_code(&mut db, &NativeCrypto).expect("first install");
    let after_first = installed_predeploy(&db);

    db.enable_bal_recording();
    LEVM::install_recent_root_code(&mut db, &NativeCrypto).expect("second install");
    let after_second = installed_predeploy(&db);

    assert_eq!(after_first.info.code_hash, after_second.info.code_hash);
    assert_eq!(after_first.info.nonce, after_second.info.nonce);
    assert_eq!(after_first.info.balance, after_second.info.balance);
    let bal = db.take_bal().expect("BAL recorder was active");
    assert!(
        !bal.accounts()
            .iter()
            .any(|a| a.address == frame_tx_recent_root()),
        "a repeat install must not record a change"
    );
}

#[test]
fn install_records_the_code_and_nonce_in_the_block_access_list() {
    // EIP-7928 parallel import rebuilds post-state from the BAL, so an install
    // that skips recording produces a state root the parallel path cannot match.
    let mut db = db_with_parent_state(AccountState::default());
    db.enable_bal_recording();

    LEVM::install_recent_root_code(&mut db, &NativeCrypto).expect("install");

    let bal = db.take_bal().expect("BAL recorder was active");
    let changes = bal
        .accounts()
        .iter()
        .find(|a| a.address == frame_tx_recent_root())
        .expect("the BAL must mention the predeploy");
    assert!(
        !changes.code_changes.is_empty(),
        "the code change must be recorded"
    );
    assert!(
        !changes.nonce_changes.is_empty(),
        "the nonce change must be recorded"
    );
}
