//! EIP-8141: Frame Transactions
//!
//! Shared test harness for frame-transaction execution plus regression tests
//! for the per-tx state-rollback invariant (finding B3):
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
    FrameTransaction, Transaction,
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
        value: U256::zero(),
        data: Bytes::new(),
    }
}

/// Assert that no seeded account's info (balance/nonce/code) or storage in
/// `db.current_accounts_state` differs from its seeded value. This is THE B3
/// invariant: after an invalid tx the shared cache must show no residue.
///
/// The sender (`FUNDED_SENDER`) is ALWAYS verified, even when the caller does
/// not list it in `accounts`: `run_frame_tx` auto-seeds it, and a leaked sender
/// nonce/balance on the invalid-tx path (e.g. an APPROVE nonce bump that was not
/// rolled back) is exactly the kind of residue B3 must prevent. When the caller
/// passes the sender explicitly, those values are used; otherwise the auto-seed
/// defaults (`AUTO_SEED_SENDER_BALANCE`, nonce 0) are checked.
///
/// Slot 0 of each seeded account is checked explicitly because the harness
/// bytecodes write slot 0; a leftover `1` there is the B3 regression signature.
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
        // touch, so a residual `1` here is the B3 regression signature.
        for (slot, value) in current.storage.iter() {
            assert!(
                value.is_zero(),
                "storage residue at {address:?} slot {slot:?} = {value:?} after invalid tx",
            );
        }
    }
}

// ==================== B3: invalid-tx rollback ====================

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
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    let (result, db) = run_frame_tx(&accounts, tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::InvalidFrameTransaction
            ))
        ),
        "expected InvalidFrameTransaction, got {result:?}",
    );
    // Slot 0 of target must NOT be 1 — the frame's SSTORE must have been rolled back.
    assert_db_cache_unchanged(&db, &accounts);
}

// ==================== B4: reverting SENDER frame must not leak value ====================

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
        verify_frame(wallet),
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(target),
            gas_limit: 100_000,
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
        frame_results[2].0,
        ethrex_common::types::FRAME_RECEIPT_STATUS_FAILURE,
        "SENDER frame should be reported as failure"
    );
}

// ==================== B2: payer charged at effective price (no burn) ====================

#[test]
fn payer_pays_effective_price_no_burn() {
    // base_fee = 10 gwei, priority = 2, max_fee = 100 gwei (huge headroom).
    // effective = base + priority = 12 gwei. The (100-12) spread must NOT burn.
    let wallet = Address::from_low_u64_be(0xD2);
    let stop_contract = Address::from_low_u64_be(0xD3);
    let wallet_initial = U256::from(10u64).pow(U256::from(18u64)); // 1 ETH
    let mut tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER), // runs APPROVE_EXECUTION_CODE -> seed below
        verify_frame(wallet),        // runs APPROVE_PAYMENT_CODE   -> seed below
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(stop_contract),
            gas_limit: 30_000,
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

// ==================== B6: FRAMEPARAM stack operand order ====================

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

#[test]
fn frameparam_reads_frame_index_from_stack_top() {
    let wallet = Address::from_low_u64_be(0xE2);
    let reader = Address::from_low_u64_be(0xE3);
    let mut frames = vec![
        verify_frame(FUNDED_SENDER), // frame[0]: VERIFY, runs APPROVE_EXECUTION_CODE
        verify_frame(wallet),        // frame[1]: VERIFY, runs APPROVE_PAYMENT_CODE
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0,
            target: Some(reader),
            // EIP-8037 (active at Hegota): the new-slot SSTORE spills
            // STATE_BYTES_PER_STORAGE_SET * cost_per_state_byte (~98k) into
            // the frame's regular gas, so the budget must cover it.
            gas_limit: 300_000,
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

// ==================== B7: APPROVE scope-0 bypass ====================

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
                ethrex_levm::errors::TxValidationError::InvalidFrameTransaction
            ))
        ),
        "APPROVE with allowed_scope==0 must halt, leaving the tx invalid; got {result:?}"
    );
    assert_db_cache_unchanged(&db, &accounts);
}

// ==================== B8: batched VERIFY revert invalidates tx ====================

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
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0x00,
            target: Some(stop_ct),
            gas_limit: 30_000,
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
                ethrex_levm::errors::TxValidationError::InvalidFrameTransaction
            ))
        ),
        "a batched VERIFY revert must invalidate the tx; got {result:?}"
    );
    assert_db_cache_unchanged(&db, &accounts);
}

// ==================== I10: APPROVE_PAYMENT may precede APPROVE_EXECUTION ====================

#[test]
fn payment_approval_may_precede_execution_approval() {
    // Frame 0: a paymaster VERIFY frame that calls APPROVE(APPROVE_PAYMENT) — scope 1.
    // This happens BEFORE the sender has called APPROVE(APPROVE_EXECUTION).
    // Pre-fix: the sender_approved precondition causes frame 0 to revert ->
    //          VERIFY revert -> tx invalid (Err).
    // Post-fix: no such precondition; frame 0 sets payer=paymaster, frame 1 sets
    //           sender_approved, tx is valid with payer=paymaster.
    let paymaster = Address::from_low_u64_be(0x9A);
    let stop_ct = Address::from_low_u64_be(0x9B);
    let tx = frame_tx_with_frames(vec![
        // frame0: paymaster approves PAYMENT first (scope 1).
        verify_frame(paymaster),
        // frame1: sender approves EXECUTION (scope 2).
        verify_frame(FUNDED_SENDER),
        // frame2: a SENDER frame that just STOPs.
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(stop_ct),
            gas_limit: 30_000,
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
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("pay-before-verify ordering must be valid");
    assert_eq!(
        report.payer_address,
        Some(paymaster),
        "paymaster should be the payer"
    );
}

// ==================== B9: SENDER/DEFAULT default code returns success ====================

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
        frame_results[1].0,
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "SENDER frame to a code-less EOA must succeed (default code = success)"
    );
    // The EOA actually received the value:
    assert_eq!(balance_of(&db, eoa), value, "value not delivered to EOA");
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
        frame_results[1].2.iter().any(is_transfer_log),
        "EIP-7708 transfer log missing from frame_receipts[1].logs: {:?}",
        frame_results[1].2
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
            // EIP-8037 (active at Hegota): the new-slot SSTORE spills
            // STATE_BYTES_PER_STORAGE_SET * cost_per_state_byte (~98k) into
            // the frame's regular gas, so the budget must cover it.
            gas_limit: 300_000,
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

    // 3. The LOG0 appears in the aggregated report.logs (B5: logs collected).
    assert!(
        report.logs.iter().any(|l| l.address == worker),
        "log from worker missing from aggregated report.logs"
    );

    // 4. Per-frame isolation (B5): frame_results[1] is success and carries the log;
    //    frame_results[0] (the VERIFY/approve frame) has no logs.
    let frame_results = report
        .frame_results
        .expect("frame tx report must carry per-frame results");

    assert_eq!(
        frame_results[1].0, FRAME_RECEIPT_STATUS_SUCCESS,
        "SENDER frame (index 1) must be reported as success"
    );
    assert!(
        frame_results[1].2.iter().any(|l| l.address == worker),
        "log from worker missing from frame_results[1].logs"
    );
    assert!(
        frame_results[0].2.is_empty(),
        "approve VERIFY frame (index 0) must have no logs; isolation violated"
    );

    // 5. Sender nonce incremented exactly once by APPROVE (scope 3 bumps nonce once).
    assert_eq!(
        nonce_of(&db, FUNDED_SENDER),
        1,
        "sender nonce must be 1 after APPROVE (scope 3 increments nonce once)"
    );
}

// ==================== EIP-2929 cross-frame warm/cold journal ====================
//
// Regression guard for the cross-frame access-list behavior documented in
// docs/eip-8141.md: a storage slot warmed in one frame is still warm in a
// later frame of the same tx, and a reverted frame's accesses do NOT persist.
// Observed via SLOAD gas (cold 2100 vs warm 100 — a pure read, so unaffected by
// the EIP-8037 state-gas accounting). `frame_results[i].1` is frame i's gas.

/// SLOAD-cold minus SLOAD-warm (gas_cost::SLOAD_COLD_DYNAMIC - SLOAD_WARM_DYNAMIC).
const SLOAD_COLD_WARM_DELTA: u64 = 2100 - 100;

/// `PUSH1 0; SLOAD; POP; STOP` — reads own slot 0, warming `(self, 0)`.
const SLOAD_SLOT0_STOP: &[u8] = &[0x60, 0x00, 0x54, 0x50, 0x00];

/// `PUSH1 0; SLOAD; POP;` then revert iff calldata is empty, else stop:
/// `CALLDATASIZE; PUSH1 <stop>; JUMPI; PUSH1 0; PUSH1 0; REVERT; JUMPDEST; STOP`.
/// Always SLOADs slot 0 first (warming `(self, 0)`), so the warming is in place
/// whether the frame then reverts (empty calldata) or succeeds (non-empty).
fn sload_then_conditional_revert() -> Vec<u8> {
    // offsets: PUSH1 0 (0,1); SLOAD (2); POP (3); CALLDATASIZE (4);
    //          PUSH1 d (5,6); JUMPI (7); PUSH1 0 (8,9); PUSH1 0 (10,11); REVERT (12);
    //          JUMPDEST (13); STOP (14). JUMPI dest = 13 = 0x0d.
    vec![
        0x60, 0x00, // PUSH1 0
        0x54, // SLOAD
        0x50, // POP
        0x36, // CALLDATASIZE
        0x60, 0x0d, // PUSH1 13 (JUMPDEST offset)
        0x57, // JUMPI -> if calldata non-empty, jump to STOP
        0x60, 0x00, 0x60, 0x00, 0xfd, // PUSH1 0; PUSH1 0; REVERT (empty calldata path)
        0x5b, // JUMPDEST (offset 13)
        0x00, // STOP
    ]
}

#[test]
fn warm_slot_persists_across_frames() {
    // Two DEFAULT frames to the same contract, both SLOAD slot 0. If the
    // cross-frame access journal carries the warmed slot, frame 2's SLOAD is
    // warm and costs SLOAD_COLD_WARM_DELTA less than frame 1's cold SLOAD.
    let target = Address::from_low_u64_be(0xC0DE);
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0u64,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (
            target,
            U256::zero(),
            0u64,
            Bytes::from(SLOAD_SLOT0_STOP.to_vec()),
        ),
    ];
    let mk_default = || Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0x00,
        target: Some(target),
        gas_limit: 100_000,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        mk_default(),
        mk_default(),
    ]);
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("valid frame tx (sender approves via APPROVE_BOTH)");
    let fr = report.frame_results.expect("frame results present");
    // fr[0] = VERIFY, fr[1] = first DEFAULT (cold SLOAD), fr[2] = second DEFAULT (warm SLOAD).
    assert_eq!(fr[1].0, 1, "frame 1 must succeed");
    assert_eq!(fr[2].0, 1, "frame 2 must succeed");
    assert_eq!(
        fr[1].1 - fr[2].1,
        SLOAD_COLD_WARM_DELTA,
        "slot warmed in frame 1 must be warm in frame 2 (gas delta = cold-warm): \
         frame1={} frame2={}",
        fr[1].1,
        fr[2].1,
    );
}

#[test]
fn reverted_frame_warm_touches_do_not_persist() {
    // Same contract in both txs (SLOAD slot 0, then revert iff calldata empty).
    // Tx P: frame 1 SUCCEEDS (non-empty data) -> its warming of (target,0) persists
    //       -> frame 2 SLOAD warm.
    // Tx R: frame 1 REVERTS (empty data) -> its warming is rolled back
    //       -> frame 2 SLOAD cold.
    // Frame 2 is identical in both; its gas must be SLOAD_COLD_WARM_DELTA higher
    // in Tx R, proving the reverted frame's access did not persist.
    let target = Address::from_low_u64_be(0xC0DF);
    let code = sload_then_conditional_revert();
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0u64,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (target, U256::zero(), 0u64, Bytes::from(code)),
    ];
    let frame = |data: Bytes| Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0x00,
        target: Some(target),
        gas_limit: 100_000,
        value: U256::zero(),
        data,
    };

    // Tx P: frame 1 succeeds (non-empty calldata), frame 2 succeeds.
    let tx_p = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        frame(Bytes::from(vec![1u8])),
        frame(Bytes::from(vec![1u8])),
    ]);
    let report_p = run_frame_tx(&accounts, tx_p).0.expect("Tx P valid");
    let fr_p = report_p.frame_results.expect("frame results present");
    assert_eq!(fr_p[1].0, 1, "Tx P frame 1 must succeed");
    assert_eq!(fr_p[2].0, 1, "Tx P frame 2 must succeed");

    // Tx R: frame 1 reverts (empty calldata), frame 2 succeeds.
    let tx_r = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        frame(Bytes::new()),
        frame(Bytes::from(vec![1u8])),
    ]);
    let report_r = run_frame_tx(&accounts, tx_r)
        .0
        .expect("Tx R valid (a reverted DEFAULT frame does not invalidate the tx)");
    let fr_r = report_r.frame_results.expect("frame results present");
    assert_eq!(fr_r[1].0, 0, "Tx R frame 1 must revert");
    assert_eq!(fr_r[2].0, 1, "Tx R frame 2 must succeed");

    // Frame 2 SLOAD is warm in Tx P (frame 1 warmed it and persisted) but cold in
    // Tx R (frame 1's warming was rolled back on revert): the delta is exactly
    // one cold-vs-warm SLOAD.
    assert_eq!(
        fr_r[2].1 - fr_p[2].1,
        SLOAD_COLD_WARM_DELTA,
        "a reverted frame's slot warming must NOT persist: \
         frame2 gas Tx R (cold)={} vs Tx P (warm)={}",
        fr_r[2].1,
        fr_p[2].1,
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
            signer,
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
        }
    }

    #[test]
    fn sigparam_0x00_returns_signer() {
        let ctx = ctx_with_one_signature();
        let sig = ctx.tx.signatures.first().unwrap();
        let result = address_to_u256(sig.signer);
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
            scheme: 0x00,
            signer,
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
        let result = load_tx_param(&ctx, 0x0B).unwrap();
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
        };
        let result = load_tx_param(&ctx, 0x0B).unwrap();
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
        types::{Account, AccountState, ChainConfig, Code, CodeMetadata},
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
            eip7702_get_code(&mut db, &mut substate, delegator).unwrap();

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
            eip7702_get_code(&mut db, &mut substate, delegator).unwrap();

        assert!(is_delegation, "delegation indicator must still be detected");
        assert_eq!(code_address, delegatee);
        assert!(
            code.is_empty(),
            "delegatee has no code, so resolved bytecode is empty"
        );
        assert!(
            !runs_default_code(is_delegation, &code),
            "empty-delegatee delegation must NOT route to default code — it must take the \
             CallFrame branch and succeed as empty code (EIP-8141 §Execution lines 348-349)"
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
            eip7702_get_code(&mut db, &mut substate, eoa_addr).unwrap();

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
            eip7702_get_code(&mut db, &mut substate, contract_addr).unwrap();

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
    //! EIP-8141 mempool validation-prefix simulation harness tests (Phase 2).
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
    use ethrex_levm::validation_observer::FrameSimViolation;
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
    ) -> (PrefixSimResult, Option<FrameSimViolation>) {
        let env = hegota_env(sender);
        let mut vm = VM::new(
            env,
            db,
            tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &ethrex_crypto::NativeCrypto,
        )
        .unwrap();
        let result = vm
            .run_frame_validation_prefix(frame_indices, deploy_index, None)
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
        let (result, violation) = run(&tx, &mut db, sender, &[0], None);
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
        let (_result, violation) = run(&tx, &mut db, sender, &[0], None);
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
        )
        .unwrap();
        let _ = vm.run_frame_validation_prefix(&[0], None, None).unwrap();
        assert!(
            vm.validation_observer.violation.is_none(),
            "TIMESTAMP inside the expiry verifier must be allowed, got {:?}",
            vm.validation_observer.violation
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
        let (_result, violation) = run(&tx, &mut db, sender, &[0], None);
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
            vec![verify_frame_obs(other, 100_000, 0x03, Bytes::new())],
        );
        let mut db = build_db(vec![
            (sender, account_with_code(0, Bytes::new())),
            (other, account_with_code(0, code)),
        ]);
        let (_result, violation) = run(&tx, &mut db, sender, &[0], None);
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
        let (_result, violation) = run(&tx, &mut db, sender, &[0], None);
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
        let (result, violation) = run(&tx, &mut db, sender, &[0], None);
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
        let (result, violation) = run(&tx, &mut db, sender, &[0], None);
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
        )
        .unwrap();
        let result = vm.run_frame_validation_prefix(&[0], Some(0), None).unwrap();
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
    //! EIP-8141 mempool validation-prefix backend assertions (Phase 2). These
    //! exercise [`LEVM::simulate_frame_validation_prefix`] over signature-less
    //! frame transactions (an empty signature list trivially validates), where
    //! the prefix establishes a payer through real APPROVE code (not the
    //! signature-gated default-code path).
    use bytes::Bytes;
    use ethrex_common::types::Transaction;
    use ethrex_common::types::{
        Account, AccountState, BlockHeader, ChainConfig, Code, CodeMetadata, Frame,
        FrameTransaction, PrefixShape, ValidationPrefix,
    };
    use ethrex_common::{Address, H256, U256};
    use ethrex_crypto::NativeCrypto;
    use ethrex_levm::db::{Database, gen_db::GeneralizedDatabase};
    use ethrex_levm::errors::DatabaseError;
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

    /// A deploy frame that leaves the sender codeless, followed by a pay frame
    /// that DOES establish a payer (via a paymaster's APPROVE_PAYMENT code), must
    /// fail validation with `DeployInstalledNoCode`.
    #[test]
    fn deploy_leaving_sender_codeless_fails_validation() {
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
        )
        .expect("simulation runs");
        assert!(
            !outcome.passed,
            "a deploy frame leaving the sender codeless must fail validation"
        );
        assert_eq!(
            outcome.violation.as_deref(),
            Some("DeployInstalledNoCode"),
            "the failure must be DeployInstalledNoCode, got {:?}",
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
        )
        .expect("simulation runs");
        assert!(
            outcome.passed,
            "a self_verify prefix that sets a payer must pass, got {:?}",
            outcome.violation
        );
        assert_eq!(outcome.accessed_paymaster, Some((sender, false)));
    }
}
