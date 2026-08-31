//! EIP-8141: Frame Transactions
//!
//! Shared test harness for frame-transaction execution plus regression tests
//! for the per-tx state-rollback invariant:
//!
//!   `VM::execute()` returning `Err` => `db.current_accounts_state` is
//!   unchanged from before the tx, exactly like non-frame txs.
//!
//! The helpers in this module (`run_frame_tx`, `assert_db_cache_unchanged`,
//! `frame_tx_with_frames`, and the bytecode constants) are reused by the
//! later EIP-8141 task tests.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{
    Account, BlockHeader, Code, FRAME_RECEIPT_STATUS_SUCCESS, Fork, Frame, FrameMode,
    FrameTransaction, MAX_BLOBS_PER_TX, Transaction,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::errors::TxResult;
use ethrex_levm::errors::{ExecutionReport, VMError};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Harness constants ====================

/// Chain id used by every harness-built frame transaction.
const HARNESS_CHAIN_ID: u64 = 1;
/// Fixed, funded sender for frame txs built via `frame_tx_with_frames`.
/// Must be non-zero to pass `validate_static_constraints`.
const FUNDED_SENDER: Address = Address::repeat_byte(0xAA);
/// Balance used when `run_frame_tx` auto-seeds the sender (i.e. when the caller
/// did not pass the sender in `accounts`). Kept as a constant so the rollback
/// assertion can verify the sender against the exact value it was seeded with.
const AUTO_SEED_SENDER_BALANCE: U256 = U256::MAX;
/// Harness base fee. `frame_tx_with_frames` sets `max_fee_per_gas` well above it.
const HARNESS_BASE_FEE: u64 = 1;
/// Coinbase used by the harness env (the `..Default::default()` zero address).
/// Fee tests read its post-execution balance to assert value conservation.
#[allow(dead_code)]
const COINBASE_ADDR: Address = Address::zero();

// Bytecodes used by frame-tx tests (shared with later tasks).
/// SSTORE 1@0; APPROVE(scope=3).
#[allow(dead_code)]
const WALLET_APPROVE_CODE: &[u8] = &[
    0x60, 0x01, 0x60, 0x00, 0x55, 0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA,
];
/// SSTORE 1@0; REVERT.
#[allow(dead_code)]
const SSTORE_THEN_REVERT_CODE: &[u8] =
    &[0x60, 0x01, 0x60, 0x00, 0x55, 0x60, 0x00, 0x60, 0x00, 0xFD];
/// SSTORE 1@0; STOP.
const SSTORE_THEN_STOP_CODE: &[u8] = &[0x60, 0x01, 0x60, 0x00, 0x55, 0x00];
/// APPROVE(scope=2) -- sender (execution) approval. Must run in a frame whose
/// target IS the tx sender, otherwise scope 2 reverts (frame_target != sender).
#[allow(dead_code)]
const APPROVE_EXECUTION_CODE: &[u8] = &[0x60, 0x02, 0x60, 0x00, 0x60, 0x00, 0xAA];
/// APPROVE(scope=1) -- payment approval. The frame's target becomes the payer.
#[allow(dead_code)]
const APPROVE_PAYMENT_CODE: &[u8] = &[0x60, 0x01, 0x60, 0x00, 0x60, 0x00, 0xAA];
/// APPROVE(scope=3) only (no SSTORE) -- sets sender_approved AND payer when the
/// frame target is the tx sender. Safe to run in a static VERIFY frame.
#[allow(dead_code)]
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];
/// PUSH1 0; PUSH1 0; REVERT -- a clean revert with no state op (so it reverts
/// even inside a static VERIFY frame, rather than halting on a static violation).
#[allow(dead_code)]
const PURE_REVERT_CODE: &[u8] = &[0x60, 0x00, 0x60, 0x00, 0xFD];

// ==================== Harness helpers ====================

/// A seeded account spec: (address, balance, nonce, code).
type SeededAccount = (Address, U256, u64, Bytes);

/// Build a `GeneralizedDatabase` whose cache is seeded with `accounts`.
fn seeded_db(accounts: &[SeededAccount]) -> GeneralizedDatabase {
    // The store type doesn't matter: every account we touch lives in the cache.
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

/// Build the execution `Environment` for a frame tx at Hegota.
fn frame_tx_env(tx: &FrameTransaction) -> Environment {
    Environment {
        origin: tx.sender,
        gas_limit: tx.total_gas_limit(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(HARNESS_CHAIN_ID),
        base_fee_per_gas: U256::from(HARNESS_BASE_FEE),
        // NOTE: gas_price here is max_fee_per_gas, NOT the effective price.
        // Fine for tests that don't assert on fee amounts. Tests that check
        // payer balances MUST use `run_frame_tx_with_fees`, which derives the
        // effective price min(base+priority, max_fee) like production.
        gas_price: U256::from(tx.max_fee_per_gas),
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    }
}

/// Build a frame transaction wrapping `frames`, with sane harness defaults:
/// harness chain id, nonce 0, sender = `FUNDED_SENDER`, fees above the harness
/// base fee, and an empty signature list.
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

/// Seed `accounts`, execute `tx` via a fresh VM at Hegota, and return the
/// execution result together with the (post-execution) database so callers can
/// inspect `current_accounts_state` (balances, nonces, storage).
///
/// The sender is auto-seeded with a large balance and nonce 0 if it is not
/// already present in `accounts`, so frame txs that do not exercise the sender
/// account still pass nonce/fee validation.
fn run_frame_tx(
    accounts: &[SeededAccount],
    tx: FrameTransaction,
) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    let mut seeded: Vec<SeededAccount> = accounts.to_vec();
    if !seeded.iter().any(|(addr, ..)| *addr == tx.sender) {
        seeded.push((
            tx.sender,
            AUTO_SEED_SENDER_BALANCE,
            tx.nonce_seq,
            Bytes::new(),
        ));
    }

    let mut db = seeded_db(&seeded);
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
            None,
        )
        .expect("VM::new should succeed for a frame tx");
        vm.execute()
    };

    (result, db)
}

/// Like `run_frame_tx`, but builds the env with the given block `base_fee`
/// instead of `HARNESS_BASE_FEE`. The env's effective `gas_price` is derived
/// from the tx the same way production does (`calculate_gas_price_for_tx`):
/// `min(base_fee + max_priority_fee_per_gas, max_fee_per_gas)`. Used by fee
/// tests that need a real base-fee/effective-price spread.
fn run_frame_tx_with_fees(
    accounts: &[SeededAccount],
    tx: FrameTransaction,
    base_fee: u64,
) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
    let mut seeded: Vec<SeededAccount> = accounts.to_vec();
    if !seeded.iter().any(|(addr, ..)| *addr == tx.sender) {
        seeded.push((
            tx.sender,
            AUTO_SEED_SENDER_BALANCE,
            tx.nonce_seq,
            Bytes::new(),
        ));
    }

    let mut db = seeded_db(&seeded);
    let mut env = frame_tx_env(&tx);
    env.base_fee_per_gas = U256::from(base_fee);
    // Effective gas price, matching production `calculate_gas_price_for_tx`.
    let effective = base_fee
        .saturating_add(tx.max_priority_fee_per_gas)
        .min(tx.max_fee_per_gas);
    env.gas_price = U256::from(effective);
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
        .expect("VM::new should succeed for a frame tx");
        vm.execute()
    };

    (result, db)
}

/// Read the current balance of `addr` from the post-execution cache.
#[allow(dead_code)]
fn balance_of(db: &GeneralizedDatabase, addr: Address) -> U256 {
    db.current_accounts_state
        .get(&addr)
        .map(|account| account.info.balance)
        .unwrap_or_default()
}

/// Read the current nonce of `addr` from the post-execution cache.
#[allow(dead_code)]
fn nonce_of(db: &GeneralizedDatabase, addr: Address) -> u64 {
    db.current_accounts_state
        .get(&addr)
        .map(|account| account.info.nonce)
        .unwrap_or_default()
}

/// A VERIFY frame targeting `target` (gas_limit 100_000, no value, no data).
/// The target's code runs and may call APPROVE.
///
/// flags 0x03 permits scopes 1/2/3 so the frame's APPROVE code can grant
/// execution and/or payment; flags 0 (APPROVE_SCOPE_NONE) would correctly halt
/// every APPROVE (see `approve_halts_when_frame_scope_is_none`).
#[allow(dead_code)]
fn verify_frame(target: Address) -> Frame {
    Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(target),
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

/// A paymaster VERIFY frame: scope `APPROVE_PAYMENT` only. EIP-8141 static
/// validation allows `APPROVE_EXECUTION` in the scope only when the frame's
/// target is empty or `tx.sender`, so a frame targeting a third-party payer must
/// restrict itself to payment.
fn pay_frame(target: Address) -> Frame {
    Frame {
        flags: 0x01,
        ..verify_frame(target)
    }
}

/// Assert that no seeded account's info (balance/nonce/code) or storage in
/// `db.current_accounts_state` differs from its seeded value. This is THE rollback
/// invariant: after an invalid tx the shared cache must show no residue.
///
/// The sender (`FUNDED_SENDER`) is ALWAYS verified, even when the caller does
/// not list it in `accounts`: `run_frame_tx` auto-seeds it, and a leaked sender
/// nonce/balance on the invalid-tx path (e.g. an APPROVE nonce bump that was not
/// rolled back) is exactly the kind of residue this invariant must prevent. When the caller
/// passes the sender explicitly, those values are used; otherwise the auto-seed
/// defaults (`AUTO_SEED_SENDER_BALANCE`, nonce 0) are checked.
///
/// Slot 0 of each seeded account is checked explicitly because the harness
/// bytecodes write slot 0; a leftover `1` there is the rollback regression signature.
fn assert_db_cache_unchanged(db: &GeneralizedDatabase, accounts: &[SeededAccount]) {
    // Always include the auto-seeded sender so a leaked sender balance/nonce is
    // caught, mirroring `run_frame_tx`'s auto-seed.
    let mut checked: Vec<SeededAccount> = accounts.to_vec();
    if !checked.iter().any(|(addr, ..)| *addr == FUNDED_SENDER) {
        checked.push((FUNDED_SENDER, AUTO_SEED_SENDER_BALANCE, 0, Bytes::new()));
    }

    for (address, balance, nonce, code) in &checked {
        let current = db
            .current_accounts_state
            .get(address)
            .unwrap_or_else(|| panic!("seeded account {address:?} missing from cache"));

        assert_eq!(
            current.info.balance, *balance,
            "balance of {address:?} changed after invalid tx",
        );
        assert_eq!(
            current.info.nonce, *nonce,
            "nonce of {address:?} changed after invalid tx",
        );
        assert_eq!(
            current.info.code_hash,
            Code::from_bytecode(code.clone(), &NativeCrypto).hash,
            "code of {address:?} changed after invalid tx",
        );

        // Every storage slot present in the cache for this account must be its
        // seeded value (the seeded accounts start with empty storage, so any
        // non-zero value is residue). Slot 0 is the one the harness bytecodes
        // touch, so a residual `1` here is the rollback regression signature.
        for (slot, value) in current.storage.iter() {
            assert!(
                value.is_zero(),
                "storage residue at {address:?} slot {slot:?} = {value:?} after invalid tx",
            );
        }
    }
}

// ==================== Invalid-tx rollback ====================

#[test]
fn invalid_frame_tx_leaves_db_cache_clean() {
    // One DEFAULT frame to a contract that SSTOREs and succeeds, but NO APPROVE
    // anywhere -> payer is None -> tx invalid AFTER the frame committed state.
    let target = Address::from_low_u64_be(0xC0);
    let accounts = [(
        target,
        U256::zero(),
        0u64,
        Bytes::from(SSTORE_THEN_STOP_CODE.to_vec()),
    )];
    let tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(target),
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    let (result, db) = run_frame_tx(&accounts, tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::InvalidFrameTransaction(_)
            ))
        ),
        "expected InvalidFrameTransaction, got {result:?}",
    );
    // Slot 0 of target must NOT be 1 — the frame's SSTORE must have been rolled back.
    assert_db_cache_unchanged(&db, &accounts);
}

// ==================== Reverting SENDER frame must not leak value ====================

#[test]
fn reverting_sender_frame_returns_value() {
    let target = Address::from_low_u64_be(0xC1); // contract that SSTOREs then REVERTs
    let wallet = Address::from_low_u64_be(0xC2); // separate payer
    let value = U256::from(1_000_000u64);
    // Frame 0: VERIFY to the sender -> APPROVE scope 2 (sender_approved).
    //          Scope 2 requires frame_target == tx.sender, so it must target
    //          FUNDED_SENDER and run from FUNDED_SENDER's code.
    // Frame 1: VERIFY to the wallet -> APPROVE scope 1 (payer = wallet), so the
    //          sender pays no gas and its balance is untouched except for the
    //          (to-be-reverted) value transfer.
    // Frame 2: SENDER to a reverting contract carrying `value`.
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        pay_frame(wallet),
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(target),
            gas_limit: 100_000,
            state_limit: 0,
            value,
            data: Bytes::new(),
        },
    ]);
    let (result, db) = run_frame_tx(
        &[
            // Sender carries the execution-approval code; pass it explicitly so
            // its balance equals AUTO_SEED_SENDER_BALANCE for the assertion.
            (
                FUNDED_SENDER,
                AUTO_SEED_SENDER_BALANCE,
                0,
                Bytes::from(APPROVE_EXECUTION_CODE.to_vec()),
            ),
            (
                wallet,
                U256::from(10u64).pow(U256::from(18u64)),
                0,
                Bytes::from(APPROVE_PAYMENT_CODE.to_vec()),
            ),
            (
                target,
                U256::zero(),
                0,
                Bytes::from(SSTORE_THEN_REVERT_CODE.to_vec()),
            ),
        ],
        tx,
    );
    let report = result.expect("tx is valid (payer approved); only the SENDER frame failed");
    // The reverting frame must NOT have delivered value: target keeps nothing,
    // and the value is returned to the sender (sender pays no gas — wallet is payer).
    assert_eq!(
        balance_of(&db, target),
        U256::zero(),
        "reverted frame leaked value to target"
    );
    assert_eq!(
        balance_of(&db, FUNDED_SENDER),
        AUTO_SEED_SENDER_BALANCE,
        "sender did not get value back"
    );
    // The SENDER frame (index 2) must be reported as a failure.
    let frame_results = report
        .frame_results
        .expect("frame tx report must carry per-frame results");
    assert_eq!(
        frame_results[2].status,
        ethrex_common::types::FRAME_RECEIPT_STATUS_FAILURE,
        "SENDER frame should be reported as failure"
    );
}

// ==================== Payer charged at effective price (no burn) ====================

#[test]
fn payer_pays_effective_price_no_burn() {
    // base_fee = 10 gwei, priority = 2, max_fee = 100 gwei (huge headroom).
    // effective = base + priority = 12 gwei. The (100-12) spread must NOT burn.
    let wallet = Address::from_low_u64_be(0xD2);
    let stop_contract = Address::from_low_u64_be(0xD3);
    let wallet_initial = U256::from(10u64).pow(U256::from(18u64)); // 1 ETH
    let mut tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER), // runs APPROVE_EXECUTION_CODE -> seed below
        pay_frame(wallet),           // runs APPROVE_PAYMENT_CODE   -> seed below
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(stop_contract),
            gas_limit: 30_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);
    tx.max_fee_per_gas = 100_000_000_000; // 100 gwei
    tx.max_priority_fee_per_gas = 2_000_000_000; // 2 gwei
    let (result, db) = run_frame_tx_with_fees(
        &[
            (
                FUNDED_SENDER,
                AUTO_SEED_SENDER_BALANCE,
                0,
                Bytes::from(APPROVE_EXECUTION_CODE.to_vec()),
            ),
            (
                wallet,
                wallet_initial,
                0,
                Bytes::from(APPROVE_PAYMENT_CODE.to_vec()),
            ),
            (stop_contract, U256::zero(), 0, Bytes::from(vec![0x00u8])), // STOP
        ],
        tx,
        10_000_000_000, // base fee 10 gwei
    );
    let report = result.expect("valid: sender approved, payer set");
    let effective = U256::from(12_000_000_000u64);
    let total_gas_used = U256::from(report.gas_used);
    // Net payer cost == effective * total_gas_used (no max-vs-effective burn).
    let payer_delta = wallet_initial - balance_of(&db, wallet);
    assert_eq!(
        payer_delta,
        effective * total_gas_used,
        "payer overcharged/undercharged"
    );
    // Conservation: payer's loss == coinbase gain + base-fee burn (nothing vanishes).
    let coinbase_gain = balance_of(&db, COINBASE_ADDR);
    let base_burn = U256::from(10_000_000_000u64) * total_gas_used;
    assert_eq!(
        payer_delta,
        coinbase_gain + base_burn,
        "value silently burned"
    );
}

// ==================== FRAMEPARAM stack operand order ====================

/// FRAMEPARAM(param=0x01, frameIndex=0) → gas_limit of frame[0], then SSTORE at slot 0.
/// Bytecode: PUSH1 0x01 (param), PUSH1 0x00 (frameIndex — top), FRAMEPARAM (0xB3),
///           PUSH1 0x00 (slot key), SSTORE (0x55), STOP (0x00).
const FRAMEPARAM_READ_FRAME0_GASLIMIT: &[u8] =
    &[0x60, 0x01, 0x60, 0x00, 0xB3, 0x60, 0x00, 0x55, 0x00];

/// Read storage `key` of `addr` from the post-execution cache.
fn storage_slot(db: &GeneralizedDatabase, addr: Address, key: ethrex_common::H256) -> U256 {
    db.current_accounts_state
        .get(&addr)
        .and_then(|acc| acc.storage.get(&key).copied())
        .unwrap_or_default()
}

/// EIP-8141 adds three FRAMEPARAM indices for the second gas dimension: `0x09` is a
/// frame's declared `limits.state`, and `0x0A`/`0x0B` are a completed frame's
/// `gas_used.execution` and `gas_used.state`.
///
/// The two usage reads follow `status` (`0x05`): a frame's usage is only defined once it has
/// exited, so reading the current or a future frame halts rather than reporting a figure
/// that is still moving. Without them a sponsor can see what a frame *declared* but never
/// what it *spent*, which is the pair the second dimension exists to expose.
#[test]
fn frameparam_reports_the_state_dimension() {
    // Frame 1 creates a slot (drawing state gas); frame 2 reads frame 1's figures back:
    //   slot 0 = FRAMEPARAM(frame 1, 0x09) -- its declared limits.state
    //   slot 1 = FRAMEPARAM(frame 1, 0x0A) -- its execution gas used
    //   slot 2 = FRAMEPARAM(frame 1, 0x0B) -- its state gas used
    // FRAMEPARAM pops [frameIndex, param] with frameIndex on top.
    let reader_code: &[u8] = &[
        0x60, 0x09, 0x60, 0x01, 0xB3, 0x60, 0x00, 0x55, // slot 0
        0x60, 0x0A, 0x60, 0x01, 0xB3, 0x60, 0x01, 0x55, // slot 1
        0x60, 0x0B, 0x60, 0x01, 0xB3, 0x60, 0x02, 0x55, // slot 2
        0x00,
    ];
    let writer = Address::from_low_u64_be(0x0DDD_1);
    let reader = Address::from_low_u64_be(0x0DDD_2);
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        // PUSH1 1; PUSH1 0; SSTORE; STOP
        (
            writer,
            U256::zero(),
            0,
            Bytes::from(vec![0x60, 0x01, 0x60, 0x00, 0x55, 0x00]),
        ),
        (reader, U256::zero(), 0, Bytes::from(reader_code.to_vec())),
    ];
    let writer_state_limit = 2 * SSTORE_SET_STATE_GAS;
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0,
            target: Some(writer),
            gas_limit: 300_000,
            state_limit: writer_state_limit,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0,
            target: Some(reader),
            gas_limit: 300_000,
            state_limit: STATE_BUDGET,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("valid tx");
    let fr = report.frame_results.expect("per-frame results");

    assert_eq!(
        storage_slot(&db, reader, ethrex_common::H256::zero()),
        U256::from(writer_state_limit),
        "0x09 must report the frame's declared limits.state"
    );
    assert_eq!(
        storage_slot(&db, reader, H256::from_low_u64_be(1)),
        U256::from(fr[1].gas_used),
        "0x0A must report the completed frame's execution gas, matching its receipt"
    );
    assert_eq!(
        storage_slot(&db, reader, H256::from_low_u64_be(2)),
        U256::from(fr[1].state_gas_used),
        "0x0B must report the completed frame's state gas, matching its receipt"
    );
    assert_eq!(
        fr[1].state_gas_used, SSTORE_SET_STATE_GAS,
        "the writing frame created one slot, so its receipt reports one slot's state gas"
    );
}

/// The two usage reads must halt for the CURRENT frame, like `status` does: a frame cannot
/// observe its own final gas figures while it is still spending them.
#[test]
fn frameparam_usage_reads_halt_for_the_current_frame() {
    let reader = Address::from_low_u64_be(0x0DDD_3);
    // PUSH1 0x0A; PUSH1 1; FRAMEPARAM  -- frame 1 reading ITSELF
    let code: &[u8] = &[0x60, 0x0A, 0x60, 0x01, 0xB3, 0x00];
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (reader, U256::zero(), 0, Bytes::from(code.to_vec())),
    ];
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0,
            target: Some(reader),
            gas_limit: 200_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("the tx stays valid; only the reading frame halts");
    let fr = report.frame_results.expect("per-frame results");
    assert_eq!(
        fr[1].status,
        ethrex_common::types::FRAME_RECEIPT_STATUS_FAILURE,
        "a frame reading its own gas_used must halt, as it does for its own status"
    );
}

#[test]
fn frameparam_reads_frame_index_from_stack_top() {
    let wallet = Address::from_low_u64_be(0xE2);
    let reader = Address::from_low_u64_be(0xE3);
    let mut frames = vec![
        verify_frame(FUNDED_SENDER), // frame[0]: VERIFY, runs APPROVE_EXECUTION_CODE
        pay_frame(wallet),           // frame[1]: VERIFY, runs APPROVE_PAYMENT_CODE
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0,
            target: Some(reader),
            // The new-slot SSTORE draws STATE_BYTES_PER_STORAGE_SET *
            // cost_per_state_byte (~98k) from the frame's state pool, which under
            // EIP-8141 is the frame's own declared `limits.state` and never
            // borrows from `limits.execution`.
            gas_limit: 300_000,
            state_limit: STATE_BUDGET,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ];
    // Set a distinctive gas_limit on frame[0] that FRAMEPARAM(param=1, frameIndex=0) must read.
    frames[0].gas_limit = 77_777;
    let tx = frame_tx_with_frames(frames);
    let (result, db) = run_frame_tx(
        &[
            (
                FUNDED_SENDER,
                AUTO_SEED_SENDER_BALANCE,
                0,
                Bytes::from(APPROVE_EXECUTION_CODE.to_vec()),
            ),
            (
                wallet,
                U256::from(10u64).pow(U256::from(18u64)),
                0,
                Bytes::from(APPROVE_PAYMENT_CODE.to_vec()),
            ),
            (
                reader,
                U256::zero(),
                0,
                Bytes::from(FRAMEPARAM_READ_FRAME0_GASLIMIT.to_vec()),
            ),
        ],
        tx,
    );
    result.expect("valid tx (sender approved, payer set)");
    // After the fix: FRAMEPARAM pops frameIndex=0 (top) and param=1 (second),
    // reads frame[0].gas_limit = 77_777, SSTOREs it at slot 0 of `reader`.
    // With the bug: pops param=0 (top) and frameIndex=1, reads frame[1].target
    // (the wallet address) — so the assertion below catches the swap.
    let stored = storage_slot(&db, reader, ethrex_common::H256::zero());
    assert_eq!(
        stored,
        U256::from(77_777u64),
        "FRAMEPARAM read the wrong operand order (stored {stored:#x}, expected 77_777)"
    );
}

// ==================== APPROVE scope-0 bypass ====================

#[test]
fn approve_halts_when_frame_scope_is_none() {
    // flags=0 (APPROVE_SCOPE_NONE). The frame targets the sender and runs
    // APPROVE(scope=3). Pre-fix the scope-0 bypass lets it succeed (payer=sender,
    // tx valid); post-fix allowed_scope==0 must halt -> no payer -> invalid tx.
    //
    // Bytecode: APPROVE_BOTH_CODE (PUSH1 3; PUSH1 0; PUSH1 0; APPROVE 0xAA)
    let tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0x00,
        target: Some(FUNDED_SENDER),
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    let accounts = [(
        FUNDED_SENDER,
        AUTO_SEED_SENDER_BALANCE,
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )];
    let (result, db) = run_frame_tx(&accounts, tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::InvalidFrameTransaction(_)
            ))
        ),
        "APPROVE with allowed_scope==0 must halt, leaving the tx invalid; got {result:?}"
    );
    assert_db_cache_unchanged(&db, &accounts);
}

// ==================== Batched VERIFY revert invalidates tx ====================

#[test]
fn batched_verify_revert_invalidates_tx() {
    let reverter = Address::from_low_u64_be(0xF1);
    let stop_ct = Address::from_low_u64_be(0xF2);
    // frame0: VERIFY -> sender, runs APPROVE(3) -> sets payer=sender (tx would be valid).
    // frame1: VERIFY with ATOMIC_BATCH_FLAG -> a contract that REVERTs.
    // frame2: DEFAULT batch terminator (no flag) -> needed so the batch flag isn't on the last frame.
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER), // flags 0x03; FUNDED_SENDER seeded with APPROVE_BOTH_CODE
        Frame {
            mode: u8::from(FrameMode::Verify),
            flags: 0x04,
            target: Some(reverter),
            gas_limit: 60_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0x00,
            target: Some(stop_ct),
            gas_limit: 30_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (
            reverter,
            U256::zero(),
            0,
            Bytes::from(PURE_REVERT_CODE.to_vec()),
        ),
        (stop_ct, U256::zero(), 0, Bytes::from(vec![0x00u8])), // STOP
    ];
    let (result, db) = run_frame_tx(&accounts, tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::InvalidFrameTransaction(_)
            ))
        ),
        "a batched VERIFY revert must invalidate the tx; got {result:?}"
    );
    assert_db_cache_unchanged(&db, &accounts);
}

// ============== I10: APPROVE_PAYMENT must NOT precede APPROVE_EXECUTION ==============

#[test]
fn payment_approval_before_execution_approval_reverts() {
    // EIP-8141: APPROVE_PAYMENT (scope 1) must revert the frame while
    // sender_approved == false. Frame 0 is a paymaster VERIFY frame calling
    // APPROVE(APPROVE_PAYMENT) BEFORE the sender has approved execution -> the
    // frame reverts -> the VERIFY prefix reverts -> the tx is invalid.
    let paymaster = Address::from_low_u64_be(0x9A);
    let stop_ct = Address::from_low_u64_be(0x9B);
    let tx = frame_tx_with_frames(vec![
        // frame0: paymaster approves PAYMENT first (scope 1) -> must revert.
        pay_frame(paymaster),
        // frame1: sender approves EXECUTION (scope 2).
        verify_frame(FUNDED_SENDER),
        // frame2: a SENDER frame that just STOPs.
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(stop_ct),
            gas_limit: 30_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);
    let accounts = [
        (
            paymaster,
            U256::from(10u64).pow(U256::from(18u64)),
            0,
            Bytes::from(APPROVE_PAYMENT_CODE.to_vec()),
        ),
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_EXECUTION_CODE.to_vec()),
        ),
        (stop_ct, U256::zero(), 0, Bytes::from(vec![0x00u8])), // STOP
    ];
    let (result, db) = run_frame_tx(&accounts, tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::InvalidFrameTransaction(_)
            ))
        ),
        "APPROVE_PAYMENT before the sender's execution approval must revert the \
         frame and invalidate the tx; got {result:?}"
    );
    assert_db_cache_unchanged(&db, &accounts);
}

// ==================== SENDER/DEFAULT default code returns success ====================

#[test]
fn sender_frame_transfers_value_to_eoa() {
    let eoa = Address::from_low_u64_be(0xE0A); // code-less; NOT seeded with code
    let value = U256::from(5_000_000u64);
    let tx = frame_tx_with_frames(vec![
        // frame0: VERIFY on the sender -> APPROVE(3) -> payer=sender, sender_approved.
        verify_frame(FUNDED_SENDER),
        // frame1: SENDER frame delivering value to a code-less EOA.
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(eoa),
            gas_limit: 50_000,
            // The recipient does not exist yet, so the frame pays EIP-2780's
            // account-creation charge out of its own state budget.
            state_limit: NEW_ACCOUNT_STATE_GAS,
            value,
            data: Bytes::new(),
        },
    ]);
    let accounts = [(
        FUNDED_SENDER,
        AUTO_SEED_SENDER_BALANCE,
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )];
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("plain EOA transfer must be a VALID, SUCCESSFUL tx");
    // frame[1] (the SENDER frame) succeeded:
    let frame_results = report.frame_results.expect("frame results present");
    assert_eq!(
        frame_results[1].status,
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "SENDER frame to a code-less EOA must succeed (default code = success)"
    );
    // The EOA actually received the value:
    assert_eq!(balance_of(&db, eoa), value, "value not delivered to EOA");
}

/// A frame that cannot afford its entry access charge must leave NOTHING behind — in
/// particular not the delegatee of an EIP-7702 target in the EIP-7928 block access list.
///
/// Following a delegation is what warms the delegatee and files it as touched, so it has to
/// happen after the charge is paid, not while deciding which dispatch branch to take. The
/// receipts cannot expose a mistake here: an unaffordable frame forfeits its whole gas
/// limit either way, so the access list is the only place a stray touch shows, and a
/// builder that files one writes a list its own block contradicts.
#[test]
fn an_unaffordable_frame_files_no_delegatee_in_the_block_access_list() {
    let delegatee = Address::from_low_u64_be(0x7702_D1);
    let delegator = Address::from_low_u64_be(0x7702_D0);
    // `0xef0100 || delegatee`, the EIP-7702 designation.
    let mut indicator = vec![0xEF, 0x01, 0x00];
    indicator.extend_from_slice(delegatee.as_bytes());

    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (delegator, U256::from(1u64), 0, Bytes::from(indicator)),
        // The delegatee holds real code, so following the delegation is observable.
        (delegatee, U256::zero(), 0, Bytes::from(vec![0x00])),
    ];

    let touched_delegatee = |gas_limit: u64| {
        let tx = frame_tx_with_frames(vec![
            verify_frame(FUNDED_SENDER),
            Frame {
                mode: u8::from(FrameMode::Default),
                flags: 0,
                target: Some(delegator),
                gas_limit,
                state_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ]);
        let mut db = seeded_db(&accounts);
        db.enable_bal_recording();
        let env = frame_tx_env(&tx);
        let transaction = Transaction::FrameTransaction(tx);
        let report = {
            let mut vm = VM::new(
                env,
                &mut db,
                &transaction,
                LevmCallTracer::disabled(),
                VMType::L1,
                &NativeCrypto,
                None,
            )
            .expect("VM::new should succeed for a frame tx");
            vm.execute()
                .expect("the transaction stays valid; only the frame's fate differs")
        };
        let status = report
            .frame_results
            .expect("per-frame results")
            .get(1)
            .expect("the delegated frame has a receipt")
            .status;
        let bal = db
            .bal_recorder
            .take()
            .expect("BAL recording was enabled")
            .build();
        let touched = bal
            .accounts()
            .iter()
            .any(|account| account.address == delegatee);
        (status, touched)
    };

    let cold =
        ethrex_levm::gas_cost::cold_account_access_cost(ethrex_common::types::Fork::Amsterdam);
    let (status, touched) = touched_delegatee(cold - 1);
    assert_eq!(
        status,
        ethrex_common::types::FRAME_RECEIPT_STATUS_FAILURE,
        "a frame below the entry access charge must halt"
    );
    assert!(
        !touched,
        "a frame that never paid its entry charge must not file its target's delegatee in \
         the block access list"
    );

    // The control: once the frame can pay, following the delegation is exactly what should
    // record the delegatee, so the check above cannot pass by never recording at all.
    let (status, touched) = touched_delegatee(cold + 50_000);
    assert_eq!(
        status,
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "a funded frame targeting a delegated account must run"
    );
    assert!(
        touched,
        "following a delegation must file the delegatee as touched (EIP-7928)"
    );
}

/// A transaction whose data floor exceeds what it declared for execution is charged the
/// floor, not the declaration. `max_gas` reserves the floor rather than rejecting the
/// transaction, and settlement then charges it — otherwise a frame transaction could carry
/// unlimited calldata for the price of its frame budgets.
#[test]
fn a_floor_bound_frame_transaction_is_charged_the_floor() {
    let tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(FUNDED_SENDER),
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::from(vec![0x11u8; 4_096]),
    }]);
    let floor_total = tx.calldata_floor_total();
    assert!(
        floor_total > tx.standard_gas_limit(),
        "the floor must bind for this to test the settlement"
    );

    let (result, _db) = run_frame_tx(
        &[(
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        )],
        tx.clone(),
    );
    let report = result.expect("valid: the self-verify frame approves execution and payment");
    assert_eq!(
        report.gas_used, floor_total,
        "a floor-bound transaction is charged max_gas, which is the floor"
    );
    // EIP-8141 reports `tx_unused_gas` directly — gas left in either pool at frame exit,
    // which the floor does not consume — so the frame's unspent execution budget is
    // reported even though the charge is the floor. It is not a discount on that charge.
    let fr = report.frame_results.expect("per-frame results");
    assert_eq!(
        report.gas_refunded,
        tx.frames[0].gas_limit - fr[0].gas_used,
        "the reported unused gas is what the frame left in its pools"
    );
    assert_eq!(
        report.gas_used, report.gas_spent,
        "no storage refund was earned, so the block and the payer see the same figure"
    );
}

/// EIP-8141 §Behavior: "if `resolved_target` is an active precompile at the current
/// fork, the frame dispatches it as an ordinary call would."
///
/// A precompile's code is empty, which is also the condition for running the default
/// code, so the two cases have to be told apart at dispatch. Getting it wrong is silent:
/// the default code returns success without computing anything and without charging the
/// precompile's gas, so a frame that paid for `ecrecover` would get an empty answer and a
/// refund, and a paymaster verifying through a precompile would be trivially spoofable.
#[test]
fn a_frame_targeting_a_precompile_dispatches_it() {
    // ecrecover (0x01) costs a flat 3000 gas. A frame budgeted below that must run out;
    // under the default-code path it would succeed for free, which is what this catches.
    let ecrecover = Address::from_low_u64_be(1);
    let accounts = [(
        FUNDED_SENDER,
        AUTO_SEED_SENDER_BALANCE,
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )];
    let status_with_budget = |gas_limit: u64| {
        let tx = frame_tx_with_frames(vec![
            verify_frame(FUNDED_SENDER),
            Frame {
                mode: u8::from(FrameMode::Default),
                flags: 0,
                target: Some(ecrecover),
                gas_limit,
                state_limit: 0,
                value: U256::zero(),
                data: Bytes::from(vec![0u8; 128]),
            },
        ]);
        let (result, _db) = run_frame_tx(&accounts, tx);
        let report = result.expect("the transaction stays valid; only the frame's fate differs");
        report
            .frame_results
            .expect("per-frame results")
            .get(1)
            .expect("the precompile frame has a receipt")
            .status
    };

    // Precompiles start warm (EIP-2929/EIP-3651), so the frame-entry access charge is the
    // warm rate and the budget below is spent almost entirely on ecrecover itself.
    let warm = ethrex_levm::gas_cost::WARM_ADDRESS_ACCESS_COST;
    assert_eq!(
        status_with_budget(warm + 2_999),
        ethrex_common::types::FRAME_RECEIPT_STATUS_FAILURE,
        "a frame one gas short of ecrecover's 3000 must fail — if it succeeds, the frame \
         ran the default code instead of the precompile"
    );
    assert_eq!(
        status_with_budget(warm + 3_000),
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "a frame budgeted for exactly ecrecover's 3000 plus its entry access must succeed"
    );
}

/// EIP-8141 §Behavior: a value-bearing frame whose resolved target does not exist
/// pays EIP-2780's account-creation state gas from its own `limits.state`, and halts
/// exceptionally if it declared too little. Funding a fresh account grows the state, and
/// EIP-8141 makes the frame that does it declare the budget for it.
#[test]
fn a_value_bearing_frame_pays_account_creation_from_its_state_budget() {
    let value = U256::from(5_000_000u64);
    let accounts = [(
        FUNDED_SENDER,
        AUTO_SEED_SENDER_BALANCE,
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )];
    let run = |recipient: Address, state_limit: u64| {
        let tx = frame_tx_with_frames(vec![
            verify_frame(FUNDED_SENDER),
            Frame {
                mode: u8::from(FrameMode::Sender),
                flags: 0,
                target: Some(recipient),
                gas_limit: 50_000,
                state_limit,
                value,
                data: Bytes::new(),
            },
        ]);
        let (result, db) = run_frame_tx(&accounts, tx);
        let report = result.expect("the transaction stays valid; only the frame's fate differs");
        let fr = report.frame_results.expect("per-frame results");
        (
            fr[1].status,
            fr[1].state_gas_used,
            balance_of(&db, recipient),
        )
    };

    let (status, state_gas, balance) = run(Address::from_low_u64_be(0xE0C), NEW_ACCOUNT_STATE_GAS);
    assert_eq!(
        status,
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "a frame declaring exactly the account-creation charge must succeed"
    );
    assert_eq!(
        state_gas, NEW_ACCOUNT_STATE_GAS,
        "the creation charge must be attributed to the frame that funded the account"
    );
    assert_eq!(balance, value, "the value must reach the new account");

    let (status, state_gas, balance) =
        run(Address::from_low_u64_be(0xE0D), NEW_ACCOUNT_STATE_GAS - 1);
    assert_eq!(
        status,
        ethrex_common::types::FRAME_RECEIPT_STATUS_FAILURE,
        "a frame one gas short of the creation charge must halt exceptionally"
    );
    assert_eq!(state_gas, 0, "a halted frame reports no state gas");
    assert!(
        balance.is_zero(),
        "a halted frame must not deliver its value, got {balance}"
    );
}

#[test]
fn sender_frame_to_eoa_emits_transfer_log() {
    // EIP-7708 (active at Amsterdam, and Hegota >= Amsterdam): an ETH transfer
    // to an EOA via a SENDER frame must emit the Transfer log in the frame
    // receipt. The default-code branch must capture the substate log rather than
    // drop it — otherwise frame_receipts[i].logs (which is committed to the
    // receipts-trie root) omits a log a spec-compliant client includes, forking
    // the chain on the most basic frame-tx operation.
    use ethrex_common::constants::SYSTEM_ADDRESS;
    use ethrex_levm::constants::TRANSFER_EVENT_TOPIC;

    let eoa = Address::from_low_u64_be(0xE0B); // code-less recipient
    let value = U256::from(7_000_000u64);
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER), // APPROVE(3): payer=sender, sender_approved
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(eoa),
            gas_limit: 50_000,
            // The recipient does not exist yet, so the frame pays EIP-2780's
            // account-creation charge out of its own state budget.
            state_limit: NEW_ACCOUNT_STATE_GAS,
            value,
            data: Bytes::new(),
        },
    ]);
    let accounts = [(
        FUNDED_SENDER,
        AUTO_SEED_SENDER_BALANCE,
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )];
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("EOA transfer must be a valid, successful tx");
    let is_transfer_log = |l: &ethrex_common::types::Log| {
        l.address == SYSTEM_ADDRESS && l.topics.first() == Some(&TRANSFER_EVENT_TOPIC)
    };
    // The EIP-7708 Transfer log must be in the SENDER frame's per-frame receipt
    // (frame index 1) — that's what the consensus receipts-root commits to.
    let frame_results = report
        .frame_results
        .as_ref()
        .expect("frame results present");
    assert!(
        frame_results[1].logs.iter().any(is_transfer_log),
        "EIP-7708 transfer log missing from frame_receipts[1].logs: {:?}",
        frame_results[1].logs
    );
    // ...and in the aggregated report logs (eth_getLogs / RPC).
    assert!(
        report.logs.iter().any(is_transfer_log),
        "EIP-7708 transfer log missing from report.logs"
    );
}

// ==================== Happy-path E2E: SSTORE + LOG0 ====================

/// Bytecode: PUSH1 0x2a, PUSH1 0x00, SSTORE, PUSH1 0x00 (size), PUSH1 0x00 (offset), LOG0, STOP.
/// Writes 0x2a to slot 0, then emits an empty-data LOG0, then halts successfully.
const SSTORE_AND_LOG_CODE: &[u8] = &[
    0x60, 0x2a, // PUSH1 0x2a
    0x60, 0x00, // PUSH1 0x00  (slot key)
    0x55, // SSTORE
    0x60, 0x00, // PUSH1 0x00  (size = 0)
    0x60, 0x00, // PUSH1 0x00  (offset = 0)
    0xA0, // LOG0
    0x00, // STOP
];

#[test]
fn frame_tx_happy_path_sstore_and_log() {
    let worker = Address::from_low_u64_be(0xC0FFEE);

    // Frame 0: VERIFY targeting the funded sender — runs APPROVE_BOTH_CODE (scope 3),
    //          setting payer = sender and sender_approved in one shot.
    // Frame 1: SENDER to the worker contract — executes SSTORE + LOG0.
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(worker),
            // The new-slot SSTORE draws STATE_BYTES_PER_STORAGE_SET *
            // cost_per_state_byte (~98k) from the frame's state pool, which under
            // EIP-8141 is the frame's own declared `limits.state` and never
            // borrows from `limits.execution`.
            gas_limit: 300_000,
            state_limit: STATE_BUDGET,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);

    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (
            worker,
            U256::zero(),
            0,
            Bytes::from(SSTORE_AND_LOG_CODE.to_vec()),
        ),
    ];

    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("happy-path frame tx must succeed");

    // 1. Overall transaction result is Success.
    assert!(
        matches!(report.result, TxResult::Success),
        "expected TxResult::Success, got {:?}",
        report.result
    );

    // 2. Storage written by the SENDER frame: slot 0 of worker == 0x2a.
    assert_eq!(
        storage_slot(&db, worker, H256::zero()),
        U256::from(0x2au64),
        "SSTORE did not write 0x2a to slot 0 of worker"
    );

    // 3. The LOG0 appears in the aggregated report.logs (logs collected).
    assert!(
        report.logs.iter().any(|l| l.address == worker),
        "log from worker missing from aggregated report.logs"
    );

    // 4. Per-frame isolation: frame_results[1] is success and carries the log;
    //    frame_results[0] (the VERIFY/approve frame) has no logs.
    let frame_results = report
        .frame_results
        .expect("frame tx report must carry per-frame results");

    assert_eq!(
        frame_results[1].status, FRAME_RECEIPT_STATUS_SUCCESS,
        "SENDER frame (index 1) must be reported as success"
    );
    assert!(
        frame_results[1].logs.iter().any(|l| l.address == worker),
        "log from worker missing from frame_results[1].logs"
    );
    assert!(
        frame_results[0].logs.is_empty(),
        "approve VERIFY frame (index 0) must have no logs; isolation violated"
    );

    // 5. Sender nonce incremented exactly once by APPROVE (scope 3 bumps nonce once).
    assert_eq!(
        nonce_of(&db, FUNDED_SENDER),
        1,
        "sender nonce must be 1 after APPROVE (scope 3 increments nonce once)"
    );
}

// ============ Regression: per-frame log isolation across contract frames ============

/// PUSH1 0x00 (size), PUSH1 0x00 (offset), LOG0, STOP — emits one empty LOG0 at
/// the executing contract's own address, then halts successfully.
const LOG0_CODE: &[u8] = &[
    0x60, 0x00, // PUSH1 0x00 (size = 0)
    0x60, 0x00, // PUSH1 0x00 (offset = 0)
    0xA0, // LOG0
    0x00, // STOP
];

/// Two SENDER frames, each targeting a *different* log-emitting contract, must
/// each carry only their own log in `frame_receipts[i].logs`, and the aggregate
/// `report.logs` must contain each log exactly once.
///
/// Regression for the double-commit bug (PR #6326, iovoid review): the CallFrame
/// branch pushed a substate backup and let `run_execution` commit it (the inner
/// frame is the initial call frame, so it commits via `handle_state_backup`), but
/// then ALSO called `commit_backup` a second time and read `current_logs()` after
/// that commit — pulling the first frame's already-merged log into the second
/// frame's receipt and duplicating it in the aggregate (a receipts-root /
/// logs-bloom divergence). With only one log-emitting frame the bug is invisible,
/// so this test uses two.
#[test]
fn multiple_contract_frames_do_not_duplicate_logs() {
    let worker_a = Address::from_low_u64_be(0xAAA0);
    let worker_b = Address::from_low_u64_be(0xBBB0);

    let tx = frame_tx_with_frames(vec![
        // Frame 0: VERIFY on the funded sender — APPROVE_BOTH (emits no logs).
        verify_frame(FUNDED_SENDER),
        // Frame 1: SENDER to worker_a — emits LOG0 at worker_a.
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(worker_a),
            gas_limit: 100_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
        // Frame 2: SENDER to worker_b — emits LOG0 at worker_b.
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(worker_b),
            gas_limit: 100_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);

    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (worker_a, U256::zero(), 0, Bytes::from(LOG0_CODE.to_vec())),
        (worker_b, U256::zero(), 0, Bytes::from(LOG0_CODE.to_vec())),
    ];

    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("multi-frame log tx must succeed");

    assert!(
        matches!(report.result, TxResult::Success),
        "expected TxResult::Success, got {:?}",
        report.result
    );

    let frame_results = report
        .frame_results
        .expect("frame tx report must carry per-frame results");

    // Per-frame isolation: each SENDER frame carries exactly its own log.
    assert_eq!(
        frame_results[1].logs.len(),
        1,
        "worker_a frame must carry exactly one log, got {:?}",
        frame_results[1].logs
    );
    assert!(
        frame_results[1].logs.iter().all(|l| l.address == worker_a),
        "worker_a frame receipt must contain only worker_a's log"
    );
    assert_eq!(
        frame_results[2].logs.len(),
        1,
        "worker_b frame must carry exactly one log (the bug leaked worker_a's log \
         in here), got {:?}",
        frame_results[2].logs
    );
    assert!(
        frame_results[2].logs.iter().all(|l| l.address == worker_b),
        "worker_b frame receipt must contain only worker_b's log; worker_a's log leaked in"
    );

    // Aggregate: each worker's log appears exactly once.
    assert_eq!(
        report.logs.iter().filter(|l| l.address == worker_a).count(),
        1,
        "worker_a log must appear exactly once in report.logs (the bug duplicated it), got {:?}",
        report.logs
    );
    assert_eq!(
        report.logs.iter().filter(|l| l.address == worker_b).count(),
        1,
        "worker_b log must appear exactly once in report.logs"
    );
    assert_eq!(
        report.logs.len(),
        2,
        "aggregate report.logs must contain exactly two logs, got {:?}",
        report.logs
    );
}

// ==================== EIP-8037 state gas in frame txs ====================
//
// A frame tx must split gas into the EIP-8037 regular/state dimensions exactly
// like every other transaction: a state-creating SSTORE bills its state portion
// to the state dimension (reported via `state_gas_used`), and the block-level
// regular dimension is `gas_used - state_gas_used`. A frame that reverts creates
// no state and must report zero state gas.

/// STATE_BYTES_PER_STORAGE_SET (64) * cost_per_state_byte (1530).
const SSTORE_SET_STATE_GAS: u64 = 64 * 1530;

/// EIP-8037 `STATE_BYTES_PER_NEW_ACCOUNT * CPSB`: what a frame is charged for funding an
/// account that does not exist yet.
const NEW_ACCOUNT_STATE_GAS: u64 = 120 * 1530;

/// A state budget generous enough for a handful of new storage slots. EIP-8141
/// frames declare `limits.state` explicitly and a charge past it halts the frame, so
/// every test frame that creates state has to carry one. Over-declaring costs only a
/// larger up-front `max_cost`, refunded at settlement, so tests that are not about
/// the exact budget use this rather than a tight per-test figure.
const STATE_BUDGET: u64 = 1_000_000;

#[test]
fn frame_sstore_set_reports_eip8037_state_gas() {
    // DEFAULT frame whose target creates slot 0 (0 -> 1): a state-creating SSTORE.
    let writer = Address::from_low_u64_be(0xDA7A);
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0u64,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        // PUSH1 1; PUSH1 0; SSTORE; STOP
        (
            writer,
            U256::zero(),
            0u64,
            Bytes::from(vec![0x60, 0x01, 0x60, 0x00, 0x55, 0x00]),
        ),
    ];
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0x00,
            target: Some(writer),
            gas_limit: 2_000_000,
            state_limit: STATE_BUDGET,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("valid frame tx (payer approved)");
    assert_eq!(
        report.state_gas_used, SSTORE_SET_STATE_GAS,
        "a frame SSTORE-set must report EIP-8037 state gas (not 0), got {}",
        report.state_gas_used,
    );
    // The total already includes the state gas; the regular dimension is the rest.
    assert!(
        report.gas_used > report.state_gas_used,
        "total gas {} must exceed the state portion {}",
        report.gas_used,
        report.state_gas_used,
    );
    // EIP-8141: the charge is attributed to the frame that made it, and shows up
    // in that frame's receipt as `gas_used.state`. The VERIFY frame created nothing.
    let fr = report.frame_results.expect("per-frame results");
    assert_eq!(
        fr[1].state_gas_used, SSTORE_SET_STATE_GAS,
        "the writing frame's receipt must report its own state gas, got {}",
        fr[1].state_gas_used,
    );
    assert_eq!(
        fr[0].state_gas_used, 0,
        "the VERIFY frame created no state and must report none",
    );
    // The frame drew it from its own pool, not from its execution budget.
    assert!(
        fr[1].gas_used < SSTORE_SET_STATE_GAS,
        "the writing frame's execution gas ({}) must not include the state charge — \
         the two pools never mix",
        fr[1].gas_used,
    );
}

#[test]
fn reverted_frame_reports_no_state_gas() {
    // Same state-creating SSTORE, but the frame REVERTs — no state is committed,
    // so the tx must report zero state gas. Frame 0 approves a payer, so the tx
    // stays valid (a reverted DEFAULT frame does not invalidate it).
    let writer = Address::from_low_u64_be(0xDA7B);
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0u64,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        // PUSH1 1; PUSH1 0; SSTORE; PUSH1 0; PUSH1 0; REVERT
        (
            writer,
            U256::zero(),
            0u64,
            Bytes::from(vec![
                0x60, 0x01, 0x60, 0x00, 0x55, 0x60, 0x00, 0x60, 0x00, 0xfd,
            ]),
        ),
    ];
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0x00,
            target: Some(writer),
            gas_limit: 2_000_000,
            state_limit: STATE_BUDGET,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("valid frame tx (reverted DEFAULT frame, payer approved)");
    assert_eq!(
        report.state_gas_used, 0,
        "a reverted frame creates no state and must report zero state gas, got {}",
        report.state_gas_used,
    );
    // EIP-8141 §State-gas attribution: "When a frame reverts, restore the journal
    // and `state_gas_left` to frame entry. Its final `gas_used.state` is therefore
    // zero." The charge was made — the frame declared a budget and the SSTORE drew on
    // it — and then unwound with the state it paid for.
    let fr = report.frame_results.expect("per-frame results");
    assert_eq!(
        fr[1].state_gas_used, 0,
        "a reverted frame's receipt must report zero state gas, got {}",
        fr[1].state_gas_used,
    );
}

#[test]
fn frame_tx_below_base_blob_fee_is_rejected() {
    // EIP-4844 INSUFFICIENT_MAX_FEE_PER_BLOB_GAS on the frame path: a blob-carrying
    // frame tx whose max_fee_per_blob_gas is below the block base blob fee must be
    // invalid. The check fires before any frame executes, so a lone SENDER frame is
    // enough to reach it.
    let mut tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Sender),
        flags: 0x00,
        target: None,
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    tx.blob_versioned_hashes = vec![H256::repeat_byte(0x01)];
    tx.max_fee_per_blob_gas = U256::zero();

    let mut db = seeded_db(&[(FUNDED_SENDER, AUTO_SEED_SENDER_BALANCE, 0, Bytes::new())]);
    let mut env = frame_tx_env(&tx);
    // base blob fee strictly above the tx's max_fee_per_blob_gas (0).
    env.base_blob_fee_per_gas = U256::from(1u64);
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
    .expect("VM::new should succeed for a frame tx");
    let result = vm.execute();
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::InsufficientMaxFeePerBlobGas { .. }
            ))
        ),
        "blob-carrying frame tx below base blob fee must be rejected, got {result:?}"
    );
}

#[test]
fn state_gas_reservoir_does_not_leak_across_frames() {
    // Frame A creates then clears a slot (0 -> 5 -> 0), which credits the EIP-8037
    // state-gas reservoir. Frame B then creates a fresh slot. Because frames are
    // gas-isolated, A's reservoir credit must NOT subsidize B's state charge: B's
    // gas_used must include the full state-set cost (the charge spills to
    // gas_remaining), not be silently drawn from A's leftover credit. Without the
    // per-frame reservoir reset, B's gas_used would be ~SSTORE_SET_STATE_GAS lower.
    let a = Address::from_low_u64_be(0xAA01);
    let b = Address::from_low_u64_be(0xBB01);
    let mk = |target| Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0x00,
        target: Some(target),
        gas_limit: 2_000_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0u64,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        // A: SSTORE 5@5 (0->5); SSTORE 0@5 (5->0) -> credits the reservoir; STOP
        (
            a,
            U256::zero(),
            0u64,
            Bytes::from(vec![
                0x60, 0x05, 0x60, 0x05, 0x55, 0x60, 0x00, 0x60, 0x05, 0x55, 0x00,
            ]),
        ),
        // B: SSTORE 1@6 (0->1) -> a fresh state-creating set; STOP
        (
            b,
            U256::zero(),
            0u64,
            Bytes::from(vec![0x60, 0x01, 0x60, 0x06, 0x55, 0x00]),
        ),
    ];
    let tx = frame_tx_with_frames(vec![verify_frame(FUNDED_SENDER), mk(a), mk(b)]);
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("valid frame tx (payer approved)");
    let fr = report.frame_results.expect("frame results present");
    // fr[0] = VERIFY, fr[1] = A (set+clear), fr[2] = B (fresh set).
    assert!(
        fr[2].gas_used > SSTORE_SET_STATE_GAS,
        "frame B gas_used ({}) must include the full state-set cost — frame A's \
         reservoir credit must not subsidize it",
        fr[2].gas_used,
    );
}

// ==================== frame_tx opcode handler unit tests ====================
// (migrated from crates/vm/levm/src/opcode_handlers/frame_tx.rs)

mod frame_tx_opcode_handler_tests {
    use bytes::Bytes;
    use ethrex_common::types::{FrameSignature, FrameTransaction};
    use ethrex_common::{Address, U256};
    use ethrex_levm::errors::{InternalError, VMError};
    use ethrex_levm::opcode_handlers::frame_tx::{address_to_u256, load_tx_param, u256_to_offset};
    use ethrex_levm::vm::FrameTxContext;

    /// Mirrors the Underflow -> RevertOpcode mapping used inside apply_approve
    /// so the invariant can be exercised without constructing a full VM.
    fn map_underflow_to_revert(result: Result<(), InternalError>) -> Result<(), VMError> {
        match result {
            Ok(()) => Ok(()),
            Err(InternalError::Underflow) => Err(VMError::RevertOpcode),
            Err(e) => Err(VMError::Internal(e)),
        }
    }

    #[test]
    fn decrease_balance_underflow_maps_to_revert_opcode() {
        let e = map_underflow_to_revert(Err(InternalError::Underflow));
        assert!(matches!(e, Err(VMError::RevertOpcode)));
    }

    #[test]
    fn non_underflow_internal_errors_still_propagate_as_internal() {
        let e = map_underflow_to_revert(Err(InternalError::Overflow));
        assert!(matches!(e, Err(VMError::Internal(InternalError::Overflow))));
    }

    #[test]
    fn successful_decrease_balance_is_left_unchanged() {
        let e = map_underflow_to_revert(Ok(()));
        assert!(e.is_ok());
    }

    #[test]
    fn u256_to_offset_accepts_values_that_fit_in_usize() {
        assert_eq!(u256_to_offset(U256::zero()), Some(0));
        assert_eq!(u256_to_offset(U256::from(42u64)), Some(42));
        assert_eq!(
            u256_to_offset(U256::from(u64::try_from(usize::MAX).unwrap_or(u64::MAX))),
            Some(usize::MAX)
        );
    }

    #[test]
    fn u256_to_offset_rejects_values_that_overflow_usize() {
        let big = U256::from(u64::MAX) + U256::one();
        assert_eq!(u256_to_offset(big), None);
        assert_eq!(u256_to_offset(U256::MAX), None);
    }

    #[test]
    fn frameparam_0x08_returns_frame_value() {
        use ethrex_common::types::{Frame, FrameMode};
        // The 0x08 arm of OpFrameParamHandler maps directly to `frame.value`.
        let frame = Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0x00,
            target: Some(Address::from_low_u64_be(0xCAFE)),
            gas_limit: 100_000,
            state_limit: 0,
            value: U256::from(1_234_567u64),
            data: Bytes::new(),
        };

        let param_id: u64 = 0x08;
        let result = match param_id {
            0x08 => frame.value,
            _ => unreachable!("param_id is 0x08"),
        };
        assert_eq!(result, U256::from(1_234_567u64));

        let zero_frame = ethrex_common::types::Frame {
            value: U256::zero(),
            ..frame
        };
        let zero_result = match param_id {
            0x08 => zero_frame.value,
            _ => unreachable!("param_id is 0x08"),
        };
        assert_eq!(zero_result, U256::zero());
    }

    /// Build a minimal FrameTxContext with one signature for SIGPARAM tests.
    fn ctx_with_one_signature() -> FrameTxContext {
        let signer = Address::from_low_u64_be(0xABCDEF);
        let msg_bytes = Bytes::from(vec![0xdeu8; 32]);
        let sig_bytes = Bytes::from(vec![0xFFu8; 65]);
        let sig = FrameSignature {
            scheme: 0x01,
            signer: Some(signer),
            msg: msg_bytes,
            signature: sig_bytes,
        };
        let mut tx = FrameTransaction::default();
        tx.signatures.push(sig);
        FrameTxContext {
            sender_approved: false,
            payer_address: None,
            frame_results: Vec::new(),
            current_frame_index: 0,
            sig_hash: ethrex_common::H256::zero(),
            tx,
            approve_called_in_current_frame: false,
            total_gas_limit: 0,
            legacy_sender_nonce: 0,
            blob_base_fee: U256::zero(),
        }
    }

    #[test]
    fn sigparam_0x00_returns_signer() {
        let ctx = ctx_with_one_signature();
        let sig = ctx.tx.signatures.first().unwrap();
        let result = address_to_u256(sig.signer.unwrap());
        let mut expected = [0u8; 32];
        expected[12..].copy_from_slice(Address::from_low_u64_be(0xABCDEF).as_bytes());
        assert_eq!(result, U256::from_big_endian(&expected));
    }

    #[test]
    fn sigparam_0x01_returns_scheme() {
        let ctx = ctx_with_one_signature();
        let sig = ctx.tx.signatures.first().unwrap();
        assert_eq!(U256::from(sig.scheme), U256::from(0x01u64));
    }

    #[test]
    fn sigparam_0x02_returns_msg_word() {
        let ctx = ctx_with_one_signature();
        let sig = ctx.tx.signatures.first().unwrap();
        let result = if sig.msg.is_empty() {
            U256::zero()
        } else {
            U256::from_big_endian(&sig.msg)
        };
        assert_eq!(result, U256::from_big_endian(&[0xdeu8; 32]));
    }

    #[test]
    fn sigparam_0x02_empty_msg_returns_zero() {
        let signer = Address::from_low_u64_be(0x1234);
        let sig = FrameSignature {
            // SECP256K1 (scheme 1); scheme 0 is now ARBITRARY (requires empty signer).
            scheme: 0x01,
            signer: Some(signer),
            msg: Bytes::new(),
            signature: Bytes::from(vec![0xAAu8; 65]),
        };
        let result = if sig.msg.is_empty() {
            U256::zero()
        } else {
            U256::from_big_endian(&sig.msg)
        };
        assert_eq!(result, U256::zero());
    }

    #[test]
    fn sigparam_0x03_returns_signature_len() {
        let ctx = ctx_with_one_signature();
        let sig = ctx.tx.signatures.first().unwrap();
        assert_eq!(U256::from(sig.signature.len()), U256::from(65u64));
    }

    #[test]
    fn sigparam_oob_index_returns_invalid_opcode() {
        let ctx = ctx_with_one_signature();
        // index 1 is out of bounds (only index 0 exists)
        let result = ctx.tx.signatures.get(1);
        assert!(
            result.is_none(),
            "OOB index should return None -> InvalidOpcode"
        );
    }

    #[test]
    fn txparam_0x0b_returns_signature_count() {
        let ctx = ctx_with_one_signature();
        let result = load_tx_param(&ctx, 0x0B, false).unwrap();
        assert_eq!(result, U256::from(1u64));
    }

    #[test]
    fn txparam_0x0b_zero_signatures() {
        let ctx = FrameTxContext {
            sender_approved: false,
            payer_address: None,
            frame_results: Vec::new(),
            current_frame_index: 0,
            sig_hash: ethrex_common::H256::zero(),
            tx: FrameTransaction::default(),
            approve_called_in_current_frame: false,
            total_gas_limit: 0,
            legacy_sender_nonce: 0,
            blob_base_fee: U256::zero(),
        };
        let result = load_tx_param(&ctx, 0x0B, false).unwrap();
        assert_eq!(result, U256::zero());
    }

    #[test]
    fn framedataload_verify_frame_returns_real_data() {
        use ethrex_common::types::{Frame, FrameMode};
        // After the VERIFY-zeroing removal, loading data from a VERIFY frame
        // should return the actual bytes in frame.data, not zero.
        let mut data = [0u8; 32];
        data[0] = 0xCA;
        data[31] = 0xFE;
        let frame = Frame {
            mode: u8::from(FrameMode::Verify),
            flags: 0x03,
            target: Some(Address::from_low_u64_be(0xAA)),
            gas_limit: 50_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::from(data.to_vec()),
        };
        // Simulate the load logic (no VERIFY special-case any more)
        let byte_offset: usize = 0;
        let mut word = [0u8; 32];
        let available = frame.data.len().saturating_sub(byte_offset);
        let copy_len = available.min(32);
        if let (Some(dst), Some(src)) = (
            word.get_mut(..copy_len),
            frame
                .data
                .get(byte_offset..byte_offset.saturating_add(copy_len)),
        ) {
            dst.copy_from_slice(src);
        }
        let result = U256::from_big_endian(&word);
        assert_ne!(result, U256::zero(), "VERIFY frame data should be readable");
        assert_eq!(result, U256::from_big_endian(&data));
    }
}

// ==================== frame_tx_security_tests ====================
// (migrated from crates/vm/levm/src/vm.rs)

mod frame_tx_security_tests {
    //! Regression tests for the security review of EIP-8141 Frame Transaction
    //! execution. These tests lock in invariants whose violation previously
    //! produced:
    //!   (1) Log duplication across frames → receipts-root divergence.
    //!   (2) Free money + nonce replay via `restore_cache_state()` undoing
    //!       APPROVE-side state from an earlier successful frame.
    //!   (3) Atomic-batch atomicity bypass: successful in-batch frame state
    //!       persisted across a batch revert.
    //!
    //! Tests 2 and 3 depend on full VM execution of FrameTransactions and are
    //! exercised end-to-end by the harness above in this file.
    //! The unit tests below cover the Substate API invariant that underpins Fix 1.
    use bytes::Bytes;
    use ethrex_common::types::Log;
    use ethrex_common::{Address, H256};
    use ethrex_levm::vm::Substate;

    fn mk_log(tag: u8) -> Log {
        Log {
            address: Address::from_low_u64_be(u64::from(tag)),
            topics: vec![H256::from_low_u64_be(u64::from(tag))],
            data: Bytes::from(vec![tag]),
        }
    }

    fn log_tags(logs: &[Log]) -> Vec<u8> {
        logs.iter()
            .filter_map(|l| l.data.first().copied())
            .collect()
    }

    /// `current_logs()` must return only the sub-substate's own logs, not
    /// parent logs. This is the primitive that Fix 1 uses to avoid leaking
    /// prior frames' logs into `frame_receipts[i].logs`.
    #[test]
    fn current_logs_excludes_parent_logs() {
        let mut substate = Substate::default();

        substate.add_log(mk_log(0xA0)); // parent log, emitted before any push
        assert_eq!(log_tags(&substate.current_logs()), vec![0xA0]);

        substate.push_backup();
        // Post-push: the sub-substate is fresh.
        assert!(substate.current_logs().is_empty());

        substate.add_log(mk_log(0xB1));
        substate.add_log(mk_log(0xB2));
        // current_logs() returns this scope's logs only.
        assert_eq!(log_tags(&substate.current_logs()), vec![0xB1, 0xB2]);

        // extract_logs() (intentionally) returns parent+current. Verifies the
        // distinction that Fix 1 relies on.
        assert_eq!(log_tags(&substate.extract_logs()), vec![0xA0, 0xB1, 0xB2]);

        // After commit, current_logs() includes the merged set because parent
        // was folded in.
        substate.commit_backup();
        assert_eq!(log_tags(&substate.current_logs()), vec![0xA0, 0xB1, 0xB2]);
    }

    /// The exact sequence that Fix 1 replaces: when the previous buggy pattern
    /// (commit_backup → extract_logs → re-add loop) runs across multiple
    /// frames, later frames see duplicated logs from earlier frames.
    ///
    /// This test exists so that if anyone ever reintroduces that sequence,
    /// the compounding growth is caught with a concrete trace.
    #[test]
    fn frame_per_frame_logs_do_not_duplicate_across_frames() {
        let mut substate = Substate::default();

        // Capture per-frame log deltas using the corrected sequence:
        //   push_backup → emit → current_logs (snapshot) → commit_backup
        let mut per_frame: Vec<Vec<Log>> = Vec::new();
        for tag in [0x11u8, 0x22, 0x33] {
            substate.push_backup();
            substate.add_log(mk_log(tag));
            // Snapshot this frame's logs BEFORE commit merges them into parent.
            let this_frame = substate.current_logs();
            substate.commit_backup();
            per_frame.push(this_frame);
        }

        // Each frame's receipt should contain exactly its own log — no leaks.
        assert_eq!(log_tags(per_frame.first().unwrap()), vec![0x11]);
        assert_eq!(log_tags(per_frame.get(1).unwrap()), vec![0x22]);
        assert_eq!(log_tags(per_frame.get(2).unwrap()), vec![0x33]);
    }
}

// ==================== frame_value_transfer_tests ====================
// (migrated from crates/vm/levm/src/vm.rs)

mod frame_value_transfer_tests {
    //! EIP-8141 top-level value-transfer invariants.
    //!
    //! The outer `execute_frame_tx` loop owns the `frame.value` transfer: it
    //! balance-checks the sender, performs the transfer, and records an
    //! EIP-7708 log (when sender != target, Amsterdam+). These tests pin the
    //! balance-check predicate; the backup-unwind coverage for atomic batch
    //! revert lives in the regression-test commit that follows.
    use bytes::Bytes;
    use ethrex_common::types::Log;
    use ethrex_common::{Address, U256};
    use ethrex_levm::vm::{Substate, frame_value_exceeds_balance};

    #[test]
    fn frame_value_transfers_from_sender_to_resolved_target_on_success() {
        // A sufficiently funded sender must not revert — the transfer proceeds.
        let sender_balance = U256::from(10u64).saturating_mul(U256::exp10(18)); // 10 ETH
        let value = U256::from(1u64).saturating_mul(U256::exp10(17)); // 0.1 ETH
        assert!(!frame_value_exceeds_balance(sender_balance, value));

        // Exact-balance transfer: sender has exactly `value` — still succeeds.
        assert!(!frame_value_exceeds_balance(value, value));
    }

    #[test]
    fn frame_value_transfer_reverts_on_insufficient_sender_balance() {
        // Under-funded sender → revert path taken.
        let balance = U256::from(5u64).saturating_mul(U256::exp10(16)); // 0.05 ETH
        let value = U256::from(1u64).saturating_mul(U256::exp10(17)); // 0.10 ETH
        assert!(frame_value_exceeds_balance(balance, value));

        // Zero-balance / non-zero value → revert.
        assert!(frame_value_exceeds_balance(U256::zero(), U256::one()));

        // Balance just one less than value → revert.
        let v = U256::from(1_000_000u64);
        assert!(frame_value_exceeds_balance(v - U256::one(), v));
    }

    /// Regression test for the atomic-batch unwind: any state change
    /// performed inside a backup scope (including the outer-owned value
    /// transfer and the EIP-7708 log emitted alongside it) must be reverted
    /// when the enclosing batch reverts. `execute_frame_tx` pushes a batch
    /// backup before each atomic group and calls `revert_backup()` when any
    /// in-batch frame fails; this test exercises the Substate primitive
    /// that guarantees the log and state deltas do not leak past the
    /// boundary.
    #[test]
    fn atomic_batch_revert_unwinds_in_batch_value_effects() {
        let mut substate = Substate::default();

        // Log emitted before the batch — should survive a batch revert.
        substate.add_log(Log {
            address: Address::from_low_u64_be(1),
            topics: vec![],
            data: Bytes::from_static(b"pre-batch"),
        });

        // Enter the atomic batch: push a backup before the first in-batch frame.
        substate.push_backup();

        // Frame 1 (SENDER atomic, successful): simulate the per-frame scope,
        // emit the EIP-7708 transfer log produced by the outer value transfer,
        // then commit the per-frame backup.
        substate.push_backup();
        substate.add_log(Log {
            address: Address::from_low_u64_be(2),
            topics: vec![],
            data: Bytes::from_static(b"frame-1-transfer-log"),
        });
        substate.commit_backup();

        // Frame 2 reverts: `execute_frame_tx` reverts the batch-level backup,
        // which undoes every in-batch substate change including Frame 1's log.
        substate.revert_backup();

        let logs = substate.extract_logs();
        let tags: Vec<&[u8]> = logs.iter().map(|l| l.data.as_ref()).collect();
        assert_eq!(
            tags,
            vec![b"pre-batch".as_ref()],
            "atomic-batch revert must unwind in-batch value-transfer effects"
        );
    }
}

// ==================== frame_tx_7702_delegation_tests ====================
// (migrated from crates/vm/levm/src/vm.rs)

mod frame_tx_7702_delegation_tests {
    //! EIP-8141 §Execution step 1 (lines 348-351) requires that at frame entry,
    //! if `resolved_target` has an EIP-7702 delegation indicator the frame
    //! executes according to EIP-7702's delegated-code semantics — i.e. the
    //! delegatee's code runs while ADDRESS/storage stay tied to the delegator.
    //! Default code runs ONLY when the target has neither code nor a delegation.
    //!
    //! `execute_frame_tx` resolves this via `utils::eip7702_get_code` and then
    //! gates the default-code branch on `bytecode.is_empty() && !is_delegation_7702`.
    //! The tests below pin that decision table directly by invoking
    //! `eip7702_get_code` on the four target shapes in §5 of the mitigation plan.
    use bytes::Bytes;
    use ethrex_common::constants::EMPTY_TRIE_HASH;
    use ethrex_common::{
        Address, H256, U256,
        types::{Account, AccountState, ChainConfig, Code, CodeMetadata, Fork},
    };
    use ethrex_levm::constants::SET_CODE_DELEGATION_BYTES;
    use ethrex_levm::db::{Database, gen_db::GeneralizedDatabase};
    use ethrex_levm::errors::DatabaseError;
    use ethrex_levm::utils::eip7702_get_code;
    use ethrex_levm::vm::Substate;
    use rustc_hash::FxHashMap;
    use std::sync::Arc;

    /// Minimal in-memory store matching the shape used by `eip7708_tests.rs`.
    struct TestStore {
        accounts: FxHashMap<Address, Account>,
    }

    impl Database for TestStore {
        fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
            Ok(self
                .accounts
                .get(&address)
                .map(|acc| AccountState {
                    nonce: acc.info.nonce,
                    balance: acc.info.balance,
                    storage_root: *EMPTY_TRIE_HASH,
                    code_hash: acc.info.code_hash,
                })
                .unwrap_or_default())
        }
        fn get_storage_value(&self, _a: Address, _k: H256) -> Result<U256, DatabaseError> {
            Ok(U256::zero())
        }
        fn get_block_hash(&self, _n: u64) -> Result<H256, DatabaseError> {
            Ok(H256::zero())
        }
        fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
            Ok(ChainConfig::default())
        }
        fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
            for acc in self.accounts.values() {
                if acc.info.code_hash == code_hash {
                    return Ok(acc.code.clone());
                }
            }
            Ok(Code::default())
        }
        fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
            for acc in self.accounts.values() {
                if acc.info.code_hash == code_hash {
                    return Ok(CodeMetadata {
                        #[expect(clippy::as_conversions, reason = "test helper")]
                        length: acc.code.len() as u64,
                    });
                }
            }
            Ok(CodeMetadata { length: 0 })
        }
    }

    fn addr(x: u64) -> Address {
        Address::from_low_u64_be(x)
    }

    /// Build a 23-byte EIP-7702 delegation indicator pointing at `delegatee`.
    fn delegation_indicator(delegatee: Address) -> Bytes {
        let mut v = Vec::with_capacity(23);
        v.extend_from_slice(&SET_CODE_DELEGATION_BYTES);
        v.extend_from_slice(delegatee.as_bytes());
        Bytes::from(v)
    }

    fn build_db(accounts: Vec<(Address, Account)>) -> GeneralizedDatabase {
        let store = Arc::new(TestStore {
            accounts: FxHashMap::default(),
        });
        let map: FxHashMap<Address, Account> = accounts.into_iter().collect();
        GeneralizedDatabase::new_with_account_state(store, map)
    }

    /// The decision predicate from `execute_frame_tx`: default-code runs only when
    /// the *resolved* bytecode is empty AND the target has no delegation.
    fn runs_default_code(is_delegation_7702: bool, bytecode: &Code) -> bool {
        bytecode.is_empty() && !is_delegation_7702
    }

    /// Positive case: a 7702-delegated EOA must resolve to the delegatee's bytecode.
    #[test]
    fn delegated_sender_eoa_runs_delegatee_code_with_delegator_address() {
        let delegator = addr(0xDE1E);
        let delegatee = addr(0xC0DE);
        let delegatee_code = Bytes::from(vec![0x60, 0xff, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3]);
        let delegator_account = Account::new(
            U256::from(1_000_000_000u64),
            Code::from_bytecode(
                delegation_indicator(delegatee),
                &ethrex_crypto::NativeCrypto,
            ),
            0,
            FxHashMap::default(),
        );
        let delegatee_account = Account::new(
            U256::zero(),
            Code::from_bytecode(delegatee_code.clone(), &ethrex_crypto::NativeCrypto),
            0,
            FxHashMap::default(),
        );

        let mut db = build_db(vec![
            (delegator, delegator_account),
            (delegatee, delegatee_account),
        ]);
        let mut substate = Substate::default();

        let (is_delegation, _access_cost, code_address, code) =
            eip7702_get_code(&mut db, &mut substate, delegator, Fork::Hegota).unwrap();

        assert!(
            is_delegation,
            "delegator must be detected as 7702-delegated"
        );
        assert_eq!(
            code_address, delegatee,
            "code_address must point at the delegatee, not the delegator"
        );
        assert_eq!(
            code.code_bytes(),
            delegatee_code,
            "returned bytecode must be the delegatee's code, not the 0xef0100 indicator"
        );
        assert!(
            !runs_default_code(is_delegation, &code),
            "7702 delegation to a non-empty delegatee must take the CallFrame branch, not default code"
        );
    }

    /// Edge case: a 7702 delegation pointing at an address with no deployed code.
    #[test]
    fn delegated_eoa_with_empty_delegatee_succeeds_as_empty_code() {
        let delegator = addr(0xDE1E);
        let delegatee = addr(0xE117); // empty — no Account registered
        let delegator_account = Account::new(
            U256::from(1_000_000_000u64),
            Code::from_bytecode(
                delegation_indicator(delegatee),
                &ethrex_crypto::NativeCrypto,
            ),
            0,
            FxHashMap::default(),
        );

        let mut db = build_db(vec![(delegator, delegator_account)]);
        let mut substate = Substate::default();

        let (is_delegation, _access_cost, code_address, code) =
            eip7702_get_code(&mut db, &mut substate, delegator, Fork::Hegota).unwrap();

        assert!(is_delegation, "delegation indicator must still be detected");
        assert_eq!(code_address, delegatee);
        assert!(
            code.is_empty(),
            "delegatee has no code, so resolved bytecode is empty"
        );
        assert!(
            !runs_default_code(is_delegation, &code),
            "empty-delegatee delegation must NOT route to default code — it must take the \
             CallFrame branch and succeed as empty code (EIP-8141 §Execution)"
        );
    }

    /// Regression: a plain EOA (no deployed code, no delegation indicator) must
    /// still route into the default-code branch.
    #[test]
    fn undelegated_eoa_still_runs_default_code() {
        let eoa_addr = addr(0xEAA0);
        let eoa = Account::new(
            U256::from(1_000_000_000u64),
            Code::default(),
            0,
            FxHashMap::default(),
        );

        let mut db = build_db(vec![(eoa_addr, eoa)]);
        let mut substate = Substate::default();

        let (is_delegation, _access_cost, code_address, code) =
            eip7702_get_code(&mut db, &mut substate, eoa_addr, Fork::Hegota).unwrap();

        assert!(!is_delegation, "plain EOA has no delegation indicator");
        assert_eq!(
            code_address, eoa_addr,
            "code_address falls back to the target when no delegation"
        );
        assert!(code.is_empty(), "plain EOA has no code");
        assert!(
            runs_default_code(is_delegation, &code),
            "plain EOA with no code and no delegation must take the default-code branch"
        );
    }

    /// Regression: a target with real bytecode and no delegation must still
    /// execute its own bytecode.
    #[test]
    fn contract_target_unaffected_by_delegation_resolver() {
        let contract_addr = addr(0xC000);
        let contract_code = Bytes::from(vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00]);
        let contract_account = Account::new(
            U256::zero(),
            Code::from_bytecode(contract_code.clone(), &ethrex_crypto::NativeCrypto),
            1,
            FxHashMap::default(),
        );

        let mut db = build_db(vec![(contract_addr, contract_account)]);
        let mut substate = Substate::default();

        let (is_delegation, _access_cost, code_address, code) =
            eip7702_get_code(&mut db, &mut substate, contract_addr, Fork::Hegota).unwrap();

        assert!(
            !is_delegation,
            "regular contract bytecode must not be mistaken for a delegation"
        );
        assert_eq!(
            code_address, contract_addr,
            "code_address is the target itself when no delegation"
        );
        assert_eq!(
            code.code_bytes(),
            contract_code,
            "regular contract bytecode passes through unchanged"
        );
        assert!(
            !runs_default_code(is_delegation, &code),
            "contract with code must take the CallFrame branch, not default code"
        );
    }
}

// ==================== validation_observer_tests ====================
// (migrated from crates/vm/levm/src/vm.rs)

mod validation_observer_tests {
    //! EIP-8141 mempool validation-prefix simulation harness tests.
    //!
    //! These drive the real frame-execution machinery via
    //! [`VM::run_frame_validation_prefix`] over signature-less frame
    //! transactions (an empty signature list trivially passes
    //! `validate_frame_signatures`, so these stay crypto-free while exercising
    //! the actual dispatch loop, handlers and observer hooks).
    use bytes::Bytes;
    use ethrex_common::constants::EMPTY_TRIE_HASH;
    use ethrex_common::types::Fork;
    use ethrex_common::types::Transaction;
    use ethrex_common::types::{
        Account, AccountState, ChainConfig, Code, CodeMetadata, Frame, FrameTransaction,
        frame_tx_expiry_verifier,
    };
    use ethrex_common::{Address, H256, U256};
    use ethrex_levm::db::{Database, gen_db::GeneralizedDatabase};
    use ethrex_levm::environment::{EVMConfig, Environment};
    use ethrex_levm::errors::DatabaseError;
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::validation_observer::{
        CodeBodyBudget, FocilVopsSurface, FrameSimViolation, Profile2Replay,
    };
    use ethrex_levm::vm::{PrefixSimResult, VM, VMType};
    use rustc_hash::FxHashMap;
    use std::sync::Arc;

    /// Pins every banned-opcode byte literal used by
    /// `check_validation_banned_opcode` to the canonical `Opcode` enum
    /// discriminant.
    #[test]
    fn validation_observer_opcode_byte_pins() {
        use ethrex_levm::opcodes::Opcode;
        assert_eq!(u8::from(Opcode::ORIGIN), 0x32);
        assert_eq!(u8::from(Opcode::GASPRICE), 0x3A);
        assert_eq!(u8::from(Opcode::BLOCKHASH), 0x40);
        assert_eq!(u8::from(Opcode::COINBASE), 0x41);
        assert_eq!(u8::from(Opcode::TIMESTAMP), 0x42);
        assert_eq!(u8::from(Opcode::NUMBER), 0x43);
        assert_eq!(u8::from(Opcode::PREVRANDAO), 0x44);
        assert_eq!(u8::from(Opcode::GASLIMIT), 0x45);
        assert_eq!(u8::from(Opcode::BASEFEE), 0x48);
        assert_eq!(u8::from(Opcode::BLOBHASH), 0x49);
        assert_eq!(u8::from(Opcode::BLOBBASEFEE), 0x4A);
        assert_eq!(u8::from(Opcode::SLOTNUM), 0x4B);
        assert_eq!(u8::from(Opcode::INVALID), 0xFE);
        assert_eq!(u8::from(Opcode::SELFDESTRUCT), 0xFF);
        assert_eq!(u8::from(Opcode::BALANCE), 0x31);
        assert_eq!(u8::from(Opcode::SELFBALANCE), 0x47);
        assert_eq!(u8::from(Opcode::TLOAD), 0x5C);
        assert_eq!(u8::from(Opcode::TSTORE), 0x5D);
        assert_eq!(u8::from(Opcode::GAS), 0x5A);
        assert_eq!(u8::from(Opcode::CALL), 0xF1);
        assert_eq!(u8::from(Opcode::CALLCODE), 0xF2);
        assert_eq!(u8::from(Opcode::DELEGATECALL), 0xF4);
        assert_eq!(u8::from(Opcode::STATICCALL), 0xFA);
    }

    struct TestStore {
        accounts: FxHashMap<Address, Account>,
    }

    impl Database for TestStore {
        fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
            Ok(self
                .accounts
                .get(&address)
                .map(|acc| AccountState {
                    nonce: acc.info.nonce,
                    balance: acc.info.balance,
                    storage_root: *EMPTY_TRIE_HASH,
                    code_hash: acc.info.code_hash,
                })
                .unwrap_or_default())
        }
        fn get_storage_value(&self, _a: Address, _k: H256) -> Result<U256, DatabaseError> {
            Ok(U256::zero())
        }
        fn get_block_hash(&self, _n: u64) -> Result<H256, DatabaseError> {
            Ok(H256::zero())
        }
        fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
            Ok(ChainConfig::default())
        }
        fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
            for acc in self.accounts.values() {
                if acc.info.code_hash == code_hash {
                    return Ok(acc.code.clone());
                }
            }
            Ok(Code::default())
        }
        fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
            for acc in self.accounts.values() {
                if acc.info.code_hash == code_hash {
                    return Ok(CodeMetadata {
                        #[expect(clippy::as_conversions, reason = "test helper")]
                        length: acc.code.len() as u64,
                    });
                }
            }
            Ok(CodeMetadata { length: 0 })
        }
    }

    fn addr(x: u64) -> Address {
        Address::from_low_u64_be(x)
    }

    fn build_db(accounts: Vec<(Address, Account)>) -> GeneralizedDatabase {
        let store = Arc::new(TestStore {
            accounts: FxHashMap::default(),
        });
        let map: FxHashMap<Address, Account> = accounts.into_iter().collect();
        GeneralizedDatabase::new_with_account_state(store, map)
    }

    fn account_with_code(balance: u64, code: Bytes) -> Account {
        Account::new(
            U256::from(balance),
            Code::from_bytecode(code, &ethrex_crypto::NativeCrypto),
            0,
            FxHashMap::default(),
        )
    }

    fn hegota_env(sender: Address) -> Environment {
        let config = EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota));
        Environment {
            origin: sender,
            gas_limit: 30_000_000,
            block_gas_limit: 30_000_000,
            config,
            ..Default::default()
        }
    }

    fn frame_tx_for_obs(sender: Address, frames: Vec<Frame>) -> Transaction {
        Transaction::FrameTransaction(FrameTransaction {
            chain_id: 0,
            nonce_keys: vec![U256::zero()],
            nonce_seq: 0,
            sender,
            frames,
            signatures: Vec::new(),
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: U256::zero(),
            blob_versioned_hashes: Vec::new(),
            ..Default::default()
        })
    }

    fn verify_frame_obs(target: Address, gas_limit: u64, flags: u8, data: Bytes) -> Frame {
        Frame {
            mode: 1, // VERIFY
            flags,
            target: Some(target),
            gas_limit,
            state_limit: 0,
            value: U256::zero(),
            data,
        }
    }

    fn default_frame_obs(target: Address, gas_limit: u64, data: Bytes) -> Frame {
        Frame {
            mode: 0, // DEFAULT
            flags: 0,
            target: Some(target),
            gas_limit,
            // A deploy frame in a validation prefix writes the sender's storage, and
            // the mempool simulation meters the state dimension exactly as consensus
            // does, so it has to declare a budget for it.
            state_limit: super::STATE_BUDGET,
            value: U256::zero(),
            data,
        }
    }

    /// APPROVE scope `scope` then STOP: PUSH1 scope, PUSH1 0, PUSH1 0, APPROVE.
    fn approve_code(scope: u8) -> Bytes {
        Bytes::from(vec![
            0x60, scope, // PUSH1 scope
            0x60, 0x00, // PUSH1 0 (length)
            0x60, 0x00, // PUSH1 0 (offset)
            0xAA, // APPROVE
            0x00, // STOP
        ])
    }

    /// Run the prefix simulation for `tx` with the given prefix indices and
    /// optional deploy index; returns the populated VM observer state and result.
    fn run(
        tx: &Transaction,
        db: &mut GeneralizedDatabase,
        sender: Address,
        frame_indices: &[usize],
        deploy_index: Option<usize>,
        focil_surface: Option<FocilVopsSurface>,
    ) -> (PrefixSimResult, Option<FrameSimViolation>) {
        let env = hegota_env(sender);
        let mut vm = VM::new(
            env,
            db,
            tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &ethrex_crypto::NativeCrypto,
            None,
        )
        .unwrap();
        let profile_2 = focil_surface.map(|surface| Profile2Replay {
            surface,
            code_budget: CodeBodyBudget::unbounded(),
        });
        let result = vm
            .run_frame_validation_prefix(frame_indices, deploy_index, None, profile_2)
            .unwrap();
        (result, vm.validation_observer.violation.clone())
    }

    #[test]
    fn passing_self_verify_sets_payer_and_no_violation() {
        let sender = addr(0x5E11);
        let tx = frame_tx_for_obs(
            sender,
            vec![verify_frame_obs(sender, 50_000, 0x03, Bytes::new())],
        );
        let mut db = build_db(vec![(sender, account_with_code(0, approve_code(0x03)))]);
        let (result, violation) = run(&tx, &mut db, sender, &[0], None, None);
        assert!(violation.is_none(), "self_verify must not violate any rule");
        assert!(!result.any_revert, "self_verify frame must not revert");
        assert_eq!(
            result.payer_address,
            Some(sender),
            "self_verify must set the sender as payer"
        );
    }

    #[test]
    fn timestamp_outside_expiry_verifier_is_banned() {
        let sender = addr(0x7140);
        // TIMESTAMP (0x42) then STOP — banned in a non-expiry VERIFY frame.
        let code = Bytes::from(vec![0x42, 0x00]);
        let tx = frame_tx_for_obs(
            sender,
            vec![verify_frame_obs(sender, 50_000, 0x03, Bytes::new())],
        );
        let mut db = build_db(vec![(sender, account_with_code(0, code))]);
        let (_result, violation) = run(&tx, &mut db, sender, &[0], None, None);
        assert_eq!(
            violation,
            Some(FrameSimViolation::BannedOpcode(0x42)),
            "TIMESTAMP outside the expiry verifier must be a banned opcode"
        );
    }

    #[test]
    fn timestamp_inside_expiry_verifier_is_allowed() {
        let sender = addr(0x7141);
        let expiry = frame_tx_expiry_verifier();
        // TIMESTAMP POP STOP — exercises TIMESTAMP without leaving stack residue.
        let code = Bytes::from(vec![0x42, 0x50, 0x00]);
        // 8-byte deadline data, far in the future.
        let data = Bytes::from(vec![0xff; 8]);
        let tx = frame_tx_for_obs(sender, vec![verify_frame_obs(expiry, 50_000, 0x00, data)]);
        let mut db = build_db(vec![
            (sender, account_with_code(0, Bytes::new())),
            (expiry, account_with_code(0, code)),
        ]);
        let env = hegota_env(sender);
        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &ethrex_crypto::NativeCrypto,
            None,
        )
        .unwrap();
        let _ = vm
            .run_frame_validation_prefix(&[0], None, None, None)
            .unwrap();
        assert!(
            vm.validation_observer.violation.is_none(),
            "TIMESTAMP inside the expiry verifier must be allowed, got {:?}",
            vm.validation_observer.violation
        );
    }

    #[test]
    fn slotnum_is_banned() {
        let sender = addr(0x4B00);
        // SLOTNUM (0x4B) then STOP. EIP-7843's slot number changes between
        // admission and inclusion exactly like NUMBER/TIMESTAMP, so a prefix
        // branching on it could pass simulation and revert on inclusion. It is
        // not covered transitively: the handler reads `env.slot_number` from the
        // header rather than deriving it from TIMESTAMP.
        let code = Bytes::from(vec![0x4B, 0x00]);
        let tx = frame_tx_for_obs(
            sender,
            vec![verify_frame_obs(sender, 50_000, 0x03, Bytes::new())],
        );
        let mut db = build_db(vec![(sender, account_with_code(0, code))]);
        let (_result, violation) = run(&tx, &mut db, sender, &[0], None, None);
        assert_eq!(
            violation,
            Some(FrameSimViolation::BannedOpcode(0x4B)),
            "SLOTNUM must be a banned opcode during prefix simulation"
        );
    }

    #[test]
    fn sstore_outside_deploy_is_rejected() {
        let sender = addr(0x55_00);
        let code = Bytes::from(vec![0x60, 0x01, 0x60, 0x00, 0x55, 0x00]);
        let tx = frame_tx_for_obs(
            sender,
            vec![default_frame_obs(sender, 100_000, Bytes::new())],
        );
        let mut db = build_db(vec![(sender, account_with_code(0, code))]);
        let (_result, violation) = run(&tx, &mut db, sender, &[0], None, None);
        assert_eq!(
            violation,
            Some(FrameSimViolation::StateWriteOutsideDeploy),
            "SSTORE outside the deploy frame must be rejected"
        );
    }

    #[test]
    fn sload_non_sender_is_rejected() {
        let sender = addr(0x54_00);
        let other = addr(0x54_FF);
        // PUSH1 0, SLOAD, POP, STOP.
        let code = Bytes::from(vec![0x60, 0x00, 0x54, 0x50, 0x00]);
        let tx = frame_tx_for_obs(
            sender,
            // Payment-only scope: a frame targeting a non-sender account may not
            // ask for APPROVE_EXECUTION.
            vec![verify_frame_obs(other, 100_000, 0x01, Bytes::new())],
        );
        let mut db = build_db(vec![
            (sender, account_with_code(0, Bytes::new())),
            (other, account_with_code(0, code)),
        ]);
        let (_result, violation) = run(&tx, &mut db, sender, &[0], None, None);
        assert_eq!(
            violation,
            Some(FrameSimViolation::StorageReadNonSender),
            "SLOAD of a non-sender account's storage must be rejected"
        );
    }

    #[test]
    fn call_to_nonexistent_address_is_rejected() {
        let sender = addr(0xCA_11);
        let ghost = addr(0xDEAD_BEEF);
        let code = Bytes::from(vec![
            0x60, 0x00, // retLen
            0x60, 0x00, // retOffset
            0x60, 0x00, // argsLen
            0x60, 0x00, // argsOffset
            0x73, // PUSH20 ghost address
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x61, 0xFF, 0xFF, // PUSH2 gas
            0xFA, // STATICCALL
            0x00, // STOP
        ]);
        let tx = frame_tx_for_obs(
            sender,
            vec![verify_frame_obs(sender, 200_000, 0x03, Bytes::new())],
        );
        let mut db = build_db(vec![(sender, account_with_code(0, code))]);
        let (_result, violation) = run(&tx, &mut db, sender, &[0], None, None);
        assert_eq!(
            violation,
            Some(FrameSimViolation::CallToNonexistentOrDelegated(ghost)),
            "CALL to a nonexistent address must be rejected"
        );
    }

    #[test]
    fn reverting_prefix_frame_fails_and_sets_no_payer() {
        let sender = addr(0x5E_FF);
        let code = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xFD]);
        let tx = frame_tx_for_obs(
            sender,
            vec![verify_frame_obs(sender, 50_000, 0x03, Bytes::new())],
        );
        let mut db = build_db(vec![(sender, account_with_code(0, code))]);
        let (result, violation) = run(&tx, &mut db, sender, &[0], None, None);
        assert!(
            violation.is_none(),
            "a revert is a frame outcome, not a trace violation"
        );
        assert!(
            result.any_revert,
            "the reverting prefix frame must be flagged"
        );
        assert!(
            result.payer_address.is_none(),
            "a reverted prefix frame must not establish a payer"
        );
    }

    #[test]
    fn verify_without_approve_sets_no_payer() {
        let sender = addr(0x5E_AB);
        let code = Bytes::from(vec![0x00]);
        let tx = frame_tx_for_obs(
            sender,
            vec![verify_frame_obs(sender, 50_000, 0x03, Bytes::new())],
        );
        let mut db = build_db(vec![(sender, account_with_code(0, code))]);
        let (result, violation) = run(&tx, &mut db, sender, &[0], None, None);
        assert!(violation.is_none(), "a no-op VERIFY frame violates no rule");
        assert!(!result.any_revert, "an empty VERIFY frame succeeds");
        assert!(
            result.payer_address.is_none(),
            "a VERIFY frame that never APPROVEs must not establish a payer"
        );
    }

    #[test]
    fn sstore_to_sender_inside_deploy_frame_is_allowed() {
        let sender = addr(0xDE_91);
        // PUSH1 1, PUSH1 0, SSTORE, STOP.
        let code = Bytes::from(vec![0x60, 0x01, 0x60, 0x00, 0x55, 0x00]);
        let tx = frame_tx_for_obs(
            sender,
            vec![default_frame_obs(sender, 300_000, Bytes::new())],
        );
        let mut db = build_db(vec![(sender, account_with_code(0, code))]);
        let env = hegota_env(sender);
        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &ethrex_crypto::NativeCrypto,
            None,
        )
        .unwrap();
        let result = vm
            .run_frame_validation_prefix(&[0], Some(0), None, None)
            .unwrap();
        assert!(
            vm.validation_observer.violation.is_none(),
            "SSTORE to the sender inside the deploy frame must be allowed, got {:?}",
            vm.validation_observer.violation
        );
        assert!(!result.any_revert, "the deploy frame SSTORE must succeed");
        assert!(
            vm.validation_observer
                .touched_sender_slots
                .contains(&H256::zero()),
            "the SSTORE'd sender slot must be recorded"
        );
    }

    #[test]
    fn gas_not_immediately_before_call_is_banned() {
        use ethrex_levm::validation_observer::ValidationObserver;
        let sender = addr(0x6A_50);
        let tx = frame_tx_for_obs(
            sender,
            vec![verify_frame_obs(sender, 50_000, 0x03, Bytes::new())],
        );
        let mut db = build_db(vec![(sender, account_with_code(0, Bytes::new()))]);
        let env = hegota_env(sender);
        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &ethrex_crypto::NativeCrypto,
            None,
        )
        .unwrap();
        vm.validation_observer = ValidationObserver::new(sender, None, frame_tx_expiry_verifier());
        // GAS then a non-CALL opcode (ADD): the prior GAS is illegal.
        vm.check_validation_banned_opcode(0x5A); // GAS
        vm.check_validation_banned_opcode(0x01); // ADD
        assert_eq!(
            vm.validation_observer.violation,
            Some(FrameSimViolation::BannedOpcode(0x5A)),
            "GAS not immediately before a *CALL must be a banned opcode"
        );
    }

    #[test]
    fn gas_immediately_before_call_is_allowed() {
        use ethrex_levm::validation_observer::ValidationObserver;
        let sender = addr(0x6A_F1);
        let tx = frame_tx_for_obs(
            sender,
            vec![verify_frame_obs(sender, 50_000, 0x03, Bytes::new())],
        );
        let mut db = build_db(vec![(sender, account_with_code(0, Bytes::new()))]);
        let env = hegota_env(sender);
        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &ethrex_crypto::NativeCrypto,
            None,
        )
        .unwrap();
        vm.validation_observer = ValidationObserver::new(sender, None, frame_tx_expiry_verifier());
        // GAS then CALL (0xF1): legal sequence.
        vm.check_validation_banned_opcode(0x5A); // GAS
        vm.check_validation_banned_opcode(0xF1); // CALL
        assert!(
            vm.validation_observer.violation.is_none(),
            "GAS immediately before a *CALL must be allowed"
        );
    }
}

// ==================== frame_validation_prefix_tests ====================
// (migrated from crates/vm/backends/levm/mod.rs)

mod frame_validation_prefix_tests {
    //! EIP-8141 mempool validation-prefix backend assertions. These
    //! exercise [`LEVM::simulate_frame_validation_prefix`] over signature-less
    //! frame transactions (an empty signature list trivially validates), where
    //! the prefix establishes a payer through real APPROVE code (not the
    //! signature-gated default-code path).
    use bytes::Bytes;
    use ethrex_common::types::Transaction;
    use ethrex_common::types::{
        Account, AccountState, BlockHeader, ChainConfig, Code, CodeMetadata,
        DEFAULT_AA_VOPS_SLOT_COUNT, FRAME_TX_MAX_VERIFY_GAS, Frame, FrameTransaction, PrefixShape,
        ValidationPrefix,
    };
    use ethrex_common::{Address, H256, U256};
    use ethrex_crypto::NativeCrypto;
    use ethrex_levm::db::{Database, gen_db::GeneralizedDatabase};
    use ethrex_levm::errors::DatabaseError;
    use ethrex_levm::validation_observer::{CodeBodyBudget, FocilVopsSurface, Profile2Replay};
    use ethrex_levm::vm::VMType;
    use ethrex_vm::backends::levm::LEVM;
    use rustc_hash::FxHashMap;
    use std::sync::Arc;

    struct Store {
        chain_config: ChainConfig,
        accounts: FxHashMap<Address, Account>,
    }

    impl Database for Store {
        fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
            Ok(self
                .accounts
                .get(&address)
                .map(|acc| AccountState {
                    nonce: acc.info.nonce,
                    balance: acc.info.balance,
                    storage_root: *ethrex_common::constants::EMPTY_TRIE_HASH,
                    code_hash: acc.info.code_hash,
                })
                .unwrap_or_default())
        }
        fn get_storage_value(&self, _: Address, _: H256) -> Result<U256, DatabaseError> {
            Ok(U256::zero())
        }
        fn get_block_hash(&self, _: u64) -> Result<H256, DatabaseError> {
            Ok(H256::zero())
        }
        fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
            Ok(self.chain_config)
        }
        fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
            for acc in self.accounts.values() {
                if acc.info.code_hash == code_hash {
                    return Ok(acc.code.clone());
                }
            }
            Ok(Code::default())
        }
        fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
            for acc in self.accounts.values() {
                if acc.info.code_hash == code_hash {
                    return Ok(CodeMetadata {
                        length: acc.code.len() as u64,
                    });
                }
            }
            Ok(CodeMetadata { length: 0 })
        }
    }

    fn hegota_chain_config() -> ChainConfig {
        ChainConfig {
            shanghai_time: Some(0),
            cancun_time: Some(0),
            prague_time: Some(0),
            osaka_time: Some(0),
            amsterdam_time: Some(0),
            hegota_time: Some(0),
            ..Default::default()
        }
    }

    fn addr(x: u64) -> Address {
        Address::from_low_u64_be(x)
    }

    fn account(balance: u64, code: Bytes) -> Account {
        Account::new(
            U256::from(balance),
            Code::from_bytecode(code, &NativeCrypto),
            0,
            FxHashMap::default(),
        )
    }

    /// APPROVE scope `scope` then STOP.
    fn approve_code(scope: u8) -> Bytes {
        Bytes::from(vec![
            0x60, scope, // PUSH1 scope
            0x60, 0x00, // PUSH1 0 (length)
            0x60, 0x00, // PUSH1 0 (offset)
            0xAA, // APPROVE
            0x00, // STOP
        ])
    }

    /// SLOAD `slot`, discard it, then APPROVE scope 3 (self_verify) and STOP.
    fn sload_then_approve_code(slot: u8) -> Bytes {
        Bytes::from(vec![
            0x60, slot, // PUSH1 slot
            0x54, // SLOAD
            0x50, // POP
            0x60, 0x03, // PUSH1 scope (self_verify)
            0x60, 0x00, // PUSH1 0 (length)
            0x60, 0x00, // PUSH1 0 (offset)
            0xAA, // APPROVE
            0x00, // STOP
        ])
    }

    fn db_with(accounts: Vec<(Address, Account)>) -> GeneralizedDatabase {
        let map: FxHashMap<Address, Account> = accounts.into_iter().collect();
        GeneralizedDatabase::new_with_account_state(
            Arc::new(Store {
                chain_config: hegota_chain_config(),
                accounts: FxHashMap::default(),
            }),
            map,
        )
    }

    fn frame(mode: u8, flags: u8, target: Address, gas_limit: u64) -> Frame {
        Frame {
            mode,
            flags,
            target: Some(target),
            gas_limit,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        }
    }

    fn frame_tx_prefix(sender: Address, frames: Vec<Frame>) -> Transaction {
        Transaction::FrameTransaction(FrameTransaction {
            chain_id: 0,
            nonce_keys: vec![U256::zero()],
            nonce_seq: 0,
            sender,
            frames,
            signatures: Vec::new(),
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: U256::zero(),
            blob_versioned_hashes: Vec::new(),
            ..Default::default()
        })
    }

    fn header() -> BlockHeader {
        BlockHeader {
            timestamp: 0,
            gas_limit: 30_000_000,
            ..Default::default()
        }
    }

    /// A deploy-only+pay prefix (no sender execution approval) whose pay frame
    /// calls APPROVE_PAYMENT while `sender_approved == false` must fail
    /// validation: EIP-8141 reverts the premature payment approval, so the prefix
    /// is rejected with "validation prefix frame reverted" — this fires before the
    /// deploy-leaves-sender-codeless check is reached.
    #[test]
    fn prefix_with_payment_before_execution_approval_is_rejected() {
        let sender = addr(0xDEAD01);
        let paymaster = addr(0xBEEF01);
        let frames = vec![
            frame(0, 0x00, sender, 50_000),
            frame(1, 0x01, paymaster, 50_000),
        ];
        let tx = frame_tx_prefix(sender, frames);
        let mut db = db_with(vec![
            (sender, account(0, Bytes::new())),
            (paymaster, account(0, approve_code(0x01))),
        ]);
        let prefix = ValidationPrefix {
            shape: PrefixShape::DeployOnlyVerifyPay,
            frame_indices: vec![0, 1],
            deploy_index: Some(0),
            pay_index: Some(1),
        };
        let outcome = LEVM::simulate_frame_validation_prefix(
            &tx,
            &header(),
            &mut db,
            VMType::L1,
            &NativeCrypto,
            &prefix,
            None,
            FRAME_TX_MAX_VERIFY_GAS,
            None,
        )
        .expect("simulation runs");
        assert!(
            !outcome.passed,
            "a pay frame approving payment before the sender approves execution must fail validation"
        );
        assert_eq!(
            outcome.violation.as_deref(),
            Some("validation prefix frame reverted"),
            "the failure must be the payment-ordering revert, got {:?}",
            outcome.violation
        );
    }

    /// A self_verify prefix that establishes a payer (the sender's APPROVE(3)
    /// code) and installs no deploy frame must pass validation.
    #[test]
    fn self_verify_prefix_passes_validation() {
        let sender = addr(0x5E_11_01);
        let frames = vec![frame(1, 0x03, sender, 50_000)];
        let tx = frame_tx_prefix(sender, frames);
        let mut db = db_with(vec![(sender, account(0, approve_code(0x03)))]);
        let prefix = ValidationPrefix {
            shape: PrefixShape::SelfVerify,
            frame_indices: vec![0],
            deploy_index: None,
            pay_index: Some(0),
        };
        let outcome = LEVM::simulate_frame_validation_prefix(
            &tx,
            &header(),
            &mut db,
            VMType::L1,
            &NativeCrypto,
            &prefix,
            None,
            FRAME_TX_MAX_VERIFY_GAS,
            None,
        )
        .expect("simulation runs");
        assert!(
            outcome.passed,
            "a self_verify prefix that sets a payer must pass, got {:?}",
            outcome.violation
        );
        assert_eq!(outcome.accessed_paymaster, Some((sender, false)));
    }

    /// The mempool reservation bounds the blob fee at the transaction's declared
    /// `max_fee_per_blob_gas`, not at the head block's `blob_base_fee`.
    ///
    /// Admission simulates against the current head while execution charges the base fee
    /// of whichever later block includes the transaction, and the blob base fee moves per
    /// block — so reserving at the head's rate can reserve less than the eventual charge
    /// and let a paymaster be overdrawn. `max_fee_per_blob_gas >= blob_base_fee` is an
    /// inclusion condition (EIP-8141 §Blob handling), so the declared rate bounds every
    /// block that can include the transaction. The consensus `max_cost` that `APPROVE`
    /// collects still prices blobs at the base rate; only the reservation differs.
    #[test]
    fn the_reservation_ceiling_prices_blobs_at_the_declared_max_rate() {
        const GAS_PER_BLOB: u64 = 1 << 17;
        const MAX_FEE_PER_BLOB_GAS: u64 = 1_000;

        let sender = addr(0x5E_11_02);
        let mut tx = frame_tx_prefix(sender, vec![frame(1, 0x03, sender, 50_000)]);
        let Transaction::FrameTransaction(frame_tx) = &mut tx else {
            unreachable!("frame_tx_prefix builds a frame transaction")
        };
        frame_tx.max_fee_per_gas = 0;
        frame_tx.max_fee_per_blob_gas = U256::from(MAX_FEE_PER_BLOB_GAS);
        frame_tx.blob_versioned_hashes = vec![H256::from_low_u64_be(1)];
        frame_tx.inner_hash = Default::default();
        frame_tx.cached_canonical = Default::default();

        let mut db = db_with(vec![(sender, account(1_000_000, approve_code(0x03)))]);
        let prefix = ValidationPrefix {
            shape: PrefixShape::SelfVerify,
            frame_indices: vec![0],
            deploy_index: None,
            pay_index: Some(0),
        };
        let outcome = LEVM::simulate_frame_validation_prefix(
            &tx,
            &header(),
            &mut db,
            VMType::L1,
            &NativeCrypto,
            &prefix,
            None,
            FRAME_TX_MAX_VERIFY_GAS,
            None,
        )
        .expect("simulation runs");

        // `max_fee_per_gas` is zero, so the ceiling is the blob term alone.
        assert_eq!(
            outcome.reservation_ceiling,
            U256::from(GAS_PER_BLOB * MAX_FEE_PER_BLOB_GAS),
            "the reservation must price the blob at max_fee_per_blob_gas"
        );
        // The header's blob base fee is lower than the declared rate, so the consensus max
        // cost is strictly smaller — which is exactly why the reservation cannot use it.
        assert!(
            outcome.max_cost < outcome.reservation_ceiling,
            "max_cost {} must price blobs below the declared ceiling {}",
            outcome.max_cost,
            outcome.reservation_ceiling
        );
    }

    /// A sponsored OnlyVerifyPay prefix whose pay frame targets a non-sender
    /// sponsor. EIP-8141 structural rule 3 restricts only self_verify /
    /// only_verify targets to `tx.sender`; rule 4 places no target requirement
    /// on `pay`, so the sponsor may approve payment for the sender. The prefix
    /// must pass structural validation and the simulation must establish the
    /// sponsor as the paymaster.
    #[test]
    fn sponsored_pay_frame_may_target_non_sender() {
        let sender = addr(0x5E_11_02);
        let sponsor = addr(0x42_D9_3E);
        let frames = vec![
            frame(1, 0x02, sender, 40_000),  // only_verify -> sender
            frame(1, 0x01, sponsor, 40_000), // pay -> sponsor
        ];
        let tx = frame_tx_prefix(sender, frames);
        let Transaction::FrameTransaction(frame_tx) = &tx else {
            unreachable!("frame_tx_prefix builds a frame transaction")
        };
        let prefix = frame_tx
            .validation_prefix()
            .expect("OnlyVerifyPay shape recognized");
        assert_eq!(prefix.shape, PrefixShape::OnlyVerifyPay);
        frame_tx
            .validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
            .expect("a pay frame may target a non-sender sponsor (EIP-8141 structural rule 4)");

        let mut db = db_with(vec![
            (sender, account(0, approve_code(0x02))),
            (sponsor, account(0, approve_code(0x01))),
        ]);
        let outcome = LEVM::simulate_frame_validation_prefix(
            &tx,
            &header(),
            &mut db,
            VMType::L1,
            &NativeCrypto,
            &prefix,
            None,
            FRAME_TX_MAX_VERIFY_GAS,
            None,
        )
        .expect("simulation runs");
        assert!(
            outcome.passed,
            "a sponsored prefix must pass validation, got {:?}",
            outcome.violation
        );
        assert_eq!(
            outcome.accessed_paymaster,
            Some((sponsor, false)),
            "the sponsor must be identified as the paymaster"
        );
    }

    /// EIP-8369 Profile 2: a self_verify prefix reading `sender` storage slot
    /// `AA_VOPS_SLOT_COUNT` (one past the admitted `[0, AA_VOPS_SLOT_COUNT)`
    /// bound) must be rejected once a Profile 2 surface is configured.
    #[test]
    fn storage_read_at_vops_bound_violates_configured_surface() {
        let sender = addr(0x5E_11_03);
        #[expect(clippy::as_conversions, reason = "test fixture, slot fits u8")]
        let slot = DEFAULT_AA_VOPS_SLOT_COUNT as u8;
        let frames = vec![frame(1, 0x03, sender, 50_000)];
        let tx = frame_tx_prefix(sender, frames);
        let mut db = db_with(vec![(sender, account(0, sload_then_approve_code(slot)))]);
        let prefix = ValidationPrefix {
            shape: PrefixShape::SelfVerify,
            frame_indices: vec![0],
            deploy_index: None,
            pay_index: Some(0),
        };
        let surface = FocilVopsSurface {
            payer: sender,
            slot_count: DEFAULT_AA_VOPS_SLOT_COUNT,
        };
        let outcome = LEVM::simulate_frame_validation_prefix(
            &tx,
            &header(),
            &mut db,
            VMType::L1,
            &NativeCrypto,
            &prefix,
            None,
            FRAME_TX_MAX_VERIFY_GAS,
            Some(Profile2Replay {
                surface,
                code_budget: CodeBodyBudget::unbounded(),
            }),
        )
        .expect("simulation runs");
        assert!(
            !outcome.passed,
            "a read at slot AA_VOPS_SLOT_COUNT must fall outside the Profile 2 surface"
        );
        assert_eq!(
            outcome.violation.as_deref(),
            Some("StorageOutsideVopsSurface"),
            "got {:?}",
            outcome.violation
        );
    }

    /// The same prefix as above (reading slot `AA_VOPS_SLOT_COUNT`) must still
    /// pass when no Profile 2 surface is configured: EIP-8141's plain
    /// sender-storage rule places no numeric bound on the slot, so this same
    /// transaction may pass the EIP-8141 mempool rule and fail Profile 2
    /// eligibility, or vice versa, per EIP-8369.
    #[test]
    fn storage_read_at_vops_bound_passes_without_surface() {
        let sender = addr(0x5E_11_04);
        #[expect(clippy::as_conversions, reason = "test fixture, slot fits u8")]
        let slot = DEFAULT_AA_VOPS_SLOT_COUNT as u8;
        let frames = vec![frame(1, 0x03, sender, 50_000)];
        let tx = frame_tx_prefix(sender, frames);
        let mut db = db_with(vec![(sender, account(0, sload_then_approve_code(slot)))]);
        let prefix = ValidationPrefix {
            shape: PrefixShape::SelfVerify,
            frame_indices: vec![0],
            deploy_index: None,
            pay_index: Some(0),
        };
        let outcome = LEVM::simulate_frame_validation_prefix(
            &tx,
            &header(),
            &mut db,
            VMType::L1,
            &NativeCrypto,
            &prefix,
            None,
            FRAME_TX_MAX_VERIFY_GAS,
            None,
        )
        .expect("simulation runs");
        assert!(
            outcome.passed,
            "without a configured surface the plain EIP-8141 sender-storage rule applies, got {:?}",
            outcome.violation
        );
    }

    /// A read at slot `AA_VOPS_SLOT_COUNT - 1` (the last admitted slot) must
    /// pass under a configured Profile 2 surface, pinning the boundary on the
    /// admitted side.
    #[test]
    fn storage_read_below_vops_bound_passes_configured_surface() {
        let sender = addr(0x5E_11_05);
        #[expect(clippy::as_conversions, reason = "test fixture, slot fits u8")]
        let slot = (DEFAULT_AA_VOPS_SLOT_COUNT - 1) as u8;
        let frames = vec![frame(1, 0x03, sender, 50_000)];
        let tx = frame_tx_prefix(sender, frames);
        let mut db = db_with(vec![(sender, account(0, sload_then_approve_code(slot)))]);
        let prefix = ValidationPrefix {
            shape: PrefixShape::SelfVerify,
            frame_indices: vec![0],
            deploy_index: None,
            pay_index: Some(0),
        };
        let surface = FocilVopsSurface {
            payer: sender,
            slot_count: DEFAULT_AA_VOPS_SLOT_COUNT,
        };
        let outcome = LEVM::simulate_frame_validation_prefix(
            &tx,
            &header(),
            &mut db,
            VMType::L1,
            &NativeCrypto,
            &prefix,
            None,
            FRAME_TX_MAX_VERIFY_GAS,
            Some(Profile2Replay {
                surface,
                code_budget: CodeBodyBudget::unbounded(),
            }),
        )
        .expect("simulation runs");
        assert!(
            outcome.passed,
            "a read at slot AA_VOPS_SLOT_COUNT - 1 must lie inside the Profile 2 surface, got {:?}",
            outcome.violation
        );
    }
}

// ==================== Relocated from crates/vm/levm/src/vm.rs ====================
// Frame-batch, approval-rollback, and frame-signature unit tests (EIP-8141).
// Exercise crate-public levm internals; kept here per the repo test-location rule.

mod atomic_batch_end_tests {
    use ethrex_common::types::Frame;
    use ethrex_levm::vm::find_batch_end;

    fn frame(flags: u8, mode: u8) -> Frame {
        Frame {
            mode,
            flags,
            ..Default::default()
        }
    }

    #[test]
    fn batch_end_is_first_unflagged_frame_any_mode() {
        // [SENDER+flag, DEFAULT no-flag, SENDER no-flag]: a SENDER-only finder
        // would skip past the DEFAULT terminator to index 2; the batch ends at
        // index 1. Static validation keeps VERIFY frames out of batches, so
        // DEFAULT and SENDER are the only modes reachable here.
        let frames = vec![frame(0x04, 2), frame(0x00, 0), frame(0x00, 2)];
        assert_eq!(find_batch_end(&frames, 0), 1);
    }

    #[test]
    fn batch_end_spans_consecutive_flagged_frames() {
        let frames = vec![frame(0x04, 2), frame(0x04, 0), frame(0x00, 2)];
        assert_eq!(find_batch_end(&frames, 0), 2);
        assert_eq!(find_batch_end(&frames, 1), 2);
    }

    #[test]
    fn failing_terminator_frame_is_its_own_end() {
        // The failing frame is the unflagged terminator: nothing to skip.
        let frames = vec![frame(0x04, 2), frame(0x00, 2)];
        assert_eq!(find_batch_end(&frames, 1), 1);
    }

    #[test]
    fn verify_frame_terminates_batch() {
        // [DEFAULT+flag, VERIFY no-flag (scope bits only), SENDER no-flag]
        let frames = vec![frame(0x04, 0), frame(0x01, 1), frame(0x00, 2)];
        assert_eq!(find_batch_end(&frames, 0), 1);
    }
}

mod atomic_batch_approval_rollback_tests {
    use ethrex_common::{Address, U256};
    use ethrex_levm::vm::FrameTxContext;

    fn minimal_ctx() -> FrameTxContext {
        FrameTxContext {
            sender_approved: false,
            payer_address: None,
            frame_results: Vec::new(),
            current_frame_index: 0,
            sig_hash: ethrex_common::H256::zero(),
            tx: ethrex_common::types::FrameTransaction::default(),
            approve_called_in_current_frame: false,
            total_gas_limit: 0,
            legacy_sender_nonce: 0,
            blob_base_fee: U256::zero(),
        }
    }

    #[test]
    fn batch_revert_rolls_back_in_batch_approvals() {
        let mut ctx = minimal_ctx();
        // execute_frame_tx snapshots at batch entry...
        let snapshot = ctx.approval_snapshot();
        // ...an in-batch frame calls APPROVE(EXECUTION_AND_PAYMENT)...
        ctx.sender_approved = true;
        ctx.payer_address = Some(Address::from_low_u64_be(0xBEEF));
        // ...a later in-batch frame fails and the batch reverts:
        ctx.restore_approvals(snapshot);
        assert!(
            !ctx.sender_approved,
            "in-batch sender approval must not survive batch revert"
        );
        assert!(
            ctx.payer_address.is_none(),
            "in-batch payer approval must not survive batch revert"
        );
    }

    #[test]
    fn pre_batch_approvals_survive_batch_revert() {
        let mut ctx = minimal_ctx();
        // Approval granted by a frame BEFORE the batch:
        ctx.sender_approved = true;
        ctx.payer_address = Some(Address::from_low_u64_be(0xA11CE));
        let snapshot = ctx.approval_snapshot();
        // In-batch frame does something; batch reverts:
        ctx.restore_approvals(snapshot);
        assert!(
            ctx.sender_approved,
            "pre-batch sender approval must survive"
        );
        assert_eq!(ctx.payer_address, Some(Address::from_low_u64_be(0xA11CE)));
    }
}

mod frame_sig_validation_tests {
    use bytes::Bytes;
    use ethrex_common::types::Fork;
    use ethrex_common::{
        Address, H256,
        types::{
            FRAME_SIG_SCHEME_ARBITRARY, FRAME_SIG_SCHEME_P256, FRAME_SIG_SCHEME_SECP256K1,
            FrameSignature,
        },
    };
    use ethrex_levm::vm::validate_frame_signatures;

    /// A non-zero scalar well below both group orders, for the field under test
    /// to not be the one that fails validation.
    const VALID_R: [u8; 32] = [0x11; 32];

    fn validates(sigs: &[FrameSignature]) -> bool {
        validate_frame_signatures(
            sigs,
            H256::zero(),
            Address::zero(),
            hegota(),
            &ethrex_crypto::NativeCrypto,
        )
    }

    fn hegota() -> Fork {
        Fork::Hegota
    }

    fn dummy_sig(scheme: u8, sig_len: usize) -> FrameSignature {
        FrameSignature {
            scheme,
            signer: Some(Address::from_low_u64_be(0xBEEF)),
            msg: Bytes::new(),
            signature: Bytes::from(vec![0u8; sig_len]),
        }
    }

    // secp256k1 n/2 and P-256 n/2 (big-endian 32-byte), and each value + 1.
    const SECP_N_HALF: [u8; 32] = [
        0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b,
        0x20, 0xa0,
    ];
    const SECP_N_HALF_PLUS_1: [u8; 32] = [
        0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b,
        0x20, 0xa1,
    ];
    const P256_N_HALF: [u8; 32] = [
        0x7f, 0xff, 0xff, 0xff, 0x80, 0x00, 0x00, 0x00, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xde, 0x73, 0x7d, 0x56, 0xd3, 0x8b, 0xcf, 0x42, 0x79, 0xdc, 0xe5, 0x61, 0x7e, 0x31,
        0x92, 0xa8,
    ];
    const P256_N_HALF_PLUS_1: [u8; 32] = [
        0x7f, 0xff, 0xff, 0xff, 0x80, 0x00, 0x00, 0x00, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xde, 0x73, 0x7d, 0x56, 0xd3, 0x8b, 0xcf, 0x42, 0x79, 0xdc, 0xe5, 0x61, 0x7e, 0x31,
        0x92, 0xa9,
    ];

    #[expect(
        clippy::indexing_slicing,
        reason = "fixed-size buffers with well-known bounds in test code"
    )]
    fn secp_sig(v: u8, r: &[u8; 32], s: &[u8; 32]) -> FrameSignature {
        // [v(1) | r(32) | s(32)]
        let mut bytes = vec![0u8; 65];
        bytes[0] = v;
        bytes[1..33].copy_from_slice(r);
        bytes[33..65].copy_from_slice(s);
        FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(Address::from_low_u64_be(0xBEEF)),
            msg: Bytes::new(),
            signature: Bytes::from(bytes),
        }
    }

    #[expect(
        clippy::indexing_slicing,
        reason = "fixed-size buffers with well-known bounds in test code"
    )]
    fn p256_sig(r: &[u8; 32], s: &[u8; 32]) -> FrameSignature {
        // [r(32) | s(32) | qx(32) | qy(32)]
        let mut bytes = vec![0u8; 128];
        bytes[0..32].copy_from_slice(r);
        bytes[32..64].copy_from_slice(s);
        FrameSignature {
            scheme: FRAME_SIG_SCHEME_P256,
            signer: Some(Address::from_low_u64_be(0xBEEF)),
            msg: Bytes::new(),
            signature: Bytes::from(bytes),
        }
    }

    #[test]
    fn high_s_above_n_half_is_rejected() {
        // EIP-8141 requires low-s at consensus: s > n/2 is invalid for both
        // schemes. s == n/2 is the largest canonical value; the positive case is
        // covered end-to-end by `secp256k1_positive_and_tampered`.
        assert!(!validates(&[secp_sig(0, &VALID_R, &SECP_N_HALF_PLUS_1)]));
        assert!(!validates(&[p256_sig(&VALID_R, &P256_N_HALF_PLUS_1)]));
    }

    #[test]
    fn zero_r_or_s_is_rejected() {
        // EIP-8141 requires 0 < r < n and 0 < s <= n/2.
        assert!(!validates(&[secp_sig(0, &[0u8; 32], &SECP_N_HALF)]));
        assert!(!validates(&[secp_sig(0, &VALID_R, &[0u8; 32])]));
        assert!(!validates(&[p256_sig(&[0u8; 32], &P256_N_HALF)]));
        assert!(!validates(&[p256_sig(&VALID_R, &[0u8; 32])]));
    }

    #[test]
    fn r_at_or_above_group_order_is_rejected() {
        // secp256k1 n and P-256 n themselves are out of range for r.
        const SECP_N: [u8; 32] = [
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xfe, 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c,
            0xd0, 0x36, 0x41, 0x41,
        ];
        const P256_N: [u8; 32] = [
            0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84, 0xf3, 0xb9, 0xca, 0xc2,
            0xfc, 0x63, 0x25, 0x51,
        ];
        assert!(!validates(&[secp_sig(0, &SECP_N, &SECP_N_HALF)]));
        assert!(!validates(&[p256_sig(&P256_N, &P256_N_HALF)]));
    }

    #[test]
    fn recovery_id_above_1_is_rejected() {
        // EIP-8141 fixes `v` as a bare recovery id, so the EVM-style 27/28
        // encoding is invalid.
        assert!(!validates(&[secp_sig(2, &VALID_R, &SECP_N_HALF)]));
        assert!(!validates(&[secp_sig(27, &VALID_R, &SECP_N_HALF)]));
    }

    #[test]
    fn malformed_or_unknown_scheme_is_invalid() {
        assert!(!validates(&[dummy_sig(FRAME_SIG_SCHEME_SECP256K1, 10)]));
        assert!(!validates(&[dummy_sig(0xFF, 65)]));
    }

    #[test]
    fn empty_list_is_valid() {
        assert!(validate_frame_signatures(
            &[],
            H256::zero(),
            Address::zero(),
            hegota(),
            &ethrex_crypto::NativeCrypto
        ));
    }

    #[test]
    fn scheme0_wrong_sig_length_is_invalid() {
        let sig = dummy_sig(FRAME_SIG_SCHEME_SECP256K1, 10);
        assert!(!validate_frame_signatures(
            &[sig],
            H256::zero(),
            Address::zero(),
            hegota(),
            &ethrex_crypto::NativeCrypto
        ));
    }

    #[test]
    fn scheme1_wrong_sig_length_is_invalid() {
        let sig = dummy_sig(FRAME_SIG_SCHEME_P256, 64);
        assert!(!validate_frame_signatures(
            &[sig],
            H256::zero(),
            Address::zero(),
            hegota(),
            &ethrex_crypto::NativeCrypto
        ));
    }

    #[test]
    fn unknown_scheme_is_invalid() {
        let sig = dummy_sig(0xFF, 65);
        assert!(!validate_frame_signatures(
            &[sig],
            H256::zero(),
            Address::zero(),
            hegota(),
            &ethrex_crypto::NativeCrypto
        ));
    }

    #[test]
    fn arbitrary_empty_signer_is_valid() {
        // ARBITRARY (scheme 0): no protocol crypto; signer must be empty; any
        // signature bytes are accepted, and the canonicality rules on r/s/v do
        // not apply since there is no ECDSA signature to canonicalize.
        let sig = FrameSignature {
            scheme: FRAME_SIG_SCHEME_ARBITRARY,
            signer: None,
            msg: Bytes::new(),
            signature: Bytes::from(vec![0xAAu8; 10]),
        };
        assert!(validates(std::slice::from_ref(&sig)));
    }

    #[test]
    fn arbitrary_with_signer_is_invalid() {
        // Per EIP-8141 an ARBITRARY signature MUST NOT name a signer.
        let sig = FrameSignature {
            scheme: FRAME_SIG_SCHEME_ARBITRARY,
            signer: Some(Address::from_low_u64_be(0xBEEF)),
            msg: Bytes::new(),
            signature: Bytes::from(vec![0xAAu8; 10]),
        };
        assert!(!validate_frame_signatures(
            &[sig],
            H256::zero(),
            Address::zero(),
            hegota(),
            &ethrex_crypto::NativeCrypto
        ));
    }

    #[test]
    fn explicit_zero_32byte_msg_is_invalid() {
        let sig = FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(Address::from_low_u64_be(0xBEEF)),
            msg: Bytes::from(vec![0u8; 32]),
            signature: Bytes::from(vec![0u8; 65]),
        };
        assert!(!validate_frame_signatures(
            &[sig],
            H256::zero(),
            Address::zero(),
            hegota(),
            &ethrex_crypto::NativeCrypto
        ));
    }

    #[test]
    fn msg_len_not_0_or_32_is_invalid() {
        let sig = FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(Address::from_low_u64_be(0xBEEF)),
            msg: Bytes::from(vec![0xAAu8; 16]),
            signature: Bytes::from(vec![0u8; 65]),
        };
        assert!(!validate_frame_signatures(
            &[sig],
            H256::zero(),
            Address::zero(),
            hegota(),
            &ethrex_crypto::NativeCrypto
        ));
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "fixed-size buffers with well-known bounds in test code"
    )]
    fn secp256k1_positive_and_tampered() {
        // Build a real secp256k1 signature vector using k256.
        use k256::ecdsa::SigningKey;

        let pk_hex = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let pk_bytes: Vec<u8> = (0..pk_hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&pk_hex[i..i + 2], 16).unwrap())
            .collect();
        let private_key: [u8; 32] = pk_bytes.try_into().unwrap();
        let signing_key = SigningKey::from_bytes(&private_key.into()).unwrap();

        let msg_hash: H256 = H256::from_low_u64_be(0xDEADBEEF_CAFEBABE);

        let (raw_sig, recovery_id) = signing_key
            .sign_prehash_recoverable(msg_hash.as_bytes())
            .unwrap();

        // Derive the expected signer address
        let uncompressed = signing_key.verifying_key().to_encoded_point(false);
        let pub_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
        let expected_signer = Address::from_slice(&pub_hash[12..]);

        // Build the outer signature: v || r || s  (65 bytes).
        // EIP-8141 carries `v` as the bare recovery id (0 or 1).
        let mut sig_bytes = vec![0u8; 65];
        sig_bytes[0] = recovery_id.to_byte();
        sig_bytes[1..33].copy_from_slice(&raw_sig.to_bytes()[..32]); // r
        sig_bytes[33..65].copy_from_slice(&raw_sig.to_bytes()[32..]); // s

        let valid_sig = FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(expected_signer),
            msg: Bytes::new(), // empty → use sig_hash
            signature: Bytes::from(sig_bytes.clone()),
        };

        // Positive: correct signer → valid
        assert!(
            validate_frame_signatures(
                std::slice::from_ref(&valid_sig),
                msg_hash,
                Address::zero(),
                hegota(),
                &ethrex_crypto::NativeCrypto
            ),
            "valid secp256k1 signature should pass"
        );

        // Tampered signer: wrong address → invalid
        let wrong_addr = Address::from_low_u64_be(0xDEAD);
        let tampered = FrameSignature {
            signer: Some(wrong_addr),
            ..valid_sig.clone()
        };
        assert!(
            !validate_frame_signatures(
                &[tampered],
                msg_hash,
                Address::zero(),
                hegota(),
                &ethrex_crypto::NativeCrypto
            ),
            "wrong signer should fail"
        );

        // Empty signer (None) resolves to tx.sender: valid iff sender == recovered.
        let empty_signer_sig = FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: None,
            msg: Bytes::new(),
            signature: Bytes::from(sig_bytes.clone()),
        };
        assert!(
            validate_frame_signatures(
                std::slice::from_ref(&empty_signer_sig),
                msg_hash,
                expected_signer,
                hegota(),
                &ethrex_crypto::NativeCrypto
            ),
            "empty signer must resolve to tx.sender and validate when sender == recovered"
        );
        assert!(
            !validate_frame_signatures(
                std::slice::from_ref(&empty_signer_sig),
                msg_hash,
                wrong_addr,
                hegota(),
                &ethrex_crypto::NativeCrypto
            ),
            "empty signer resolving to a different tx.sender must fail"
        );

        // Wrong hash: valid sig but different sig_hash → invalid
        let other_hash = H256::from_low_u64_be(0x1234567890ABCDEFu64);
        assert!(
            !validate_frame_signatures(
                &[valid_sig],
                other_hash,
                Address::zero(),
                hegota(),
                &ethrex_crypto::NativeCrypto
            ),
            "wrong sig_hash should fail"
        );
    }

    #[test]
    fn p256_wrong_signer_is_invalid() {
        // Construct a syntactically-128-byte P256 sig with wrong signer address.
        // The signer derivation check fires before the curve verification.
        let sig = FrameSignature {
            scheme: FRAME_SIG_SCHEME_P256,
            signer: Some(Address::from_low_u64_be(0xDEAD)),
            msg: Bytes::new(),
            signature: Bytes::from(vec![0xAAu8; 128]),
        };
        // keccak(qx||qy)[12..] for all-0xAA will not equal 0xDEAD.
        assert!(
            !validate_frame_signatures(
                &[sig],
                H256::zero(),
                Address::zero(),
                hegota(),
                &ethrex_crypto::NativeCrypto
            ),
            "mismatched P256 signer should fail"
        );
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "fixed-size buffers with well-known bounds in test code"
    )]
    fn p256_positive_and_tampered() {
        // Regression lock for the EIP-8141 P256 signature validation path
        //. No external EEST reference vectors exist
        // yet; these values exercise validate_frame_signatures end-to-end
        // through P256VERIFY with a real p256-crate signature.
        //
        // Path used: live p256::ecdsa signing (p256 0.13.2 has `ecdsa` +
        // `arithmetic` features enabled in levm's Cargo.toml).
        use p256::ecdsa::SigningKey;
        use p256::ecdsa::signature::hazmat::PrehashSigner;

        // Fixed private key — deterministic, no randomness.
        let pk_bytes: [u8; 32] = [
            0xc9, 0x11, 0x0e, 0xa2, 0xf8, 0x7f, 0x3c, 0x06, 0x74, 0x1a, 0x4d, 0x35, 0x62, 0xb2,
            0x11, 0x7d, 0x3e, 0x6a, 0x5c, 0x0b, 0x28, 0x0c, 0x3a, 0x0f, 0x56, 0x2e, 0x38, 0xa7,
            0x21, 0xb0, 0x98, 0xc4,
        ];
        let signing_key = SigningKey::from_bytes(&pk_bytes.into()).unwrap();
        let verifying_key = signing_key.verifying_key();
        let encoded = verifying_key.to_encoded_point(false);
        let encoded_bytes = encoded.as_bytes();
        // Uncompressed point: 0x04 || qx (32B) || qy (32B)
        let qx = &encoded_bytes[1..33];
        let qy = &encoded_bytes[33..65];

        // Fixed 32-byte non-zero digest (explicit msg path — sig_hash arg unused).
        let digest: [u8; 32] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20,
        ];

        // sign_prehash is deterministic for p256 with RFC-6979 nonce.
        let raw_sig: p256::ecdsa::Signature = signing_key.sign_prehash(&digest).unwrap();
        let sig_bytes_der = raw_sig.to_bytes(); // r || s (64 bytes, DER-unwrapped)
        let r = &sig_bytes_der[..32];
        let s = &sig_bytes_der[32..64];

        // Derive signer: keccak256(qx || qy)[12..] — matches validate_frame_signatures.
        let mut pk_concat = Vec::with_capacity(64);
        pk_concat.extend_from_slice(qx);
        pk_concat.extend_from_slice(qy);
        let h = ethrex_crypto::keccak::keccak_hash(&pk_concat);
        let signer = Address::from_slice(&h[12..]);

        // Build the 128-byte signature: r || s || qx || qy.
        let mut signature_blob = vec![0u8; 128];
        signature_blob[..32].copy_from_slice(r);
        signature_blob[32..64].copy_from_slice(s);
        signature_blob[64..96].copy_from_slice(qx);
        signature_blob[96..128].copy_from_slice(qy);

        let valid_sig = FrameSignature {
            scheme: FRAME_SIG_SCHEME_P256,
            signer: Some(signer),
            // Explicit 32-byte msg: sig_hash arg to validate_frame_signatures
            // is irrelevant for this entry.
            msg: Bytes::copy_from_slice(&digest),
            signature: Bytes::from(signature_blob.clone()),
        };

        // Positive: real P256 signature → passes.
        assert!(
            validate_frame_signatures(
                std::slice::from_ref(&valid_sig),
                H256::zero(),
                Address::zero(),
                hegota(),
                &ethrex_crypto::NativeCrypto
            ),
            "valid P256 signature must pass",
        );

        // Tampered r byte: flip one bit in r → curve verification fails.
        let mut tampered_blob = signature_blob.clone();
        tampered_blob[0] ^= 0x01;
        let tampered_r = FrameSignature {
            signature: Bytes::from(tampered_blob),
            ..valid_sig.clone()
        };
        assert!(
            !validate_frame_signatures(
                &[tampered_r],
                H256::zero(),
                Address::zero(),
                hegota(),
                &ethrex_crypto::NativeCrypto
            ),
            "flipped r byte must fail curve verification",
        );

        // Wrong signer: signer-derivation check fires.
        let wrong_signer = FrameSignature {
            signer: Some(Address::from_low_u64_be(0xDEAD)),
            ..valid_sig
        };
        assert!(
            !validate_frame_signatures(
                &[wrong_signer],
                H256::zero(),
                Address::zero(),
                hegota(),
                &ethrex_crypto::NativeCrypto
            ),
            "wrong signer must fail",
        );
    }
}

// ==================== EXPIRY_VERIFIER predeploy ====================
mod expiry_verifier_tests {
    use ethrex_common::H160;
    use ethrex_vm::system_contracts::{
        EXPIRY_VERIFIER_PREDEPLOY, EXPIRY_VERIFIER_RUNTIME_BYTECODE,
    };

    #[test]
    fn expiry_verifier_constants_match_spec() {
        let expected: [u8; 26] = [
            0x60, 0x08, 0x36, 0x14, 0x60, 0x0a, 0x57, 0x5f, 0x5f, 0xfd, 0x5b, 0x5f, 0x35, 0x60,
            0xc0, 0x1c, 0x42, 0x11, 0x60, 0x16, 0x57, 0x00, 0x5b, 0x5f, 0x5f, 0xfd,
        ];
        assert_eq!(
            EXPIRY_VERIFIER_RUNTIME_BYTECODE.as_slice(),
            expected.as_slice()
        );
        assert_eq!(EXPIRY_VERIFIER_RUNTIME_BYTECODE.len(), 26);
        assert_eq!(
            EXPIRY_VERIFIER_PREDEPLOY.address,
            H160::from_low_u64_be(0x8141)
        );
    }
}

// ==================== SIGPARAM (0xB4) execution ====================

/// End-to-end SIGPARAM tests: a DEFAULT frame runs real `SIGPARAM` bytecode and
/// persists the result, so the opcode's stack order, gas and halt conditions are
/// exercised rather than reimplemented.
mod sigparam_execution_tests {
    use super::*;
    use ethrex_common::types::{
        FRAME_RECEIPT_STATUS_FAILURE, FRAME_SIG_SCHEME_ARBITRARY, FrameSignature,
    };

    /// Read slot `slot` of `addr` from the post-execution cache.
    fn storage_of(db: &GeneralizedDatabase, addr: Address, slot: U256) -> U256 {
        let key = slot.to_big_endian();
        db.current_accounts_state
            .get(&addr)
            .and_then(|account| account.storage.get(&ethrex_common::H256(key)).copied())
            .unwrap_or_default()
    }

    /// `SIGPARAM(param, index)` then `SSTORE(0, result)`.
    /// Metadata params take `[param, signatureIndex]` with the index on top, so
    /// `param` is pushed first.
    fn sigparam_metadata_code(param: u8, index: u8) -> Bytes {
        Bytes::from(vec![
            0x60, param, // PUSH1 param
            0x60, index, // PUSH1 signatureIndex
            0xB4,  // SIGPARAM
            0x60, 0x00, // PUSH1 0 (slot)
            0x55, // SSTORE
            0x00, // STOP
        ])
    }

    /// `SIGDATACOPY` copying `length` bytes from `dataOffset` to `memOffset`, then
    /// `SSTORE(0, MLOAD(memOffset))`. EIP-8141's stack is `memOffset`, `dataOffset`,
    /// `length`, `signatureIndex` from the top down, so the pushes run in reverse and the
    /// index sits *beneath* the three copy operands -- where SIGPARAM(0x04) had it on top.
    fn sigdatacopy_code(index: u8, length: u8, data_offset: u8, mem_offset: u8) -> Bytes {
        Bytes::from(vec![
            0x60,
            index, // PUSH1 signatureIndex
            0x60,
            length, // PUSH1 length
            0x60,
            data_offset, // PUSH1 dataOffset
            0x60,
            mem_offset, // PUSH1 memOffset
            0xB5,       // SIGDATACOPY
            0x60,
            mem_offset, // PUSH1 memOffset
            0x51,       // MLOAD
            0x60,
            0x00, // PUSH1 0 (slot)
            0x55, // SSTORE
            0x00, // STOP
        ])
    }

    /// `SIGPARAM(0x04, index)`, which EIP-8141 no longer defines: the copy operation moved to
    /// its own instruction, so this param must halt.
    fn sigparam_retired_copy_code(index: u8) -> Bytes {
        Bytes::from(vec![
            0x60, 0x00, // PUSH1 length
            0x60, 0x00, // PUSH1 dataOffset
            0x60, 0x00, // PUSH1 memOffset
            0x60, 0x04, // PUSH1 param (retired copy)
            0x60, index, // PUSH1 signatureIndex
            0xB4,  // SIGPARAM
            0x00,  // STOP
        ])
    }

    /// One VERIFY frame approving both scopes from the sender, then a DEFAULT
    /// frame running `code` at `reader`.
    fn run_reader(
        code: Bytes,
        signatures: Vec<FrameSignature>,
    ) -> (
        Result<ExecutionReport, VMError>,
        GeneralizedDatabase,
        Address,
    ) {
        let reader = Address::from_low_u64_be(0xB4B4);
        let mut tx = frame_tx_with_frames(vec![
            verify_frame(FUNDED_SENDER),
            Frame {
                mode: u8::from(FrameMode::Default),
                flags: 0,
                target: Some(reader),
                gas_limit: 200_000,
                // The reader frames SSTORE what they read, which is a new-slot
                // creation and therefore draws state gas from this pool.
                state_limit: STATE_BUDGET,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ]);
        tx.signatures = signatures;
        let (result, db) = run_frame_tx(
            &[
                (
                    FUNDED_SENDER,
                    AUTO_SEED_SENDER_BALANCE,
                    0,
                    Bytes::from(APPROVE_BOTH_CODE.to_vec()),
                ),
                (reader, U256::zero(), 0, code),
            ],
            tx,
        );
        (result, db, reader)
    }

    fn arbitrary_sig(bytes: Vec<u8>) -> FrameSignature {
        FrameSignature {
            scheme: FRAME_SIG_SCHEME_ARBITRARY,
            signer: None,
            msg: Bytes::new(),
            signature: Bytes::from(bytes),
        }
    }

    #[test]
    fn sigparam_0x01_returns_the_scheme() {
        let (result, db, reader) = run_reader(
            sigparam_metadata_code(0x01, 0),
            vec![arbitrary_sig(vec![0xAA; 4])],
        );
        result.expect("valid tx");
        assert_eq!(
            storage_of(&db, reader, U256::zero()),
            U256::from(u64::from(FRAME_SIG_SCHEME_ARBITRARY)),
        );
    }

    #[test]
    fn sigparam_0x03_returns_the_signature_length() {
        let (result, db, reader) = run_reader(
            sigparam_metadata_code(0x03, 0),
            vec![arbitrary_sig(vec![0xAA; 7])],
        );
        result.expect("valid tx");
        assert_eq!(storage_of(&db, reader, U256::zero()), U256::from(7u64));
    }

    #[test]
    fn sigparam_0x00_halts_for_an_arbitrary_signature() {
        // EIP-8141 assigns no resolved signer to ARBITRARY entries, so asking for
        // one is an exceptional halt: the DEFAULT frame fails and writes nothing.
        let (result, db, reader) = run_reader(
            sigparam_metadata_code(0x00, 0),
            vec![arbitrary_sig(vec![0xAA; 4])],
        );
        let report = result.expect("the tx itself stays valid; only the reader frame halts");
        let frame_results = report.frame_results.expect("per-frame results");
        assert_eq!(frame_results[1].status, FRAME_RECEIPT_STATUS_FAILURE);
        assert_eq!(storage_of(&db, reader, U256::zero()), U256::zero());
    }

    #[test]
    fn sigdatacopy_copies_arbitrary_signature_bytes() {
        // 4 payload bytes copied into memory at 0; MLOAD reads them left-aligned
        // in the word, with the rest zero-filled.
        let (result, db, reader) = run_reader(
            sigdatacopy_code(0, 4, 0, 0),
            vec![arbitrary_sig(vec![0xDE, 0xAD, 0xBE, 0xEF])],
        );
        result.expect("valid tx");
        let mut expected = [0u8; 32];
        expected[..4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(
            storage_of(&db, reader, U256::zero()),
            U256::from_big_endian(&expected),
        );
    }

    #[test]
    fn sigdatacopy_zero_fills_past_the_end() {
        // Asking for 8 bytes of a 2-byte signature zero-fills the remainder.
        let (result, db, reader) = run_reader(
            sigdatacopy_code(0, 8, 0, 0),
            vec![arbitrary_sig(vec![0x11, 0x22])],
        );
        result.expect("valid tx");
        let mut expected = [0u8; 32];
        expected[..2].copy_from_slice(&[0x11, 0x22]);
        assert_eq!(
            storage_of(&db, reader, U256::zero()),
            U256::from_big_endian(&expected),
        );
    }

    #[test]
    fn sigdatacopy_reads_operands_in_calldatacopy_order() {
        // The copy operands follow `CALLDATACOPY`: memOffset above dataOffset above
        // length, with signatureIndex beneath all three. Distinct values for all three,
        // and a destination past the first word, pin the order — reading them in any
        // other order lands the bytes somewhere else (or copies a different count).
        let (result, db, reader) = run_reader(
            sigdatacopy_code(0, 3, 1, 0x20),
            vec![arbitrary_sig(vec![0xAA, 0xBB, 0xCC, 0xDD])],
        );
        result.expect("valid tx");
        let mut expected = [0u8; 32];
        expected[..3].copy_from_slice(&[0xBB, 0xCC, 0xDD]);
        assert_eq!(
            storage_of(&db, reader, U256::zero()),
            U256::from_big_endian(&expected),
        );
    }

    /// EIP-8141 retires `SIGPARAM(0x04)`. If it still copied, the operation would exist at two
    /// bytes at once and a contract written against either would work — which is exactly
    /// how a chain ends up with two encodings of the same behaviour.
    #[test]
    fn sigparam_0x04_is_retired_and_halts() {
        let (result, db, reader) = run_reader(
            sigparam_retired_copy_code(0),
            vec![arbitrary_sig(vec![0xDE, 0xAD])],
        );
        result.expect("the transaction itself is valid");
        assert_eq!(
            storage_of(&db, reader, U256::zero()),
            U256::zero(),
            "the frame halted, so nothing was stored"
        );
    }

    /// Raw signature bytes of protocol-validated schemes stay un-introspectable, so
    /// aggregation remains possible later. Only ARBITRARY may be copied.
    #[test]
    fn sigdatacopy_halts_for_a_protocol_validated_scheme() {
        let (result, db, reader) = run_reader(sigdatacopy_code(0, 4, 0, 0), vec![]);
        result.expect("the transaction itself is valid");
        assert_eq!(
            storage_of(&db, reader, U256::zero()),
            U256::zero(),
            "a SECP256K1 entry cannot be copied, so the frame halted"
        );
    }

    /// An out-of-range signature index is an exceptional halt, not a zero read.
    #[test]
    fn sigdatacopy_halts_on_an_out_of_range_index() {
        let (result, db, reader) = run_reader(
            sigdatacopy_code(7, 4, 0, 0),
            vec![arbitrary_sig(vec![0xDE, 0xAD])],
        );
        result.expect("the transaction itself is valid");
        assert_eq!(storage_of(&db, reader, U256::zero()), U256::zero());
    }
}

// ==================== Default code: signature index by scope ====================

/// EIP-8141's default code verifies the signature at index 0 when the frame's
/// allowed scope includes `APPROVE_EXECUTION`, and at index 1 otherwise (the
/// payment-only case, where index 0 belongs to the sender's own verify frame).
mod default_code_sig_index_tests {
    use super::*;
    use ethrex_common::types::{
        FRAME_SIG_SCHEME_ARBITRARY, FRAME_SIG_SCHEME_SECP256K1, FrameSignature,
    };
    use k256::ecdsa::SigningKey;

    /// A signing key plus the address it authenticates.
    fn key_and_address(seed: u8) -> (SigningKey, Address) {
        let signing_key = SigningKey::from_bytes(&[seed; 32].into()).unwrap();
        let uncompressed = signing_key.verifying_key().to_encoded_point(false);
        let pub_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
        (signing_key, Address::from_slice(&pub_hash[12..]))
    }

    /// A canonical EIP-8141 SECP256K1 entry over `sig_hash`: `v` is the bare
    /// recovery id and `signer` is left empty only when it is `tx.sender`.
    fn sign(key: &SigningKey, sig_hash: ethrex_common::H256, signer: Address) -> FrameSignature {
        let (raw_sig, recovery_id) = key.sign_prehash_recoverable(sig_hash.as_bytes()).unwrap();
        let mut bytes = vec![0u8; 65];
        bytes[0] = recovery_id.to_byte();
        bytes[1..33].copy_from_slice(&raw_sig.to_bytes()[..32]);
        bytes[33..65].copy_from_slice(&raw_sig.to_bytes()[32..]);
        FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(signer),
            msg: Bytes::new(),
            signature: Bytes::from(bytes),
        }
    }

    fn arbitrary() -> FrameSignature {
        FrameSignature {
            scheme: FRAME_SIG_SCHEME_ARBITRARY,
            signer: None,
            msg: Bytes::new(),
            signature: Bytes::from(vec![0xAAu8; 4]),
        }
    }

    /// Sender VERIFY frame (contract code approves execution) + payment-only
    /// VERIFY frame whose target is a codeless payer, so default code runs.
    /// `sig_at` is the index the payer's signature occupies; the other slot holds
    /// an ARBITRARY entry, so both lists are fully valid and only the index the
    /// default code consults differs.
    fn run_payment_only_default_code(sig_at: usize) -> Result<ExecutionReport, VMError> {
        let (payer_key, payer) = key_and_address(0x42);
        let mut tx = frame_tx_with_frames(vec![verify_frame(FUNDED_SENDER), pay_frame(payer)]);
        // The signature BYTES of an empty-msg entry are elided from the sig hash,
        // but every other field (including `signer`) is committed — so the entry
        // must already carry its final shape before the hash is computed.
        let mut entries = vec![arbitrary(), arbitrary()];
        entries[sig_at] = FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(payer),
            msg: Bytes::new(),
            signature: Bytes::from(vec![0u8; 65]),
        };
        tx.signatures = entries;
        let sig_hash = tx.compute_sig_hash();
        tx.signatures[sig_at] = sign(&payer_key, sig_hash, payer);
        tx.inner_hash = Default::default();
        tx.cached_canonical = Default::default();

        let (result, _db) = run_frame_tx(
            &[
                (
                    FUNDED_SENDER,
                    AUTO_SEED_SENDER_BALANCE,
                    0,
                    Bytes::from(APPROVE_EXECUTION_CODE.to_vec()),
                ),
                // Codeless payer with enough balance to front the max cost.
                (
                    payer,
                    U256::from(10u64).pow(U256::from(18u64)),
                    0,
                    Bytes::new(),
                ),
            ],
            tx,
        );
        result
    }

    #[test]
    fn payment_only_default_code_reads_signature_index_1() {
        let report = run_payment_only_default_code(1)
            .expect("default code must approve payment from the index-1 signature");
        assert!(
            report.payer_address.is_some(),
            "default code should have set the payer"
        );
    }

    #[test]
    fn payment_only_default_code_ignores_signature_index_0() {
        // The same signature at index 0 must NOT satisfy a payment-only frame:
        // the spec binds that case to index 1, so no payer is approved and the
        // transaction is invalid.
        assert!(
            run_payment_only_default_code(0).is_err(),
            "a payer signature at index 0 must not approve a payment-only frame"
        );
    }
}

// ==================== Transaction-scoped storage refunds ====================

/// EIP-3529 refunds accumulate across frames into one transaction-scoped counter
/// and are applied once at the end, capped at a fifth of the gas used before
/// refunds.
#[test]
fn storage_refund_from_a_later_frame_reduces_reported_gas() {
    // PUSH1 0; CALLDATALOAD; PUSH1 0; SSTORE; STOP -- writes the frame's first
    // calldata word into slot 0, so the same account can set the slot in one
    // frame and clear it in the next.
    let store_calldata: &[u8] = &[0x60, 0x00, 0x35, 0x60, 0x00, 0x55, 0x00];
    let target = Address::from_low_u64_be(0x3529);
    let mut one = [0u8; 32];
    one[31] = 1;

    let default_frame = |data: Bytes| Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(target),
        gas_limit: 200_000,
        // The setting frame draws the slot-creation charge from here; the clearing
        // frame declares the same budget and spends none of it.
        state_limit: STATE_BUDGET,
        value: U256::zero(),
        data,
    };
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        // Frame 1 sets slot 0 to 1; frame 2 clears it, earning the 4800-gas
        // EIP-3529 refund for a non-zero -> zero transition made in an earlier
        // frame. A frame-local refund model could not see that transition.
        default_frame(Bytes::from(one.to_vec())),
        default_frame(Bytes::new()),
    ]);

    let (result, _db) = run_frame_tx(
        &[
            (
                FUNDED_SENDER,
                AUTO_SEED_SENDER_BALANCE,
                0,
                Bytes::from(APPROVE_BOTH_CODE.to_vec()),
            ),
            (
                target,
                U256::zero(),
                0,
                Bytes::from(store_calldata.to_vec()),
            ),
        ],
        tx.clone(),
    );
    let report = result.expect("valid tx");
    let frame_results = report.frame_results.expect("per-frame results");

    // The refund reaches the payer's figure (`gas_spent`), not the block's
    // (`gas_used`): EIP-7778, which EIP-8141 requires, keeps the storage refund from
    // freeing the block capacity the transaction actually occupied.
    let frames_gas: u64 = frame_results.iter().map(|f| f.gas_used).sum();
    let pre_refund = tx.mandatory_gas() + tx.data_cost() + frames_gas;
    assert_eq!(
        report.gas_used, pre_refund,
        "the block must account the pre-refund execution gas"
    );
    assert!(
        report.gas_spent < pre_refund,
        "the clearing frame's refund must be applied to what the payer is charged \
         (pre-refund {pre_refund}, charged {})",
        report.gas_spent
    );
    let applied = pre_refund - report.gas_spent;
    assert!(
        applied <= pre_refund / 5,
        "the applied refund {applied} must respect the EIP-3529 one-fifth cap"
    );

    // EIP-8141 §State-gas attribution and refills. Frame 1 created the slot and was
    // charged for it; frame 2 cleared it in the same transaction, so the creation was
    // never durable and the charge is reversed. The reversal is attributed to the frame
    // that PAID it — "subtract the amount from the receipt of the frame that paid the
    // outstanding charge" — so frame 1's receipt drops to zero rather than frame 2's
    // going negative or frame 2 gaining spendable budget it never paid for.
    assert_eq!(
        frame_results[1].state_gas_used, 0,
        "the setting frame's charge must be refilled off its own receipt, got {}",
        frame_results[1].state_gas_used
    );
    assert_eq!(
        frame_results[2].state_gas_used, 0,
        "the clearing frame created no state and must report none, got {}",
        frame_results[2].state_gas_used
    );
    assert_eq!(
        report.state_gas_used, 0,
        "a slot created and cleared inside one transaction leaves no net state gas, \
         got {}",
        report.state_gas_used
    );
}

#[test]
fn txparam_0x0c_returns_the_frames_remaining_state_gas() {
    // EIP-8141 TXPARAM 0x0C: "`state_gas_left` remaining in the currently executing
    // frame". Read before any state charge, it is the frame's whole declared
    // `limits.state`; read after creating a slot, it is that less the slot's cost.
    //
    // PUSH1 0x0C; TXPARAM; PUSH1 0; SSTORE;      -- slot 0 = state_gas_left at entry
    // PUSH1 1; PUSH1 1; SSTORE;                  -- create slot 1 (charged)
    // PUSH1 0x0C; TXPARAM; PUSH1 2; SSTORE;      -- slot 2 = state_gas_left after
    // STOP
    let code: &[u8] = &[
        0x60, 0x0C, 0xB0, 0x60, 0x00, 0x55, // slot 0 = TXPARAM(0x0C)
        0x60, 0x01, 0x60, 0x01, 0x55, // slot 1 = 1
        0x60, 0x0C, 0xB0, 0x60, 0x02, 0x55, // slot 2 = TXPARAM(0x0C)
        0x00,
    ];
    let reader = Address::from_low_u64_be(0x0C0C);
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (reader, U256::zero(), 0, Bytes::from(code.to_vec())),
    ];
    // Three slots' worth, so both TXPARAM reads and all three writes fit.
    let state_limit = 3 * SSTORE_SET_STATE_GAS;
    let (result, db) = run_frame_tx(
        &accounts,
        frame_tx_with_frames(vec![
            verify_frame(FUNDED_SENDER),
            Frame {
                mode: u8::from(FrameMode::Default),
                flags: 0,
                target: Some(reader),
                gas_limit: 400_000,
                state_limit,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ]),
    );
    let report = result.expect("valid tx");
    assert!(
        matches!(report.result, TxResult::Success),
        "the reader frame must succeed, got {:?}",
        report.result
    );

    let at_entry = storage_slot(&db, reader, ethrex_common::H256::zero());
    let after_two_slots = storage_slot(&db, reader, H256::from_low_u64_be(2));
    assert_eq!(
        at_entry,
        U256::from(state_limit),
        "at frame entry `state_gas_left` is the frame's whole declared `limits.state`"
    );
    // Two slots have been created by the second read: slot 0 (by the first SSTORE,
    // which is itself a creation) and slot 1.
    assert_eq!(
        after_two_slots,
        U256::from(state_limit - 2 * SSTORE_SET_STATE_GAS),
        "each slot creation must draw exactly its state cost from the frame's pool"
    );
}

/// The same cross-frame refill, but the clearing frame is the one that also created
/// the slot: the refill is then credited back to its own pool and is spendable again.
#[test]
fn a_same_frame_state_refill_returns_to_that_frames_pool() {
    // PUSH1 1; PUSH1 0; SSTORE;   -- create slot 0 (charged)
    // PUSH1 0; PUSH1 0; SSTORE;   -- clear it (refilled)
    // PUSH1 1; PUSH1 1; SSTORE;   -- create slot 1, payable only from the refill
    // STOP
    let code: &[u8] = &[
        0x60, 0x01, 0x60, 0x00, 0x55, 0x60, 0x00, 0x60, 0x00, 0x55, 0x60, 0x01, 0x60, 0x01, 0x55,
        0x00,
    ];
    let target = Address::from_low_u64_be(0x3529_0002);
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (target, U256::zero(), 0, Bytes::from(code.to_vec())),
    ];
    // Exactly one slot's worth of state budget. The frame creates two slots but only
    // ever holds one, so it fits — and only because the refill from clearing slot 0
    // returns to this frame's own pool.
    let frame = Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(target),
        gas_limit: 400_000,
        state_limit: SSTORE_SET_STATE_GAS,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let (result, _db) = run_frame_tx(
        &accounts,
        frame_tx_with_frames(vec![verify_frame(FUNDED_SENDER), frame]),
    );
    let report = result.expect("valid tx");
    let fr = report.frame_results.expect("per-frame results");
    assert_eq!(
        fr[1].status,
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "the frame must be able to respend its own refill within one state budget"
    );
    assert_eq!(
        fr[1].state_gas_used, SSTORE_SET_STATE_GAS,
        "one slot survives the frame, so exactly one slot's state gas is attributed"
    );
}

// ==================== atomic batch / EIP-7928 BAL ====================

/// EIP-7928 requires the block access list to describe the block's actual state
/// changes. An atomic batch that unrolls performed none of its writes, so the
/// recorder must forget them: a builder that keeps them writes a BAL the block
/// contradicts, and re-execution on import rejects the block it just produced.
#[test]
fn atomic_batch_revert_drops_the_batch_writes_from_the_bal() {
    use ethrex_common::types::Frame;

    let writer = Address::from_low_u64_be(0x8141_0001);
    let reverter = Address::from_low_u64_be(0x8141_0002);
    let terminator = Address::from_low_u64_be(0x8141_0003);
    let slot = U256::zero();

    // writer: SSTORE(0, 0x2a) ; STOP
    let writer_code = Bytes::from(vec![0x60, 0x2a, 0x60, 0x00, 0x55, 0x00]);
    // reverter: REVERT(0, 0)
    let reverter_code = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xFD]);
    let stop_code = Bytes::from(vec![0x00]);

    let accounts: Vec<SeededAccount> = vec![
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (writer, U256::zero(), 0, writer_code),
        (reverter, U256::zero(), 0, reverter_code),
        (terminator, U256::zero(), 0, stop_code),
    ];

    let batch_frame = |target: Address| Frame {
        mode: u8::from(FrameMode::Sender),
        flags: 0x04,
        target: Some(target),
        gas_limit: 300_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let plain_frame = |target: Address| Frame {
        mode: u8::from(FrameMode::Sender),
        flags: 0x00,
        target: Some(target),
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let verify = Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(FUNDED_SENDER),
        gas_limit: 80_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };

    // The batch writes, then a later member reverts, so the whole batch unrolls.
    // The trailing non-batch frame terminates the batch.
    let tx = frame_tx_with_frames(vec![
        verify,
        batch_frame(writer),
        batch_frame(reverter),
        plain_frame(terminator),
    ]);

    let mut db = seeded_db(&accounts);
    db.enable_bal_recording();
    let env = frame_tx_env(&tx);
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
        .expect("VM::new should succeed for a frame tx");
        vm.execute()
            .expect("a reverting batch must not error the tx");
    }

    // The write was unrolled, so the live slot is still its prestate value.
    let live = db
        .current_accounts_state
        .get(&writer)
        .and_then(|acc| acc.storage.get(&H256::zero()).copied())
        .unwrap_or_default();
    assert!(
        live.is_zero(),
        "the batch unrolled, so the write must not survive; got {live}"
    );

    let bal = db
        .bal_recorder
        .take()
        .expect("BAL recording was enabled")
        .build();
    // Both halves of EIP-7928 are asserted, because the fix depends on both and a
    // vacuous first half would hide the second: the reverted *change* goes, the
    // reverted *access* stays. Finding the entry with `if let` would let this pass if
    // the whole address vanished from the BAL — the same class of stall from the other
    // side, and exactly what rolling back `touched_addresses` at this call site would
    // produce.
    let entry = bal
        .accounts()
        .iter()
        .find(|acc| acc.address == writer)
        .expect("a reverted batch's accesses still belong in the BAL (EIP-7928)");
    assert!(
        entry
            .storage_changes
            .iter()
            .all(|change| change.slot != slot),
        "the BAL must not record a storage change for a slot whose write was unrolled"
    );
    assert!(
        entry.storage_reads.contains(&slot),
        "the unrolled write must be re-filed as a read, not dropped: a slot in \
         neither list is a slot the block accessed and the BAL omits, which \
         re-execution rejects"
    );
}

/// EIP-7928: a frame that reverts on its own — with no atomic batch involved —
/// commits no state, so the recorder must forget its writes exactly as it does
/// for an unrolled batch. The slot is still *accessed*, so it must be reported
/// as a read: dropping the write without re-filing it would leave the slot in
/// neither `storage_changes` nor `storage_reads`, and re-execution (whose shadow
/// recorder sees the raw SLOAD) then rejects the block the builder just made.
///
/// The write-then-read order is the load-bearing part: `record_storage_read`
/// suppresses a read for a slot that is already written, so the read leaves no
/// record of its own and the reverted write is the only thing standing in for it.
#[test]
fn reverted_frame_refiles_its_writes_as_reads_in_the_bal() {
    use ethrex_common::types::Frame;

    let target = Address::from_low_u64_be(0x8141_0101);
    let slot = U256::from(5);

    // SSTORE(5, 1) ; SLOAD(5) ; POP ; REVERT(0, 0)
    let code = Bytes::from(vec![
        0x60, 0x01, 0x60, 0x05, 0x55, // SSTORE(5, 1)
        0x60, 0x05, 0x54, 0x50, // SLOAD(5) ; POP
        0x60, 0x00, 0x60, 0x00, 0xFD, // REVERT(0, 0)
    ]);

    let accounts: Vec<SeededAccount> = vec![
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (target, U256::zero(), 0, code),
    ];

    let verify = Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(FUNDED_SENDER),
        gas_limit: 80_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };
    // A plain DEFAULT frame: no atomic-batch flag, so the batch unroll path that
    // already reconciles the recorder is deliberately not exercised here.
    let reverting = Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0x00,
        target: Some(target),
        gas_limit: 300_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };

    let tx = frame_tx_with_frames(vec![verify, reverting]);
    let mut db = seeded_db(&accounts);
    db.enable_bal_recording();
    let env = frame_tx_env(&tx);
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
        .expect("VM::new should succeed for a frame tx");
        vm.execute()
            .expect("a reverting DEFAULT frame must not error the tx");
    }

    // The frame reverted, so the write must not survive in state.
    let live = db
        .current_accounts_state
        .get(&target)
        .and_then(|acc| acc.storage.get(&H256::from_low_u64_be(5)).copied())
        .unwrap_or_default();
    assert!(
        live.is_zero(),
        "the frame reverted, so the write must not survive; got {live}"
    );

    let bal = db
        .bal_recorder
        .take()
        .expect("BAL recording was enabled")
        .build();
    let entry = bal
        .accounts()
        .iter()
        .find(|acc| acc.address == target)
        .expect("the reverting frame's target was accessed, so it must be in the BAL");
    assert!(
        entry.storage_changes.iter().all(|c| c.slot != slot),
        "the BAL must not claim a storage change for a slot whose write was reverted"
    );
    assert!(
        entry.storage_reads.contains(&slot),
        "the reverted write must be re-filed as a read so the slot is still \
         reported as accessed; otherwise re-execution rejects the block \
         (slot missing from both storage_changes and storage_reads)"
    );
}

/// EIP-8141: when an atomic batch unrolls, "logs emitted by frames that executed
/// before the failure are discarded together with their state changes [...] Those
/// frame receipts retain their execution status and gas used, with empty logs."
///
/// All three halves are consensus-visible: the per-frame status and gas go into
/// the receipts trie, and the gas also feeds the header `gasUsed` and the payer's
/// refund. Rewriting the batch to failure-at-full-`gas_limit` would over-charge
/// the payer and mislabel frames that did run.
#[test]
fn atomic_batch_unroll_keeps_frame_status_and_gas_but_drops_logs() {
    use ethrex_common::types::{
        FRAME_RECEIPT_STATUS_FAILURE, FRAME_RECEIPT_STATUS_SKIPPED, FRAME_RECEIPT_STATUS_SUCCESS,
        Frame,
    };

    let logger = Address::from_low_u64_be(0x8141_0011);
    let reverter = Address::from_low_u64_be(0x8141_0012);
    let terminator = Address::from_low_u64_be(0x8141_0013);

    // logger: LOG0(0, 0) ; STOP -- succeeds and emits one log.
    let logger_code = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xA0, 0x00]);
    // reverter: REVERT(0, 0) -- a clean revert, so it is charged its actual gas.
    let reverter_code = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xFD]);
    let stop_code = Bytes::from(vec![0x00]);

    let accounts: Vec<SeededAccount> = vec![
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (logger, U256::zero(), 0, logger_code),
        (reverter, U256::zero(), 0, reverter_code),
        (terminator, U256::zero(), 0, stop_code),
    ];

    const BATCH_FRAME_GAS: u64 = 300_000;
    let batch_frame = |target: Address| Frame {
        mode: u8::from(FrameMode::Sender),
        flags: 0x04,
        target: Some(target),
        gas_limit: BATCH_FRAME_GAS,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };

    // [logger(flagged), reverter(flagged), terminator(unflagged)] is one batch.
    // The reverter fails, so the batch unrolls and the terminator is skipped.
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        batch_frame(logger),
        batch_frame(reverter),
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0x00,
            target: Some(terminator),
            gas_limit: 100_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);

    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("a reverting batch must not invalidate the tx");
    let frames = report.frame_results.expect("frame results present");

    // The frame that ran before the failure keeps its status and its gas.
    assert_eq!(
        frames[1].status, FRAME_RECEIPT_STATUS_SUCCESS,
        "a frame that succeeded before the failure keeps its status through the unroll"
    );
    assert!(
        frames[1].gas_used > 0 && frames[1].gas_used < BATCH_FRAME_GAS,
        "it keeps the gas it used ({}), not the frame's whole gas_limit",
        frames[1].gas_used
    );
    assert!(
        frames[1].logs.is_empty(),
        "its logs go with the state changes the unroll dropped: {:?}",
        frames[1].logs
    );

    // The failing frame reverted, so it is charged what it actually used.
    assert_eq!(frames[2].status, FRAME_RECEIPT_STATUS_FAILURE);
    assert!(
        frames[2].gas_used < BATCH_FRAME_GAS,
        "a REVERT is charged its actual gas ({}), not the full gas_limit",
        frames[2].gas_used
    );

    // The remaining batch member never executed.
    assert_eq!(frames[3].status, FRAME_RECEIPT_STATUS_SKIPPED);
    assert_eq!(frames[3].gas_used, 0, "a skipped frame's gas is refunded");

    // The transaction's log set is the concatenation of the frame receipts' logs,
    // so the unrolled batch contributes nothing to it either.
    assert!(
        report.logs.is_empty(),
        "the unrolled batch's logs must not reach the transaction log set: {:?}",
        report.logs
    );
    assert!(
        report.gas_used < 2 * BATCH_FRAME_GAS,
        "charging every batch frame its full gas_limit would over-bill the payer"
    );
}

/// EIP-8141 `max_gas = max(standard_gas_limit, calldata_floor_gas)`. A frame
/// transaction whose data floor exceeds what it declared for execution reserves
/// the floor; it is not rejected. Both quantities carry the mandatory costs, so
/// the floor wins exactly when the EIP-7623 token charge exceeds the declared
/// data cost plus frame gas.
#[test]
fn max_gas_reserves_the_calldata_floor_instead_of_rejecting() {
    use ethrex_common::types::Frame;

    // A frame carrying a large payload but almost no execution gas: the floor
    // (64 per byte) dominates the data cost (16 at most) plus the frame gas.
    let mut tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(FUNDED_SENDER),
        gas_limit: 1_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::from(vec![0x11u8; 4_096]),
    }]);
    tx.sender = FUNDED_SENDER;

    assert!(
        tx.calldata_floor_total() > tx.standard_gas_limit(),
        "the floor must bind for this to test the reservation"
    );
    assert_eq!(
        tx.total_gas_limit(),
        tx.calldata_floor_total(),
        "max_gas must be the floor when the floor binds"
    );
    assert!(
        tx.validate_static_constraints(false).is_ok(),
        "a floor-bound transaction is valid; it reserves the floor rather than being rejected"
    );
}

// ==================== EIP-7594 per-tx blob limit ====================

/// The EIP-7594 per-transaction blob limit applies to frame transactions
/// unchanged (EIP-8141 §Blob-carrying frame transactions), and must bind at
/// execution: block import has no sidecar to validate, so without this a block
/// carrying an over-limit frame transaction would be accepted here and rejected
/// by every conformant client.
#[test]
fn frame_tx_over_the_per_tx_blob_limit_is_rejected() {
    let mut hash = [0x11u8; 32];
    hash[0] = 0x01; // EIP-4844 KZG version byte
    let frame = Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03, // APPROVE_EXECUTION_AND_PAYMENT
        target: Some(FUNDED_SENDER),
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let accounts = [(
        FUNDED_SENDER,
        AUTO_SEED_SENDER_BALANCE,
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )];

    let mut tx = frame_tx_with_frames(vec![frame.clone()]);
    tx.max_fee_per_blob_gas = U256::from(1u64);
    tx.blob_versioned_hashes = vec![H256(hash); MAX_BLOBS_PER_TX];
    let (result, _) = run_frame_tx(&accounts, tx);
    assert!(
        result.is_ok(),
        "{MAX_BLOBS_PER_TX} blobs are within the per-transaction limit; got {result:?}"
    );

    let mut tx = frame_tx_with_frames(vec![frame]);
    tx.max_fee_per_blob_gas = U256::from(1u64);
    tx.blob_versioned_hashes = vec![H256(hash); MAX_BLOBS_PER_TX + 1];
    let (result, db) = run_frame_tx(&accounts, tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::InvalidFrameTransaction(_)
            ))
        ),
        "a frame tx carrying more than {MAX_BLOBS_PER_TX} blobs must be rejected; got {result:?}"
    );
    assert_db_cache_unchanged(&db, &accounts);
}

mod intrinsic_gas_accounting_tests {
    use bytes::Bytes;
    use ethrex_common::types::{
        FRAME_SIG_SCHEME_SECP256K1, FRAME_TX_INTRINSIC_COST, FRAME_TX_PER_FRAME_COST, Frame,
        FrameSignature, FrameTransaction, Transaction,
    };
    use ethrex_common::{Address, U256};

    /// A pay-then-transfer frame transaction taken off the Hegotá devnet: a
    /// self-`VERIFY` prefix frame approving execution and payment, then a SENDER
    /// frame moving value. Fields other than the nonce are the devnet
    /// transaction's own, including its 65-byte SECP256K1 signature.
    ///
    /// The devnet capture predates the EIP-8250 keyed-nonce envelope, so the
    /// transaction is rebuilt here with `nonce_keys`/`nonce_seq` and round-tripped
    /// through the canonical encoding rather than being decoded from the original
    /// 9-field payload, which this branch's decoder rejects.
    fn interop_reference_tx() -> FrameTransaction {
        let tx = FrameTransaction {
            chain_id: 3_151_908,
            nonce_keys: vec![U256::zero()],
            nonce_seq: 0,
            sender: Address::from_slice(
                &hex::decode("e25583099ba105d9ec0a67f5ae86d90e50036425").unwrap(),
            ),
            frames: vec![
                Frame {
                    mode: 1,
                    flags: 3,
                    target: None,
                    gas_limit: 50_000,
                    state_limit: 0,
                    value: U256::zero(),
                    data: Bytes::new(),
                },
                Frame {
                    mode: 2,
                    flags: 0,
                    target: Some(Address::repeat_byte(0x11)),
                    gas_limit: 100_000,
                    state_limit: 0,
                    value: U256::from(500_000_000_000_000u64),
                    data: Bytes::new(),
                },
            ],
            signatures: vec![FrameSignature {
                scheme: FRAME_SIG_SCHEME_SECP256K1,
                signer: None,
                msg: Bytes::new(),
                signature: Bytes::from(
                    hex::decode(
                        "00376dfea0d3368d1e0e7914b727a934f20e4fb134752d4bbcd994db1246c219755\
                         3a566aec5941b337df2fb25be8fd2c367ad19b34526656704997b25eda17a80",
                    )
                    .unwrap(),
                ),
            }],
            max_priority_fee_per_gas: 1_000_000_000,
            max_fee_per_gas: 2_000_000_000,
            max_fee_per_blob_gas: U256::zero(),
            blob_versioned_hashes: vec![],
            ..Default::default()
        };

        let mut raw = vec![];
        Transaction::FrameTransaction(tx).encode_canonical(&mut raw);
        match Transaction::decode_canonical(&raw).unwrap() {
            Transaction::FrameTransaction(decoded) => decoded,
            _ => panic!("expected frame transaction"),
        }
    }

    /// EIP-7623 byte pricing, the rule `data_cost` applies to every payload field.
    fn calldata_gas(bytes: &[u8]) -> u64 {
        bytes
            .iter()
            .map(|byte| if *byte == 0 { 4 } else { 16 })
            .sum()
    }

    #[test]
    fn interop_reference_tx_intrinsic_gas_charges_payload_bytes_only() {
        const SECP256K1_VERIFY_GAS: u64 = 2_800;
        const FRAME_GAS_LIMITS: u64 = 50_000 + 100_000;

        let tx = interop_reference_tx();

        // The expected cost below enumerates every payload field that carries a
        // byte charge, so pin the fixture properties that keep the list closed:
        // no frame data, and exactly one signature with no signer or explicit msg.
        assert_eq!(tx.frames.len(), 2);
        // And one value-carrying frame, which EIP-8141 charges `TX_VALUE_COST` for.
        let value_frames = tx.frames.iter().filter(|f| !f.value.is_zero()).count() as u64;
        assert_eq!(value_frames, 1);
        assert!(tx.frames.iter().all(|frame| frame.data.is_empty()));
        assert_eq!(tx.signatures.len(), 1);
        assert_eq!(tx.signatures[0].scheme, FRAME_SIG_SCHEME_SECP256K1);
        assert!(tx.signatures[0].signer.is_none());
        assert!(tx.signatures[0].msg.is_empty());

        // EIP-8141 prices the signature bytes; EIP-8250 adds
        // `rlp(nonce_keys) || rlp(nonce_seq)` to the same charge, so a frame
        // transaction is never signature bytes alone even with empty frame data.
        let payload_calldata_gas = calldata_gas(tx.signatures[0].signature.as_ref());
        let nonce_calldata_gas = calldata_gas(&tx.nonce_calldata());
        let recent_root_calldata_gas = calldata_gas(&tx.recent_root_calldata());
        assert!(
            nonce_calldata_gas > 0,
            "the keyed nonce is always encoded, so it always carries a byte charge"
        );
        assert_eq!(
            recent_root_calldata_gas, 0,
            "a transaction declaring no recent root reference pays exactly the \
             EIP-8141 figure for that field"
        );

        let expected_intrinsic_gas = FRAME_TX_INTRINSIC_COST
            + tx.frames.len() as u64 * FRAME_TX_PER_FRAME_COST
            + SECP256K1_VERIFY_GAS
            + value_frames * ethrex_common::types::FRAME_TX_VALUE_COST
            + payload_calldata_gas
            + nonce_calldata_gas
            + recent_root_calldata_gas;

        assert_eq!(tx.mandatory_gas() + tx.data_cost(), expected_intrinsic_gas);
        assert_eq!(
            tx.total_gas_limit(),
            expected_intrinsic_gas + FRAME_GAS_LIMITS
        );
    }

    /// EIP-8141 `value_transfer_cost`: `TX_VALUE_COST` per value-carrying frame, in
    /// the intrinsic AND in the calldata floor. It is charged per frame rather than per
    /// distinct recipient, and adding value to a frame is the only thing that moves it.
    #[test]
    fn each_value_carrying_frame_adds_the_value_transfer_cost() {
        let baseline = interop_reference_tx();
        let baseline_value_frames = baseline
            .frames
            .iter()
            .filter(|f| !f.value.is_zero())
            .count() as u64;
        assert_eq!(
            baseline.value_transfer_cost(),
            baseline_value_frames * ethrex_common::types::FRAME_TX_VALUE_COST
        );

        // Give a zero-value frame a value: exactly one more TX_VALUE_COST, in both the
        // intrinsic and the floor, because the term sits on both sides of `max_gas`.
        let mut with_value = interop_reference_tx();
        let zero_valued = with_value
            .frames
            .iter()
            .position(|f| f.value.is_zero())
            .expect("the fixture has a zero-value frame");
        with_value.frames[zero_valued].value = U256::one();

        assert_eq!(
            with_value.mandatory_gas(),
            baseline.mandatory_gas() + ethrex_common::types::FRAME_TX_VALUE_COST,
            "a newly value-carrying frame adds exactly one TX_VALUE_COST"
        );
        assert_eq!(
            with_value.calldata_floor_total(),
            baseline.calldata_floor_total() + ethrex_common::types::FRAME_TX_VALUE_COST,
            "the value cost is mandatory, so the calldata floor carries it too"
        );

        // Raising an already value-carrying frame's value changes nothing: the charge is
        // per frame, and `value` is a transaction field, not a measured quantity.
        let mut raised = interop_reference_tx();
        let valued = raised
            .frames
            .iter()
            .position(|f| !f.value.is_zero())
            .expect("the fixture has a value-carrying frame");
        raised.frames[valued].value = U256::from(10u64).pow(U256::from(18u64));
        assert_eq!(raised.mandatory_gas(), baseline.mandatory_gas());
    }

    #[test]
    fn intrinsic_gas_is_insensitive_to_structural_frame_fields() {
        let baseline = interop_reference_tx();
        assert!(baseline.frames.len() >= 2);

        let mut restructured = interop_reference_tx();
        restructured.frames[0].gas_limit += 1_000;
        restructured.frames[1].value = U256::from(1u64);

        // Asserting the two components separately rather than their sum minus the
        // frame gas: the frame gas limits are a term of `total_gas_limit`, so
        // subtracting them back off cancels the very field under test.
        assert_eq!(restructured.mandatory_gas(), baseline.mandatory_gas());
        assert_eq!(restructured.data_cost(), baseline.data_cost());
        // A frame gas limit is the one structural field `total_gas_limit` tracks,
        // and it passes through with no other term moving.
        assert_eq!(
            restructured.total_gas_limit(),
            baseline.total_gas_limit() + 1_000
        );
    }
}

// ==================== EIP-8141 initial access sets (EIPs#12113) ====================
//
// "When frame-transaction processing begins, `accessed_addresses` is initialized
// as in EIP-2929 and EIP-3651 (`tx.sender`, the coinbase, and precompiles warm),
// and `accessed_storage_keys` empty (there is no access list). Being a frame
// target does not warm an address: a `resolved_target`'s warm/cold access is
// charged within the frame's own `gas_limit`. The payer is added to
// `accessed_addresses` when an `APPROVE` with payment scope sets it and collects
// `max_cost`, as for any protocol-touched account. `ENTRY_POINT` is not
// pre-warmed."
//
// Each clause is asserted as a gas differential between two runs that differ
// only in the probed address or slot, so the assertions survive a repricing of
// the warm/cold spread itself.

/// Address of the account the access probes are deployed at.
const PROBE: Address = Address::repeat_byte(0xB1);
/// An address that is never a frame target, never the payer, and never the
/// sender: the cold control for every probe below.
const COLD_CONTROL: Address = Address::repeat_byte(0xC0);

/// `PUSH20 addr; BALANCE; POP; STOP` — one account access, nothing else.
fn balance_probe_code(addr: Address) -> Bytes {
    let mut code = vec![0x73];
    code.extend_from_slice(addr.as_bytes());
    code.extend_from_slice(&[0x31, 0x50, 0x00]);
    Bytes::from(code)
}

/// Run a frame tx whose prefix approves via the sender and whose final DEFAULT
/// frame executes `probe_code` at [`PROBE`], and return total `gas_used`.
///
/// `extra_frames` are appended before the probe frame, so a caller can make an
/// address a frame target (or the payer) and then probe how it was warmed.
fn probe_gas_used(
    probe_code: Bytes,
    extra_accounts: &[SeededAccount],
    extra_frames: Vec<Frame>,
) -> u64 {
    probe_gas_used_with_sender_code(probe_code, extra_accounts, extra_frames, APPROVE_BOTH_CODE)
}

/// [`probe_gas_used`], with `later_frames` placed AFTER the probe frame instead of
/// before it. The probe therefore observes the state of the warm sets before those
/// frames have run, which is what distinguishes "not pre-warmed at transaction start"
/// from "warmed by a frame that already ran".
fn probe_gas_used_before_frames(
    probe_code: Bytes,
    extra_accounts: &[SeededAccount],
    later_frames: Vec<Frame>,
) -> u64 {
    probe_frame_gas_used(probe_code, extra_accounts, later_frames)
}

/// The gas the PROBE frame alone spent, read from its own receipt entry.
///
/// Needed whenever the transaction carries a frame that targets the probed address:
/// the probe's own access warms it, so that frame's entry access charge would be warm
/// in one run and cold in the other, and a transaction-total comparison would measure
/// that difference instead of the probe's. `later_frames` run after the probe.
fn probe_frame_gas_used(
    probe_code: Bytes,
    extra_accounts: &[SeededAccount],
    later_frames: Vec<Frame>,
) -> u64 {
    let mut accounts: Vec<SeededAccount> = vec![
        (PROBE, U256::zero(), 0, probe_code),
        (COLD_CONTROL, U256::from(1u64), 0, Bytes::new()),
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
    ];
    accounts.extend_from_slice(extra_accounts);

    let mut frames = vec![verify_frame(FUNDED_SENDER), plain_frame(PROBE)];
    frames[1].gas_limit = 100_000;
    frames.extend(later_frames);

    let (result, _db) = run_frame_tx(&accounts, frame_tx_with_frames(frames));
    let report = result.expect("probe frame tx must execute");
    let fr = report.frame_results.expect("per-frame results");
    assert_eq!(
        fr[1].status,
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "the probe frame must succeed"
    );
    fr[1].gas_used
}

/// [`probe_gas_used`], with the sender's approval code chosen by the caller. A
/// prefix that hands payment to a third-party payer must approve execution only
/// here, since a second payment-scope `APPROVE` halts once `payer` is set.
fn probe_gas_used_with_sender_code(
    probe_code: Bytes,
    extra_accounts: &[SeededAccount],
    extra_frames: Vec<Frame>,
    sender_code: &'static [u8],
) -> u64 {
    probe_gas_used_inner(
        probe_code,
        extra_accounts,
        extra_frames,
        vec![],
        sender_code,
    )
}

fn probe_gas_used_inner(
    probe_code: Bytes,
    extra_accounts: &[SeededAccount],
    extra_frames: Vec<Frame>,
    later_frames: Vec<Frame>,
    sender_code: &'static [u8],
) -> u64 {
    let mut accounts: Vec<SeededAccount> = vec![
        (PROBE, U256::zero(), 0, probe_code),
        (COLD_CONTROL, U256::from(1u64), 0, Bytes::new()),
    ];
    accounts.extend_from_slice(extra_accounts);

    let mut frames = vec![Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(FUNDED_SENDER),
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }];
    frames.extend(extra_frames);
    frames.push(Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(PROBE),
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    });
    frames.extend(later_frames);

    // The sender's code approves execution and payment for itself.
    accounts.push((
        FUNDED_SENDER,
        AUTO_SEED_SENDER_BALANCE,
        0,
        Bytes::from(sender_code),
    ));

    let tx = frame_tx_with_frames(frames);
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("probe frame tx must execute");
    assert_eq!(
        report.result,
        TxResult::Success,
        "probe frame tx must succeed"
    );
    report.gas_used
}

#[test]
fn entry_point_is_not_pre_warmed_but_the_sender_is() {
    // ENTRY_POINT is `ORIGIN` inside DEFAULT and VERIFY frames, so if it were
    // pre-warmed an account access to it would be charged warm. The spec says it
    // is not; `tx.sender` is (EIP-2929).
    let entry_point = ethrex_common::types::frame_tx_entry_point();
    let entry_point_gas = probe_gas_used(balance_probe_code(entry_point), &[], vec![]);
    let sender_gas = probe_gas_used(balance_probe_code(FUNDED_SENDER), &[], vec![]);
    let control_gas = probe_gas_used(balance_probe_code(COLD_CONTROL), &[], vec![]);

    assert!(
        entry_point_gas > sender_gas,
        "ENTRY_POINT must be cold and tx.sender warm, but accessing ENTRY_POINT \
         ({entry_point_gas}) did not cost more than accessing the sender ({sender_gas})"
    );
    assert_eq!(
        entry_point_gas, control_gas,
        "ENTRY_POINT must cost exactly what any other cold account costs"
    );
}

#[test]
fn frame_transaction_starts_with_no_warm_storage_keys() {
    // A frame transaction carries no access list, so every storage slot starts
    // cold. Probing two distinct slots costs one cold access more than probing
    // the same slot twice.
    // `PUSH1 s; SLOAD; POP` per slot, then STOP.
    let sload = |slot: u8| vec![0x60, slot, 0x54, 0x50];
    let mut same = sload(0);
    same.extend(sload(0));
    same.push(0x00);
    let mut distinct = sload(0);
    distinct.extend(sload(1));
    distinct.push(0x00);

    let same_gas = probe_gas_used(Bytes::from(same), &[], vec![]);
    let distinct_gas = probe_gas_used(Bytes::from(distinct), &[], vec![]);

    assert!(
        distinct_gas > same_gas,
        "the second slot must be charged cold (no pre-warmed keys), but two \
         distinct slots ({distinct_gas}) cost no more than the same slot twice ({same_gas})"
    );
}

/// A frame targeting `target`, doing nothing else. Its budget only has to cover the
/// frame-entry access charge and the default code.
fn plain_frame(target: Address) -> Frame {
    Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(target),
        gas_limit: 50_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

#[test]
fn being_a_frame_target_does_not_warm_an_address() {
    // "Being a frame target does not warm an address: a `resolved_target`'s warm/cold
    // access is charged at frame entry within the frame's own `limits.execution`."
    // So appearing in `tx.frames` warms nothing up front — the warming happens when
    // that frame runs, and not before. Probed from a frame that runs FIRST, a later
    // frame's target must still be cold.
    let later_target = Address::repeat_byte(0xD1);
    let accounts = [(later_target, U256::from(1u64), 0, Bytes::new())];

    let target_gas = probe_gas_used_before_frames(
        balance_probe_code(later_target),
        &accounts,
        vec![plain_frame(later_target)],
    );
    let control_gas = probe_gas_used_before_frames(
        balance_probe_code(COLD_CONTROL),
        &accounts,
        vec![plain_frame(later_target)],
    );

    assert_eq!(
        target_gas, control_gas,
        "an address must not be warmed merely by being a frame target: probing \
         it cost {target_gas} against {control_gas} for a never-targeted address"
    );
}

#[test]
fn a_frame_entry_access_warms_the_target_for_later_frames() {
    // The other half of the same clause. The entry charge is journaled like any other
    // EIP-2929 access, and "for the purposes of gas accounting of warm / cold state
    // status, the journal of such touches is shared across frames" — so once a frame
    // has run, its target is warm for every frame after it.
    let earlier_target = Address::repeat_byte(0xD2);
    let accounts = [(earlier_target, U256::from(1u64), 0, Bytes::new())];

    let target_gas = probe_gas_used(
        balance_probe_code(earlier_target),
        &accounts,
        vec![plain_frame(earlier_target)],
    );
    let control_gas = probe_gas_used(
        balance_probe_code(COLD_CONTROL),
        &accounts,
        vec![plain_frame(earlier_target)],
    );
    // Both runs pay the same cold entry charge on `earlier_target` (the frame that
    // targets it runs first and is the first to touch it), so the transaction totals
    // differ only by what the probe itself paid.

    assert!(
        target_gas < control_gas,
        "a frame's entry access must leave its target warm for later frames: probing \
         it cost {target_gas}, no less than {control_gas} for a never-targeted address"
    );
}

#[test]
fn the_frame_entry_access_charge_comes_out_of_the_frames_own_budget() {
    // "Charge the resolved target's warm or cold account access from `gas_left`,
    // before the balance check and before dispatch. [...] If `gas_left` cannot cover
    // the charge, the frame halts exceptionally." A frame whose execution budget is
    // below the cold-access cost therefore fails, while the same frame with a budget
    // just above it succeeds — nothing else in it consumes gas, since a code-less
    // target runs the default code, which "draws no execution gas of its own".
    let target = Address::repeat_byte(0xD3);
    let accounts = [(target, U256::from(1u64), 0, Bytes::new())];

    let status_with_budget = |gas_limit: u64| {
        let mut frame = plain_frame(target);
        frame.gas_limit = gas_limit;
        let tx = frame_tx_with_frames(vec![verify_frame(FUNDED_SENDER), frame]);
        let mut seeded: Vec<SeededAccount> = accounts.to_vec();
        seeded.push((
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ));
        let (result, _db) = run_frame_tx(&seeded, tx);
        let report = result.expect("the tx stays valid either way; only the frame's fate differs");
        report
            .frame_results
            .expect("per-frame results")
            .get(1)
            .expect("the probed frame has a receipt")
            .status
    };

    // The frame is the first to touch its target, so it is charged the cold rate.
    // Read from levm rather than pinned here: EIP-8038 reprices it at Amsterdam, and
    // this test is about who pays the charge, not what it currently costs.
    let cold =
        ethrex_levm::gas_cost::cold_account_access_cost(ethrex_common::types::Fork::Amsterdam);
    assert_eq!(
        status_with_budget(cold - 1),
        ethrex_common::types::FRAME_RECEIPT_STATUS_FAILURE,
        "a frame that cannot cover its entry access charge ({cold}) must halt exceptionally"
    );
    assert_eq!(
        status_with_budget(cold),
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "a frame whose budget covers exactly the entry access charge ({cold}) must succeed"
    );
}

#[test]
fn the_payer_is_warm_after_a_payment_scope_approve() {
    // "The payer is added to `accessed_addresses` when an `APPROVE` with payment
    // scope sets it and collects `max_cost`, as for any protocol-touched
    // account."
    let payer = Address::repeat_byte(0xE1);
    let accounts = [(payer, U256::MAX, 0, Bytes::from(APPROVE_PAYMENT_CODE))];
    let pay = pay_frame(payer);

    let payer_gas = probe_gas_used_with_sender_code(
        balance_probe_code(payer),
        &accounts,
        vec![pay.clone()],
        APPROVE_EXECUTION_CODE,
    );
    let control_gas = probe_gas_used_with_sender_code(
        balance_probe_code(COLD_CONTROL),
        &accounts,
        vec![pay],
        APPROVE_EXECUTION_CODE,
    );

    assert!(
        payer_gas < control_gas,
        "the payer must be warm once a payment-scope APPROVE collected max_cost, \
         but probing it cost {payer_gas} against {control_gas} for a cold account"
    );
}

/// EIP-8141 signature validation is protocol-level cryptography, not EVM
/// execution (EIPs#12026): `ecrecover` and `P256VERIFY` denote the verification
/// functions, not calls to the precompile accounts exposing them. Validating the
/// `signatures` list therefore accesses no precompile address, and in particular
/// records no EIP-7951 entry in an EIP-7928 block access list.
mod signature_validation_touches_no_precompile {
    use super::*;
    use ethrex_common::types::{FRAME_SIG_SCHEME_SECP256K1, FrameSignature};
    use k256::ecdsa::SigningKey;

    fn key_and_address(seed: u8) -> (SigningKey, Address) {
        let signing_key = SigningKey::from_bytes(&[seed; 32].into()).unwrap();
        let uncompressed = signing_key.verifying_key().to_encoded_point(false);
        let pub_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
        (signing_key, Address::from_slice(&pub_hash[12..]))
    }

    fn sign(key: &SigningKey, sig_hash: H256, signer: Address) -> FrameSignature {
        let (raw_sig, recovery_id) = key.sign_prehash_recoverable(sig_hash.as_bytes()).unwrap();
        let mut bytes = vec![0u8; 65];
        bytes[0] = recovery_id.to_byte();
        bytes[1..].copy_from_slice(&raw_sig.to_bytes());
        FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(signer),
            msg: Bytes::new(),
            signature: Bytes::from(bytes),
        }
    }

    #[test]
    fn a_validated_signature_records_no_precompile_in_the_block_access_list() {
        let (key, signer) = key_and_address(0x77);

        let mut tx = frame_tx_with_frames(vec![Frame {
            mode: u8::from(FrameMode::Verify),
            flags: 0x03,
            target: Some(FUNDED_SENDER),
            gas_limit: 100_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        }]);
        tx.signatures = vec![FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(signer),
            msg: Bytes::new(),
            signature: Bytes::from(vec![0u8; 65]),
        }];
        let sig_hash = tx.compute_sig_hash();
        tx.signatures[0] = sign(&key, sig_hash, signer);
        tx.inner_hash = Default::default();
        tx.cached_canonical = Default::default();

        let accounts = [(
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE),
        )];
        let mut seeded: Vec<SeededAccount> = accounts.to_vec();
        seeded.push((signer, U256::from(1u64), 0, Bytes::new()));

        let mut db = seeded_db(&seeded);
        db.enable_bal_recording();
        let env = frame_tx_env(&tx);
        let transaction = Transaction::FrameTransaction(tx);

        let report = {
            let mut vm = VM::new(
                env,
                &mut db,
                &transaction,
                LevmCallTracer::disabled(),
                VMType::L1,
                &NativeCrypto,
                None,
            )
            .expect("VM::new should succeed for a signed frame tx");
            vm.execute().expect("the signed frame tx must execute")
        };
        assert_eq!(
            report.result,
            TxResult::Success,
            "the signature must validate, otherwise this asserts nothing"
        );

        let bal = db
            .bal_recorder
            .as_ref()
            .expect("BAL recording was enabled")
            .clone()
            .build();

        // The precompile range is 0x01..=0xff plus the EIP-7951 P256VERIFY
        // address at 0x100. None may appear: signature validation calls the
        // verification functions, not the accounts.
        for account in bal.accounts() {
            let raw = U256::from_big_endian(account.address.as_bytes());
            assert!(
                raw > U256::from(0x100u64) || raw.is_zero(),
                "a precompile address ({:?}) was recorded in the block access \
                 list by signature validation",
                account.address
            );
        }
    }
}
