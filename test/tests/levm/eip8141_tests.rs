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
    Account, BlockHeader, Code, FRAME_RECEIPT_STATUS_FAILURE, FRAME_RECEIPT_STATUS_SKIPPED,
    FRAME_RECEIPT_STATUS_SUCCESS, Fork, Frame, FrameMode, FrameTransaction, Transaction,
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
/// Harness base fee. `frame_tx_with_frames` sets a `max_fee` far above what it requires.
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
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(HARNESS_CHAIN_ID),
        base_fee_per_gas: U256::from(HARNESS_BASE_FEE),
        // What `GASPRICE` reports, derived from the fee budget like production.
        gas_price: tx
            .fee_budget(U256::from(HARNESS_BASE_FEE), U256::zero())
            .map_or(U256::from(HARNESS_BASE_FEE), |budget| budget.gas_price),
        tx_nonce: tx.nonce_seq,
        ..Default::default()
    }
}

/// Build a frame transaction wrapping `frames`, with sane harness defaults:
/// harness chain id, nonce 0, sender = `FUNDED_SENDER` and a fee budget far above
/// what the harness base fee requires.
fn frame_tx_with_frames(frames: Vec<Frame>) -> FrameTransaction {
    FrameTransaction {
        chain_id: HARNESS_CHAIN_ID,
        nonce_keys: vec![],
        nonce_seq: 0,
        sender: FUNDED_SENDER,
        frames,
        max_fee: U256::from(1_000_000_000_000_000_000u64),
        max_priority_fee_per_gas: U256::from(1u64),
        blob_versioned_hashes: Vec::new(),
        inner_hash: Default::default(),
        cached_canonical: Default::default(),
        stripped_proofs: Default::default(),
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
/// instead of `HARNESS_BASE_FEE`. The env's `gas_price` is derived from the fee
/// budget the same way production does. Used by fee tests that need a real
/// base-fee/priority-fee spread.
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
    env.gas_price = tx
        .fee_budget(env.base_fee_per_gas, env.base_blob_fee_per_gas)
        .map_or(env.base_fee_per_gas, |budget| budget.gas_price);
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
        state_gas_limit: 1_000_000,
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
        state_gas_limit: 1_000_000,
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
            state_gas_limit: 1_000_000,
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
            state_gas_limit: 1_000_000,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);
    // 100 gwei for every unit of `max_gas`: the budget covers the whole tip.
    tx.max_fee = U256::from(100_000_000_000u64) * U256::from(tx.max_gas());
    tx.max_priority_fee_per_gas = U256::from(2_000_000_000u64); // 2 gwei
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
            // EIP-8037 (active at Hegota): the new-slot SSTORE spills
            // STATE_BYTES_PER_STORAGE_SET * cost_per_state_byte (~98k) into
            // the frame's regular gas, so the budget must cover it.
            gas_limit: 300_000,
            state_gas_limit: 1_000_000,
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

/// FRAMEPARAM(param=0x05, frameIndex=3) -> status of frame[3], then SSTORE at slot 0.
/// FRAMEPARAM takes frameIndex from the top of the stack and param below it, so 0x03 (the
/// frameIndex) is pushed last.
/// Bytecode: PUSH1 0x05 (param), PUSH1 0x03 (frameIndex), FRAMEPARAM (0xB3),
///           PUSH1 0x00 (slot key), SSTORE (0x55), STOP (0x00).
const FRAMEPARAM_READ_FRAME3_STATUS: &[u8] =
    &[0x60, 0x05, 0x60, 0x03, 0xB3, 0x60, 0x00, 0x55, 0x00];

/// Covers both halves of an atomic-batch revert: a frame the batch never reached
/// reports `0x2` SKIPPED and is refunded, and a frame that executed before the
/// failure keeps the status and gas it earned, losing only its logs. Both feed the
/// block header `gasUsed` and the receipts trie, so each assertion here is
/// consensus-visible.
#[test]
fn frameparam_status_of_skipped_frame_is_two() {
    // EIP-8141: a frame skipped by a failed atomic batch reports status 0x2
    // (not 0x3). Layout:
    //   frame0: VERIFY on the sender -> APPROVE(3) -> payer + sender_approved.
    //   frame1: DEFAULT, atomic-batch flag, succeeds -> rolled back with the batch.
    //   frame2: DEFAULT, atomic-batch flag, reverts -> unrolls the batch.
    //   frame3: DEFAULT, atomic-batch flag -> SKIPPED.
    //   frame4: DEFAULT, no flag -> batch terminator, also SKIPPED.
    //   frame5: DEFAULT, no flag -> reads FRAMEPARAM(0x05, 3) and stores it.
    const ROLLED_BACK_FRAME_GAS_LIMIT: u64 = 30_000;
    let reverter = Address::from_low_u64_be(0xD1);
    let stop_ct = Address::from_low_u64_be(0xD2);
    let reader = Address::from_low_u64_be(0xD3);
    // Emits an empty LOG0 and stops, so the rolled-back frame has both gas and a log
    // to account for: PUSH1 0x00, PUSH1 0x00, LOG0, STOP.
    let logger = Address::from_low_u64_be(0xD4);
    const LOGGER_CODE: &[u8] = &[0x60, 0x00, 0x60, 0x00, 0xA0, 0x00];
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0x04,
            target: Some(logger),
            gas_limit: ROLLED_BACK_FRAME_GAS_LIMIT,
            state_gas_limit: 1_000_000,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0x04,
            target: Some(reverter),
            gas_limit: 60_000,
            state_gas_limit: 1_000_000,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0x04,
            target: Some(stop_ct),
            gas_limit: 30_000,
            state_gas_limit: 1_000_000,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0x00,
            target: Some(stop_ct),
            gas_limit: 30_000,
            state_gas_limit: 1_000_000,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0x00,
            target: Some(reader),
            // EIP-8037 (active at Hegota): the new-slot SSTORE spills
            // STATE_BYTES_PER_STORAGE_SET * cost_per_state_byte (~98k) into
            // the frame's regular gas, so the budget must cover it.
            gas_limit: 300_000,
            state_gas_limit: 1_000_000,
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
        (logger, U256::zero(), 0, Bytes::from(LOGGER_CODE.to_vec())),
        (
            reader,
            U256::zero(),
            0,
            Bytes::from(FRAMEPARAM_READ_FRAME3_STATUS.to_vec()),
        ),
    ];
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("a DEFAULT-frame batch revert must not invalidate the tx");

    let frame_results = report
        .frame_results
        .expect("frame tx report must carry per-frame results");
    assert_eq!(
        frame_results[3].0, FRAME_RECEIPT_STATUS_SKIPPED,
        "frame[3] must be recorded as skipped"
    );
    assert_eq!(
        frame_results[4].0, FRAME_RECEIPT_STATUS_SKIPPED,
        "the batch terminator is part of the skip"
    );
    // Skipped frames are refunded, and `gas_used` here feeds the block header's
    // `gasUsed`, so the zero is consensus-visible.
    assert_eq!(frame_results[3].1, 0, "a skipped frame consumes no gas");
    assert_eq!(frame_results[4].1, 0);
    // EIP-8141: "Those frame receipts retain their execution status and gas used,
    // with empty logs." frame[1] ran and succeeded before the batch failed, so it
    // keeps SUCCESS and the gas it actually used — charging its full gas_limit would
    // bill the payer for gas the transaction never spent, and bills a frame
    // differently inside a batch than outside one.
    assert_eq!(
        frame_results[1].0,
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "a frame that succeeded before the batch failed keeps its status"
    );
    assert!(
        frame_results[1].1 > 0 && frame_results[1].1 < ROLLED_BACK_FRAME_GAS_LIMIT,
        "a rolled-back batch frame keeps the gas it used, not its full limit: {}",
        frame_results[1].1
    );
    assert!(
        frame_results[1].3.is_empty(),
        "logs written before the failure are discarded with the batch state"
    );
    // The failing frame reverted, so it is charged what it used, exactly as it would
    // be outside a batch.
    assert!(
        frame_results[2].1 < 60_000,
        "a REVERTing frame is charged its actual gas inside a batch too: {}",
        frame_results[2].1
    );

    // FRAMEPARAM(0x05) must surface the same code, and it must be 2.
    assert_eq!(
        storage_slot(&db, reader, H256::zero()),
        U256::from(2u64),
        "FRAMEPARAM param 0x05 must return 2 for a frame skipped by a failed atomic batch"
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
        state_gas_limit: 1_000_000,
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

// ==================== Batched VERIFY revert invalidates tx ====================

#[test]
fn atomic_batch_flag_on_a_verify_frame_is_a_format_rejection() {
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
            state_gas_limit: 1_000_000,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0x00,
            target: Some(stop_ct),
            gas_limit: 30_000,
            state_gas_limit: 1_000_000,
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
    // EIP-8141 forbids the atomic batch flag on a `VERIFY` frame, so this
    // transaction never reaches the batch at all: it is rejected by static
    // validation. The assertion used to read `InvalidFrameTransaction` and so
    // passed for the wrong reason -- the reverting frame below is never
    // executed, and a batched `VERIFY` revert is not constructible while the
    // flag is forbidden.
    let (result, db) = run_frame_tx(&accounts, tx);
    match result {
        Err(VMError::TxValidation(
            ethrex_levm::errors::TxValidationError::InvalidFrameTransactionFormat(ref reason),
        )) => assert!(
            reason.contains("atomic batch flag"),
            "expected the atomic-batch-flag rule, got {reason:?}"
        ),
        ref other => panic!("expected InvalidFrameTransactionFormat, got {other:?}"),
    }
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
            state_gas_limit: 1_000_000,
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
                ethrex_levm::errors::TxValidationError::InvalidFrameTransaction
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
            state_gas_limit: 1_000_000,
            value,
            data: Bytes::new(),
        },
    ]);
    // Seeded with a balance so the recipient is already alive: reviving a dead
    // account is a separate, priced case (`sender_frame_reviving_a_dead_target_pays_new_account`).
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (eoa, U256::one(), 0, Bytes::new()),
    ];
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
    assert_eq!(
        balance_of(&db, eoa),
        value.saturating_add(U256::one()),
        "value not delivered to EOA"
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
            state_gas_limit: 1_000_000,
            value,
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
        (eoa, U256::one(), 0, Bytes::new()),
    ];
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
        frame_results[1].3.iter().any(is_transfer_log),
        "EIP-7708 transfer log missing from frame_receipts[1].logs: {:?}",
        frame_results[1].3
    );
    // ...and in the aggregated report logs (eth_getLogs / RPC).
    assert!(
        report.logs.iter().any(is_transfer_log),
        "EIP-7708 transfer log missing from report.logs"
    );
}

/// A SENDER frame whose value transfer revives a dead account pays the EIP-8037
/// NEW_ACCOUNT state cost at entry, from its own gas limit and before it runs
/// (EELS `charge_value_transfer_to_non_alive_account`). A frame that cannot
/// afford the charge never executes and forfeits its whole gas limit.
///
/// Found on a devnet, not by a fixture: ethrex charged nothing here and billed
/// the frame 3_000 while Nethermind billed it the full limit, so the two split
/// on the header `gasUsed` of the first frame transaction that paid a fresh
/// address.
#[test]
fn sender_frame_reviving_a_dead_target_pays_new_account() {
    let dead = Address::from_low_u64_be(0xDEAD1); // never seeded: not alive
    let value = U256::from(5_000_000u64);
    let accounts = [(
        FUNDED_SENDER,
        AUTO_SEED_SENDER_BALANCE,
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )];

    // EIP-8141 budgets the revival charge in the state dimension, so the frame is
    // refused entry on its `limits.state`, not on its execution gas. The closure
    // therefore varies the state budget and keeps execution comfortable.
    let sender_frame = |state_gas_limit: u64| Frame {
        mode: u8::from(FrameMode::Sender),
        flags: 0,
        target: Some(dead),
        gas_limit: 1_000_000,
        state_gas_limit,
        value,
        data: Bytes::new(),
    };

    // A frame limit that covers a cold access but not the account creation.
    let tx = frame_tx_with_frames(vec![verify_frame(FUNDED_SENDER), sender_frame(50_000)]);
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("the transaction stays valid; only the frame fails");
    let frame_results = report.frame_results.expect("frame results present");
    assert_eq!(
        frame_results[1].0,
        ethrex_common::types::FRAME_RECEIPT_STATUS_FAILURE,
        "a frame that cannot pay its entry charge must fail"
    );
    assert_eq!(
        frame_results[1].1, 1_000_000,
        "a frame that fails at entry forfeits its whole execution gas limit"
    );
    assert_eq!(
        frame_results[1].2, 0,
        "and is attributed no state gas, having created nothing"
    );
    assert_eq!(
        balance_of(&db, dead),
        U256::zero(),
        "the account must not be revived by a frame that never ran"
    );

    // The same frame with room for the charge runs, and is billed exactly the
    // cold access plus the NEW_ACCOUNT state cost.
    let tx = frame_tx_with_frames(vec![verify_frame(FUNDED_SENDER), sender_frame(10_000_000)]);
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("the funded frame must succeed");
    let frame_results = report.frame_results.expect("frame results present");
    assert_eq!(
        frame_results[1].0,
        ethrex_common::types::FRAME_RECEIPT_STATUS_SUCCESS,
        "a frame that can pay the revival charge must succeed"
    );
    assert_eq!(
        balance_of(&db, dead),
        value,
        "the revived account must receive the value"
    );
    // The two charges land in their own dimensions: the target's cold access is
    // execution gas, and reviving it is EIP-8037 state gas.
    let cold_access = ethrex_levm::gas_cost::cold_account_access_cost(Fork::Hegota);
    assert_eq!(
        frame_results[1].1, cold_access,
        "the frame's execution gas is the cold access alone; the revival is state gas"
    );
    assert!(
        frame_results[1].2 > 0,
        "the frame must be billed a NEW_ACCOUNT charge in the state dimension"
    );
    assert_eq!(
        report.state_gas_used, frame_results[1].2,
        "the transaction's state gas is what the reviving frame was attributed"
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
            state_gas_limit: 1_000_000,
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
        frame_results[1].0, FRAME_RECEIPT_STATUS_SUCCESS,
        "SENDER frame (index 1) must be reported as success"
    );
    assert!(
        frame_results[1].3.iter().any(|l| l.address == worker),
        "log from worker missing from frame_results[1].logs"
    );
    assert!(
        frame_results[0].3.is_empty(),
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
            state_gas_limit: 1_000_000,
            value: U256::zero(),
            data: Bytes::new(),
        },
        // Frame 2: SENDER to worker_b — emits LOG0 at worker_b.
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(worker_b),
            gas_limit: 100_000,
            state_gas_limit: 1_000_000,
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
        frame_results[1].3.len(),
        1,
        "worker_a frame must carry exactly one log, got {:?}",
        frame_results[1].3
    );
    assert!(
        frame_results[1].3.iter().all(|l| l.address == worker_a),
        "worker_a frame receipt must contain only worker_a's log"
    );
    assert_eq!(
        frame_results[2].3.len(),
        1,
        "worker_b frame must carry exactly one log (the bug leaked worker_a's log \
         in here), got {:?}",
        frame_results[2].3
    );
    assert!(
        frame_results[2].3.iter().all(|l| l.address == worker_b),
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

#[test]
fn frame_sstore_set_reports_eip8037_state_gas() {
    // Comfortably above SSTORE_SET_STATE_GAS so the charge fits its own dimension.
    let frame_state_budget: u64 = 200_000;
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
            state_gas_limit: frame_state_budget,
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
    // EIP-8141 gave a frame two independent budgets, so `gas_used` is the execution
    // dimension alone and the state figure is reported beside it rather than inside
    // it. The old single-budget model spilled state gas into the frame's gas, which
    // is why this used to assert that the total exceeded the state portion; under
    // the split there is no arithmetic relation between the two to assert.
    assert!(
        report.gas_used > 0,
        "the frame still consumed execution gas for the SSTORE itself"
    );
    assert!(
        report.gas_used < frame_state_budget,
        "execution gas {} must not include the state charge, which is budgeted \
         separately ({} of state gas against a {} state budget)",
        report.gas_used,
        report.state_gas_used,
        frame_state_budget,
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
            state_gas_limit: 1_000_000,
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
}

#[test]
fn frame_tx_budget_must_cover_the_blob_fee() {
    // EIP-8141 §Fee budget: `max_fee` pays for the blob gas at the block's blob base
    // fee as well as `max_gas` at the base fee, and the transaction is invalid when it
    // covers less. The check fires before any frame executes, so a lone SENDER frame
    // is enough to reach it.
    let run = |extra_fee: u64| {
        let mut tx = frame_tx_with_frames(vec![Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0x00,
            target: None,
            gas_limit: 100_000,
            state_gas_limit: 1_000_000,
            value: U256::zero(),
            data: Bytes::new(),
        }]);
        tx.blob_versioned_hashes = vec![H256::repeat_byte(0x01)];
        // `max_gas` at the base fee, plus `extra_fee` towards the blob gas.
        tx.max_fee = U256::from(tx.max_gas() * HARNESS_BASE_FEE + extra_fee);

        let mut db = seeded_db(&[(FUNDED_SENDER, AUTO_SEED_SENDER_BALANCE, 0, Bytes::new())]);
        let mut env = frame_tx_env(&tx);
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
        vm.execute()
    };
    let insufficient = |result: &Result<ExecutionReport, VMError>| {
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::InsufficientMaxFeePerGas
            ))
        )
    };

    // One blob is `GAS_PER_BLOB` (131072) blob gas at a blob base fee of 1.
    let short = run(131_071);
    assert!(
        insufficient(&short),
        "a budget one wei short of the blob fee must be rejected, got {short:?}"
    );
    let exact = run(131_072);
    assert!(
        !insufficient(&exact),
        "a budget covering the blob fee exactly passes the fee check, got {exact:?}"
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
        state_gas_limit: 0,
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
        fr[2].1 > SSTORE_SET_STATE_GAS,
        "frame B gas_used ({}) must include the full state-set cost — frame A's \
         reservoir credit must not subsidize it",
        fr[2].1,
    );
}

// ==================== frame_tx opcode handler unit tests ====================
// (migrated from crates/vm/levm/src/opcode_handlers/frame_tx.rs)

mod frame_tx_opcode_handler_tests {
    use bytes::Bytes;
    use ethrex_common::types::{Frame, FrameMode, FrameTransaction};
    use ethrex_common::{Address, U256};
    use ethrex_levm::errors::{InternalError, VMError};
    use ethrex_levm::opcode_handlers::frame_tx::{load_tx_param, u256_to_offset};
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
            state_gas_limit: 1_000_000,
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

    /// A minimal `FrameTxContext` over a transaction with `proofs` leading proof
    /// frames followed by a VERIFY frame.
    fn ctx_with_proof_frames(proofs: usize) -> FrameTxContext {
        let mut tx = FrameTransaction::default();
        for _ in 0..proofs {
            tx.frames.push(Frame {
                mode: u8::from(FrameMode::Proof),
                data: Bytes::from(vec![0x01u8; 64]),
                ..Default::default()
            });
        }
        tx.frames.push(Frame {
            mode: u8::from(FrameMode::Verify),
            ..Default::default()
        });
        FrameTxContext {
            sender_approved: false,
            payer_address: None,
            frame_results: Vec::new(),
            current_frame_index: 0,
            outstanding_charge_owners: Default::default(),
            journal: Vec::new(),
            sig_hash: ethrex_common::H256::zero(),
            tx,
            approve_called_in_current_frame: false,
            max_gas: 0,
            legacy_sender_nonce: 0,
            fee_budget: ethrex_common::types::FrameFeeBudget {
                max_cost: U256::zero(),
                gas_price: U256::zero(),
            },
        }
    }

    #[test]
    fn txparam_0x0b_returns_the_proof_frame_count() {
        let ctx = ctx_with_proof_frames(2);
        let result = load_tx_param(&ctx, 0x0B).unwrap();
        assert_eq!(result, U256::from(2u64));
        // `TXPARAM_FRAME_COUNT` counts them among the frames.
        assert_eq!(load_tx_param(&ctx, 0x09).unwrap(), U256::from(3u64));
    }

    #[test]
    fn txparam_0x0b_is_zero_without_proof_frames() {
        let ctx = ctx_with_proof_frames(0);
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
            state_gas_limit: 1_000_000,
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
        assert_eq!(u8::from(Opcode::GASPRICE), 0x3A);
        assert_eq!(u8::from(Opcode::BLOCKHASH), 0x40);
        assert_eq!(u8::from(Opcode::COINBASE), 0x41);
        assert_eq!(u8::from(Opcode::TIMESTAMP), 0x42);
        assert_eq!(u8::from(Opcode::NUMBER), 0x43);
        assert_eq!(u8::from(Opcode::PREVRANDAO), 0x44);
        assert_eq!(u8::from(Opcode::GASLIMIT), 0x45);
        assert_eq!(u8::from(Opcode::BASEFEE), 0x48);
        assert_eq!(u8::from(Opcode::BLOBBASEFEE), 0x4A);
        assert_eq!(u8::from(Opcode::SLOTNUM), 0x4B);
        assert_eq!(u8::from(Opcode::INVALID), 0xFE);
        assert_eq!(u8::from(Opcode::SELFDESTRUCT), 0xFF);
        assert_eq!(u8::from(Opcode::BALANCE), 0x31);
        assert_eq!(u8::from(Opcode::SELFBALANCE), 0x47);
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
            nonce_keys: vec![],
            nonce_seq: 0,
            sender,
            frames,
            max_fee: U256::zero(),
            max_priority_fee_per_gas: U256::zero(),
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
            state_gas_limit: 1_000_000,
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
            state_gas_limit: 1_000_000,
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
            None,
        )
        .unwrap();
        let result = vm
            .run_frame_validation_prefix(frame_indices, deploy_index, None, None)
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
        // EIP-8141: an expiry verifier frame declares no state budget.
        let expiry_frame = Frame {
            state_gas_limit: 0,
            ..verify_frame_obs(expiry, 50_000, 0x00, data)
        };
        let tx = frame_tx_for_obs(sender, vec![expiry_frame]);
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
        let (_result, violation) = run(&tx, &mut db, sender, &[0], None);
        assert_eq!(
            violation,
            Some(FrameSimViolation::BannedOpcode(0x4B)),
            "SLOTNUM must be a banned opcode during prefix simulation"
        );
    }

    /// EIP-8141 relaxed the banned list: `ORIGIN` (0x32), `BLOBHASH` (0x49),
    /// `TLOAD` (0x5C) and `TSTORE` (0x5D) were banned originally and are not any
    /// more. None of them can make a prefix pass at admission and then fail in a
    /// block, which is the whole point of the list: `ORIGIN` is fixed per frame by
    /// the mode, `BLOBHASH` reads the transaction's own versioned hashes, and
    /// transient storage cannot outlive the transaction that wrote it.
    ///
    /// Kept as a test rather than only a comment because the ban is one match arm
    /// away: re-adding any of the four would silently start rejecting valid frame
    /// transactions at admission, which no consensus fixture can catch.
    #[test]
    fn the_relaxed_opcodes_are_not_banned() {
        for (opcode, name) in [
            (0x32u8, "ORIGIN"),
            (0x49u8, "BLOBHASH"),
            (0x5Cu8, "TLOAD"),
            (0x5Du8, "TSTORE"),
        ] {
            let sender = addr(0x3200);
            // The opcode, then POP and STOP: TLOAD/BLOBHASH/ORIGIN each push one
            // word, and TSTORE needs two operands, so push two zeros up front and
            // let the extra word be popped.
            let code = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, opcode, 0x50, 0x00]);
            let tx = frame_tx_for_obs(
                sender,
                vec![verify_frame_obs(sender, 50_000, 0x03, Bytes::new())],
            );
            let mut db = build_db(vec![(sender, account_with_code(0, code))]);
            let (_result, violation) = run(&tx, &mut db, sender, &[0], None);
            assert!(
                !matches!(violation, Some(FrameSimViolation::BannedOpcode(op)) if op == opcode),
                "{name} ({opcode:#04x}) must not be reported as a banned opcode"
            );
        }
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
            // Payment-only scope: a frame targeting a non-sender account may not
            // ask for APPROVE_EXECUTION.
            vec![verify_frame_obs(other, 100_000, 0x01, Bytes::new())],
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
        FRAME_TX_MAX_VERIFY_GAS, Frame, FrameTransaction, PrefixShape, ValidationPrefix,
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
            Ok(self.chain_config.clone())
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
            state_gas_limit: 100_000,
            value: U256::zero(),
            data: Bytes::new(),
        }
    }

    fn frame_tx_prefix(sender: Address, frames: Vec<Frame>) -> Transaction {
        Transaction::FrameTransaction(FrameTransaction {
            chain_id: 0,
            nonce_keys: vec![],
            nonce_seq: 0,
            sender,
            frames,
            max_fee: U256::zero(),
            max_priority_fee_per_gas: U256::zero(),
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
            recent_root_index: None,
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
            recent_root_index: None,
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
        )
        .expect("simulation runs");
        assert!(
            outcome.passed,
            "a self_verify prefix that sets a payer must pass, got {:?}",
            outcome.violation
        );
        assert_eq!(outcome.accessed_paymaster, Some((sender, false)));
    }

    /// The paymaster reservation is the transaction's `max_fee`, not the `max_cost`
    /// APPROVE collects at the head block's base fees. Admission simulates against
    /// the current head while execution charges the base fees of whichever later
    /// block includes the transaction, and both base fees move per block. EIP-8141
    /// bounds `max_cost` by `max_fee` at any base fees, so `max_fee` is the ceiling;
    /// the head's `max_cost` is not.
    #[test]
    fn reservation_ceiling_is_the_max_fee() {
        const MAX_FEE: u64 = 1_000_000_000;

        let sender = addr(0x5E_11_02);
        let mut tx = frame_tx_prefix(sender, vec![frame(1, 0x03, sender, 50_000)]);
        let Transaction::FrameTransaction(frame_tx) = &mut tx else {
            unreachable!("frame_tx_prefix builds a frame transaction")
        };
        frame_tx.max_fee = U256::from(MAX_FEE);
        frame_tx.blob_versioned_hashes = vec![H256([
            0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0,
        ])];

        // Enough balance to cover the `max_cost` APPROVE collects at the head's base
        // fees, which is far below `max_fee`.
        let mut db = db_with(vec![(sender, account(1_000_000, approve_code(0x03)))]);
        let prefix = ValidationPrefix {
            shape: PrefixShape::SelfVerify,
            frame_indices: vec![0],
            deploy_index: None,
            pay_index: Some(0),
            recent_root_index: None,
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
        )
        .expect("simulation runs");
        assert!(
            outcome.passed,
            "the blob-carrying self_verify prefix must pass, got {:?}",
            outcome.violation
        );
        assert_eq!(
            outcome.reservation_ceiling,
            U256::from(MAX_FEE),
            "the reservation must be the transaction's max_fee"
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
            outstanding_charge_owners: Default::default(),
            journal: Vec::new(),
            sig_hash: ethrex_common::H256::zero(),
            tx: ethrex_common::types::FrameTransaction::default(),
            approve_called_in_current_frame: false,
            max_gas: 0,
            legacy_sender_nonce: 0,
            fee_budget: ethrex_common::types::FrameFeeBudget {
                max_cost: U256::zero(),
                gas_price: U256::zero(),
            },
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

// ==================== Relocated from crates/vm/system_contracts.rs ====================
mod expiry_verifier_tests {
    use ethrex_common::H160;
    use ethrex_vm::system_contracts::EXPIRY_VERIFIER_RUNTIME_BYTECODE;

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
            ethrex_common::types::frame_tx_expiry_verifier(),
            H160(hex_literal::hex!(
                "81413f0cF12e9b6a49B1D0439E081c577D57FfFf"
            ))
        );
    }
}

// ==================== Verifier precompiles ====================

/// The three verifier precompiles EIP-8141 adds, called directly: the input
/// they take, what they return, and the gas they consume when the claim holds
/// and when it does not.
mod verifier_precompile_tests {
    use bytes::Bytes;
    use ethrex_common::types::{
        FRAME_TX_LEANSPHINCS_VERIFIER, FRAME_TX_P256_VERIFIER, FRAME_TX_SECP256K1_VERIFIER, Fork,
        LEANSPHINCS_VERIFIER_GAS, P256_VERIFIER_GAS, SECP256K1_VERIFIER_GAS,
    };
    use ethrex_common::{Address, H256};
    use ethrex_levm::errors::VMError;
    use ethrex_levm::precompiles::{
        is_precompile, leansphincs_verifier, p256_verifier, secp256k1_verifier,
    };
    use ethrex_levm::vm::VMType;

    type Verifier =
        fn(&Bytes, &mut u64, Fork, &dyn ethrex_crypto::Crypto) -> Result<Bytes, VMError>;

    /// Gas given to every call: more than any verifier costs, so a call that
    /// holds leaves the difference and one that fails is told apart by its error.
    const GAS: u64 = 100_000;

    /// The word a verifier precompile returns when its claim holds.
    fn one() -> Bytes {
        let mut word = [0u8; 32];
        word[31] = 1;
        Bytes::copy_from_slice(&word)
    }

    /// Call `verifier` on `input` and return its result with the gas it consumed.
    fn call(verifier: Verifier, input: &[u8]) -> (Result<Bytes, VMError>, u64) {
        let mut gas = GAS;
        let result = verifier(
            &Bytes::copy_from_slice(input),
            &mut gas,
            Fork::Hegota,
            &ethrex_crypto::NativeCrypto,
        );
        (result, GAS - gas)
    }

    fn holds(verifier: Verifier, input: &[u8]) -> bool {
        call(verifier, input).0.is_ok()
    }

    /// `address` left-padded to the 32-byte `key` of a claim.
    fn address_key(address: Address) -> [u8; 32] {
        H256::from(address).0
    }

    /// A secp256k1 key, the address it controls, and the verifier input
    /// `key || msg || v || r || s` for its signature of `msg`.
    fn secp256k1_claim(msg: [u8; 32]) -> (Address, Vec<u8>) {
        use k256::ecdsa::SigningKey;
        let signing_key = SigningKey::from_bytes(&[0x42; 32].into()).unwrap();
        let uncompressed = signing_key.verifying_key().to_encoded_point(false);
        let public_key_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
        let signer = Address::from_slice(&public_key_hash[12..]);
        let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&msg).unwrap();

        let mut input = address_key(signer).to_vec();
        input.extend_from_slice(&msg);
        input.push(recovery_id.to_byte());
        input.extend_from_slice(&signature.to_bytes());
        (signer, input)
    }

    /// A P-256 key and the verifier input `key || msg || r || s || qx || qy` for
    /// its signature of `msg`, with `s` normalized to the low half.
    fn p256_claim(msg: [u8; 32]) -> Vec<u8> {
        use p256::ecdsa::SigningKey;
        use p256::ecdsa::signature::hazmat::PrehashSigner;
        let signing_key = SigningKey::from_bytes(&[0x37; 32].into()).unwrap();
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let public_key = &encoded.as_bytes()[1..65];
        let signature: p256::ecdsa::Signature = signing_key.sign_prehash(&msg).unwrap();
        let signature = signature.normalize_s().unwrap_or(signature);
        let public_key_hash = ethrex_crypto::keccak::keccak_hash(public_key);

        let mut input = address_key(Address::from_slice(&public_key_hash[12..])).to_vec();
        input.extend_from_slice(&msg);
        input.extend_from_slice(&signature.to_bytes());
        input.extend_from_slice(public_key);
        input
    }

    /// A leanSPHINCS key and the verifier input `key || msg || signature`.
    fn leansphincs_claim(msg: [u8; 32]) -> Vec<u8> {
        let secret = [0x51u8; 32];
        let mut input = lean_sphincs::key_gen(&secret).to_bytes().to_vec();
        input.extend_from_slice(&msg);
        input.extend_from_slice(&lean_sphincs::sign(&secret, &msg).to_bytes());
        input
    }

    #[test]
    fn verifier_precompiles_activate_with_hegota() {
        for verifier in [
            FRAME_TX_SECP256K1_VERIFIER,
            FRAME_TX_P256_VERIFIER,
            FRAME_TX_LEANSPHINCS_VERIFIER,
        ] {
            assert!(is_precompile(&verifier, Fork::Hegota, VMType::L1));
            assert!(!is_precompile(&verifier, Fork::Amsterdam, VMType::L1));
        }
    }

    #[test]
    fn secp256k1_verifier_accepts_the_signer_of_the_message() {
        let (_, input) = secp256k1_claim([0x11; 32]);
        assert_eq!(input.len(), 129);
        let (result, gas_used) = call(secp256k1_verifier, &input);
        assert_eq!(result.unwrap(), one());
        // A call that succeeds consumes exactly the verifier's cost.
        assert_eq!(gas_used, SECP256K1_VERIFIER_GAS);
    }

    #[test]
    fn secp256k1_verifier_rejects_what_does_not_prove_the_claim() {
        let (signer, input) = secp256k1_claim([0x11; 32]);

        // Another key, another message, a tampered signature.
        let mut other_key = input.clone();
        other_key[31] ^= 1;
        assert!(!holds(secp256k1_verifier, &other_key));
        let mut other_msg = input.clone();
        other_msg[63] ^= 1;
        assert!(!holds(secp256k1_verifier, &other_msg));
        let mut tampered = input.clone();
        tampered[128] ^= 1;
        assert!(!holds(secp256k1_verifier, &tampered));

        // A key that is not an address left-padded with zeros.
        let mut padded = input.clone();
        padded[0] = 1;
        assert!(!holds(secp256k1_verifier, &padded));
        assert_eq!(&input[12..32], signer.as_bytes());

        // `v` is the bare recovery id: 27 and 28 are not accepted.
        let mut legacy_v = input.clone();
        legacy_v[64] += 27;
        assert!(!holds(secp256k1_verifier, &legacy_v));

        // A zero `r`, and a zero `s`.
        let mut zero_r = input.clone();
        zero_r[65..97].fill(0);
        assert!(!holds(secp256k1_verifier, &zero_r));
        let mut zero_s = input.clone();
        zero_s[97..129].fill(0);
        assert!(!holds(secp256k1_verifier, &zero_s));

        // Any other length.
        assert!(!holds(secp256k1_verifier, &input[..128]));
        let mut longer = input;
        longer.push(0);
        assert!(!holds(secp256k1_verifier, &longer));
        assert!(!holds(secp256k1_verifier, &[]));
    }

    #[test]
    fn secp256k1_verifier_rejects_the_high_s_form_of_a_valid_signature() {
        // Each signature has one encoding: `s` above n / 2 is rejected even though
        // it recovers the same signer with the other recovery id.
        const SECP256K1_N: [u8; 32] = [
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xfe, 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c,
            0xd0, 0x36, 0x41, 0x41,
        ];
        let (_, input) = secp256k1_claim([0x11; 32]);
        let n = ethrex_common::U256::from_big_endian(&SECP256K1_N);
        let s = ethrex_common::U256::from_big_endian(&input[97..129]);
        let mut high_s = input;
        high_s[97..129].copy_from_slice(&(n - s).to_big_endian());
        high_s[64] ^= 1;
        assert!(!holds(secp256k1_verifier, &high_s));
    }

    #[test]
    fn a_verifier_that_fails_consumes_all_the_gas_it_was_given() {
        // A failed call is an error, which the caller turns into a failed call
        // that keeps none of its gas -- whether the gas ran short, the input was
        // malformed or the signature was invalid.
        let (_, input) = secp256k1_claim([0x11; 32]);
        let mut short_gas = SECP256K1_VERIFIER_GAS - 1;
        assert!(
            secp256k1_verifier(
                &Bytes::from(input.clone()),
                &mut short_gas,
                Fork::Hegota,
                &ethrex_crypto::NativeCrypto
            )
            .is_err()
        );
        let mut tampered = input;
        tampered[128] ^= 1;
        assert!(call(secp256k1_verifier, &tampered).0.is_err());
        assert!(call(p256_verifier, &[0u8; 192]).0.is_err());
        assert!(call(leansphincs_verifier, &[0u8; 7804]).0.is_err());
    }

    #[test]
    fn p256_verifier_accepts_the_key_that_signed_the_message() {
        let input = p256_claim([0x22; 32]);
        assert_eq!(input.len(), 192);
        let (result, gas_used) = call(p256_verifier, &input);
        assert_eq!(result.unwrap(), one());
        assert_eq!(gas_used, P256_VERIFIER_GAS);
    }

    #[test]
    fn p256_verifier_rejects_what_does_not_prove_the_claim() {
        let input = p256_claim([0x22; 32]);

        // The key is the address of the public key the proof carries.
        let mut other_key = input.clone();
        other_key[31] ^= 1;
        assert!(!holds(p256_verifier, &other_key));
        let mut other_msg = input.clone();
        other_msg[63] ^= 1;
        assert!(!holds(p256_verifier, &other_msg));
        let mut tampered = input.clone();
        tampered[64] ^= 1;
        assert!(!holds(p256_verifier, &tampered));

        // The high-s form, which `P256VERIFY` itself accepts.
        const SECP256R1_N: [u8; 32] = [
            0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84, 0xf3, 0xb9, 0xca, 0xc2,
            0xfc, 0x63, 0x25, 0x51,
        ];
        let n = ethrex_common::U256::from_big_endian(&SECP256R1_N);
        let s = ethrex_common::U256::from_big_endian(&input[96..128]);
        let mut high_s = input.clone();
        high_s[96..128].copy_from_slice(&(n - s).to_big_endian());
        assert!(!holds(p256_verifier, &high_s));

        assert!(!holds(p256_verifier, &input[..191]));
    }

    #[test]
    fn leansphincs_verifier_accepts_the_key_that_signed_the_message() {
        let input = leansphincs_claim([0x33; 32]);
        assert_eq!(input.len(), 7804);
        let (result, gas_used) = call(leansphincs_verifier, &input);
        assert_eq!(result.unwrap(), one());
        assert_eq!(gas_used, LEANSPHINCS_VERIFIER_GAS);
    }

    #[test]
    fn leansphincs_verifier_rejects_what_does_not_prove_the_claim() {
        let input = leansphincs_claim([0x33; 32]);

        let mut other_key = input.clone();
        other_key[0] ^= 1;
        assert!(!holds(leansphincs_verifier, &other_key));
        let mut other_msg = input.clone();
        other_msg[63] ^= 1;
        assert!(!holds(leansphincs_verifier, &other_msg));
        let mut tampered = input.clone();
        tampered[7803] ^= 1;
        assert!(!holds(leansphincs_verifier, &tampered));

        // The signature has exactly one length.
        assert!(!holds(leansphincs_verifier, &input[..7803]));
        let mut longer = input;
        longer.push(0);
        assert!(!holds(leansphincs_verifier, &longer));
    }
}

// ==================== Proof frames ====================

/// End-to-end proof frame tests: a transaction's proof frames run their
/// verifiers before any other frame, and the claims they settle are read by the
/// frames that follow.
mod proof_frame_tests {
    use super::*;
    use ethrex_common::types::{
        FRAME_TX_LEANSPHINCS_VERIFIER, FRAME_TX_SECP256K1_VERIFIER, LEANSPHINCS_VERIFIER_GAS,
        PROOF_COMMIT_FLAG, PROOF_READ_FLAG, SECP256K1_VERIFIER_GAS,
    };
    use ethrex_levm::errors::TxValidationError;
    use k256::ecdsa::SigningKey;

    /// What a frame pays to reach a warm target, such as a precompile.
    const WARM_ACCESS: u64 = 100;
    /// Address of the contract the reader frames run.
    const READER: Address = Address::repeat_byte(0xB4);
    /// Address of a contract verifier.
    const VERIFIER: Address = Address::repeat_byte(0x7E);

    /// Read slot `slot` of `addr` from the post-execution cache.
    fn storage_of(db: &GeneralizedDatabase, addr: Address, slot: u64) -> U256 {
        db.current_accounts_state
            .get(&addr)
            .and_then(|account| account.storage.get(&H256::from_low_u64_be(slot)).copied())
            .unwrap_or_default()
    }

    /// A secp256k1 key and the address it controls.
    fn key_and_address(seed: u8) -> (SigningKey, Address) {
        let signing_key = SigningKey::from_bytes(&[seed; 32].into()).unwrap();
        let uncompressed = signing_key.verifying_key().to_encoded_point(false);
        let public_key_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
        (signing_key, Address::from_slice(&public_key_hash[12..]))
    }

    /// A proof frame for `verifier` claiming that `key` authorizes `msg`, a zero
    /// `msg` standing for the canonical signature hash, with `proof` as given.
    fn proof_frame(verifier: Address, key: H256, msg: H256, proof: &[u8], gas_limit: u64) -> Frame {
        let mut data = key.as_bytes().to_vec();
        data.extend_from_slice(msg.as_bytes());
        data.extend_from_slice(proof);
        Frame {
            mode: u8::from(FrameMode::Proof),
            flags: 0,
            target: Some(verifier),
            gas_limit,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::from(data),
        }
    }

    /// A proof frame for the secp256k1 verifier with a placeholder proof of the
    /// right length. The proof is elided from the signature hash, so it is filled
    /// in by [`sign_secp256k1_proof`] once the hash is known.
    fn secp256k1_proof_frame(signer: Address, msg: H256) -> Frame {
        proof_frame(
            FRAME_TX_SECP256K1_VERIFIER,
            H256::from(signer),
            msg,
            &[0u8; 65],
            SECP256K1_VERIFIER_GAS + WARM_ACCESS,
        )
    }

    /// Replace the proof of the secp256k1 proof frame at `index` by `key`'s
    /// signature of the message the frame claims.
    fn sign_secp256k1_proof(tx: &mut FrameTransaction, index: usize, key: &SigningKey) {
        let claimed = tx.frames[index].proof_msg();
        let msg = if claimed.is_zero() {
            tx.compute_sig_hash()
        } else {
            claimed
        };
        let (signature, recovery_id) = key.sign_prehash_recoverable(msg.as_bytes()).unwrap();
        let mut data = tx.frames[index].proof_claim().to_vec();
        data.push(recovery_id.to_byte());
        data.extend_from_slice(&signature.to_bytes());
        tx.frames[index].data = Bytes::from(data);
        tx.inner_hash = Default::default();
        tx.cached_canonical = Default::default();
    }

    /// A DEFAULT frame running the code at `READER`.
    fn reader_frame() -> Frame {
        Frame {
            mode: u8::from(FrameMode::Default),
            flags: 0,
            target: Some(READER),
            gas_limit: 200_000,
            state_gas_limit: 1_000_000,
            value: U256::zero(),
            data: Bytes::new(),
        }
    }

    /// `PROOFPARAM(param, index)` then `SSTORE(0, result)`. The operands are
    /// `[param, frameIndex]` with the index on top, so `param` is pushed first.
    fn proofparam_code(param: u8, index: u8) -> Bytes {
        Bytes::from(vec![
            0x60, param, // PUSH1 param
            0x60, index, // PUSH1 frameIndex
            0xB4,  // PROOFPARAM
            0x60, 0x00, // PUSH1 0 (slot)
            0x55, // SSTORE
            0x00, // STOP
        ])
    }

    /// `[proof(s), VERIFY approving both scopes, DEFAULT reader]`, with the sender
    /// a contract that approves unconditionally, so the transaction's validity
    /// depends on its proof frames alone.
    fn run_with_proofs(
        proofs: Vec<Frame>,
        reader_code: Bytes,
        extra_accounts: &[SeededAccount],
    ) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
        let mut frames = proofs;
        frames.push(verify_frame(FUNDED_SENDER));
        frames.push(reader_frame());
        let mut accounts = vec![
            (
                FUNDED_SENDER,
                AUTO_SEED_SENDER_BALANCE,
                0,
                Bytes::from(APPROVE_BOTH_CODE.to_vec()),
            ),
            (READER, U256::zero(), 0, reader_code),
        ];
        accounts.extend_from_slice(extra_accounts);
        run_frame_tx(&accounts, frame_tx_with_frames(frames))
    }

    /// A proof frame whose verifier is the contract at `VERIFIER`, with enough gas
    /// for the cold access and a little execution.
    fn contract_proof_frame() -> Frame {
        proof_frame(
            VERIFIER,
            H256::repeat_byte(0x01),
            H256::zero(),
            &[0xAA; 8],
            50_000,
        )
    }

    /// Run a transaction whose only proof frame targets a verifier with `code`.
    fn run_contract_verifier(code: &[u8]) -> Result<ExecutionReport, VMError> {
        run_with_proofs(
            vec![contract_proof_frame()],
            Bytes::from_static(&[0x00]),
            &[(VERIFIER, U256::zero(), 1, Bytes::copy_from_slice(code))],
        )
        .0
    }

    fn is_invalid_proof(result: &Result<ExecutionReport, VMError>) -> bool {
        matches!(
            result,
            Err(VMError::TxValidation(TxValidationError::InvalidFrameProof))
        )
    }

    #[test]
    fn a_proof_frame_whose_claim_holds_is_charged_its_whole_limit() {
        let (key, signer) = key_and_address(0x42);
        let mut tx = frame_tx_with_frames(vec![
            // 500 gas more than the verifier needs: charged all the same.
            Frame {
                gas_limit: SECP256K1_VERIFIER_GAS + WARM_ACCESS + 500,
                ..secp256k1_proof_frame(signer, H256::zero())
            },
            verify_frame(FUNDED_SENDER),
        ]);
        sign_secp256k1_proof(&mut tx, 0, &key);
        let (result, _db) = run_frame_tx(
            &[(
                FUNDED_SENDER,
                AUTO_SEED_SENDER_BALANCE,
                0,
                Bytes::from(APPROVE_BOTH_CODE.to_vec()),
            )],
            tx,
        );
        let report = result.expect("the claim holds");
        let frame_results = report.frame_results.expect("per-frame results");
        // Like any other frame it yields a receipt: success, its whole execution
        // limit, no state gas and no logs.
        assert_eq!(
            frame_results[0],
            (
                FRAME_RECEIPT_STATUS_SUCCESS,
                SECP256K1_VERIFIER_GAS + WARM_ACCESS + 500,
                0,
                Vec::new()
            )
        );
    }

    #[test]
    fn a_proof_frame_whose_claim_does_not_hold_invalidates_the_transaction() {
        let (key, signer) = key_and_address(0x42);
        let (other_key, _) = key_and_address(0x43);
        let accounts = [(
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        )];

        // Signed by a key the claim does not name.
        let mut wrong_signer = frame_tx_with_frames(vec![
            secp256k1_proof_frame(signer, H256::zero()),
            verify_frame(FUNDED_SENDER),
        ]);
        sign_secp256k1_proof(&mut wrong_signer, 0, &other_key);
        let (result, db) = run_frame_tx(&accounts, wrong_signer);
        assert!(is_invalid_proof(&result), "got {result:?}");
        assert_db_cache_unchanged(&db, &accounts);

        // A valid signature of the signature hash, claimed for another message.
        let mut wrong_msg = frame_tx_with_frames(vec![
            secp256k1_proof_frame(signer, H256::zero()),
            verify_frame(FUNDED_SENDER),
        ]);
        sign_secp256k1_proof(&mut wrong_msg, 0, &key);
        let mut data = wrong_msg.frames[0].data.to_vec();
        data[63] = 1;
        wrong_msg.frames[0].data = Bytes::from(data);
        let (result, _db) = run_frame_tx(&accounts, wrong_msg);
        assert!(is_invalid_proof(&result), "got {result:?}");
    }

    #[test]
    fn a_proof_frame_must_afford_its_verifier() {
        let (key, signer) = key_and_address(0x42);
        let accounts = [(
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        )];
        let run = |gas_limit: u64| {
            let mut tx = frame_tx_with_frames(vec![
                Frame {
                    gas_limit,
                    ..secp256k1_proof_frame(signer, H256::zero())
                },
                verify_frame(FUNDED_SENDER),
            ]);
            sign_secp256k1_proof(&mut tx, 0, &key);
            run_frame_tx(&accounts, tx).0
        };
        // The verifier's cost plus the access of a warm precompile, exactly.
        assert!(run(SECP256K1_VERIFIER_GAS + WARM_ACCESS).is_ok());
        // One gas short of the verifier, and short of reaching it at all.
        assert!(is_invalid_proof(&run(
            SECP256K1_VERIFIER_GAS + WARM_ACCESS - 1
        )));
        assert!(is_invalid_proof(&run(WARM_ACCESS - 1)));
    }

    #[test]
    fn a_proof_of_an_explicit_message_can_be_committed() {
        let (key, signer) = key_and_address(0x42);
        let msg = H256::repeat_byte(0x09);
        let accounts = [(
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        )];
        for flags in [
            0,
            PROOF_COMMIT_FLAG,
            PROOF_READ_FLAG,
            PROOF_COMMIT_FLAG | PROOF_READ_FLAG,
        ] {
            let mut tx = frame_tx_with_frames(vec![
                Frame {
                    flags,
                    ..secp256k1_proof_frame(signer, msg)
                },
                verify_frame(FUNDED_SENDER),
            ]);
            sign_secp256k1_proof(&mut tx, 0, &key);
            assert!(run_frame_tx(&accounts, tx).0.is_ok(), "flags {flags:#x}");
        }
    }

    #[test]
    fn proofparam_reads_the_claim() {
        let (key, signer) = key_and_address(0x42);
        let msg = H256::repeat_byte(0x09);
        for (param, expected) in [
            (0x00, U256::from_big_endian(H256::from(signer).as_bytes())),
            (
                0x01,
                U256::from_big_endian(H256::from(FRAME_TX_SECP256K1_VERIFIER).as_bytes()),
            ),
            (0x02, U256::from_big_endian(msg.as_bytes())),
        ] {
            let mut frames = vec![secp256k1_proof_frame(signer, msg)];
            frames.push(verify_frame(FUNDED_SENDER));
            frames.push(reader_frame());
            let mut tx = frame_tx_with_frames(frames);
            sign_secp256k1_proof(&mut tx, 0, &key);
            let (result, db) = run_frame_tx(
                &[
                    (
                        FUNDED_SENDER,
                        AUTO_SEED_SENDER_BALANCE,
                        0,
                        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
                    ),
                    (READER, U256::zero(), 0, proofparam_code(param, 0)),
                ],
                tx,
            );
            result.expect("valid tx");
            assert_eq!(storage_of(&db, READER, 0), expected, "param {param:#x}");
        }
    }

    #[test]
    fn proofparam_returns_a_zero_msg_for_the_signature_hash() {
        // The claim is read as encoded: the signature hash is not substituted.
        let (result, db) = run_with_proofs(
            vec![contract_proof_frame()],
            // Store a marker first, so a zero result is told apart from a halt.
            Bytes::from(vec![
                0x60, 0x01, 0x60, 0x01, 0x55, // SSTORE(1, 1)
                0x60, 0x02, 0x60, 0x00, 0xB4, // PROOFPARAM(0x02, 0)
                0x60, 0x00, 0x55, 0x00, // SSTORE(0, result); STOP
            ]),
            &[(VERIFIER, U256::zero(), 1, Bytes::from_static(&[0x00]))],
        );
        result.expect("valid tx");
        assert_eq!(storage_of(&db, READER, 1), U256::one());
        assert_eq!(storage_of(&db, READER, 0), U256::zero());
    }

    #[test]
    fn proofparam_halts_outside_a_claim() {
        let verifier = [(VERIFIER, U256::zero(), 1, Bytes::from_static(&[0x00]))];
        for (case, code) in [
            (
                "a frame that is not a proof frame",
                proofparam_code(0x00, 1),
            ),
            ("an out-of-bounds frame", proofparam_code(0x00, 9)),
            ("an undefined param", proofparam_code(0x03, 0)),
        ] {
            let (result, db) = run_with_proofs(vec![contract_proof_frame()], code, &verifier);
            let report = result.expect("only the reader frame halts");
            let frame_results = report.frame_results.expect("per-frame results");
            assert_eq!(frame_results[2].0, FRAME_RECEIPT_STATUS_FAILURE, "{case}");
            assert_eq!(storage_of(&db, READER, 0), U256::zero(), "{case}");
        }
    }

    #[test]
    fn txparam_0x0b_counts_the_proof_frames() {
        // PUSH1 0x0B; TXPARAM; PUSH1 0; SSTORE; STOP
        let code = Bytes::from(vec![0x60, 0x0B, 0xB0, 0x60, 0x00, 0x55, 0x00]);
        let verifier = [(VERIFIER, U256::zero(), 1, Bytes::from_static(&[0x00]))];
        let (result, db) = run_with_proofs(
            vec![contract_proof_frame(), contract_proof_frame()],
            code,
            &verifier,
        );
        result.expect("valid tx");
        assert_eq!(storage_of(&db, READER, 0), U256::from(2u64));
    }

    #[test]
    fn frameparam_reads_a_proof_frame_like_any_other() {
        // FRAMEPARAM(0x04, 0): the data length, which the signature hash commits
        // to, stays observable although the proof's bytes are not.
        let code = Bytes::from(vec![0x60, 0x04, 0x60, 0x00, 0xB3, 0x60, 0x00, 0x55, 0x00]);
        let verifier = [(VERIFIER, U256::zero(), 1, Bytes::from_static(&[0x00]))];
        let (result, db) = run_with_proofs(vec![contract_proof_frame()], code, &verifier);
        result.expect("valid tx");
        assert_eq!(storage_of(&db, READER, 0), U256::from(64u64 + 8));
    }

    /// `FRAMEDATALOAD(offset 64, frame 0)`, then `SSTORE(0, word)` after a marker
    /// in slot 1.
    fn framedataload_proof_code() -> Bytes {
        Bytes::from(vec![
            0x60, 0x01, 0x60, 0x01, 0x55, // SSTORE(1, 1)
            0x60, 0x00, // PUSH1 frameIndex
            0x60, 0x40, // PUSH1 offset
            0xB1, // FRAMEDATALOAD
            0x60, 0x00, 0x55, 0x00, // SSTORE(0, word); STOP
        ])
    }

    #[test]
    fn a_proof_is_unreadable_outside_a_verify_frame() {
        // Even with `PROOF_READ_FLAG`, a DEFAULT frame cannot read the proof.
        let verifier = [(VERIFIER, U256::zero(), 1, Bytes::from_static(&[0x00]))];
        for flags in [0, PROOF_READ_FLAG] {
            let proof = Frame {
                flags,
                ..contract_proof_frame()
            };
            let (result, db) = run_with_proofs(vec![proof], framedataload_proof_code(), &verifier);
            let report = result.expect("only the reader frame halts");
            let frame_results = report.frame_results.expect("per-frame results");
            assert_eq!(frame_results[2].0, FRAME_RECEIPT_STATUS_FAILURE);
            assert_eq!(storage_of(&db, READER, 1), U256::zero());
        }
    }

    /// Account code for a VERIFY frame: approve both scopes if the word at offset
    /// 64 of frame 0's data, its proof, is `0xAA` repeated eight times.
    fn approve_if_proof_is_readable_code() -> Bytes {
        let mut expected = [0u8; 32];
        expected[..8].fill(0xAA);
        let mut code = vec![
            0x60, 0x00, // PUSH1 frameIndex
            0x60, 0x40, // PUSH1 offset
            0xB1, // FRAMEDATALOAD
            0x7F, // PUSH32 expected
        ];
        code.extend_from_slice(&expected);
        code.extend_from_slice(&[
            0x14, // EQ
            0x60, 0x2F, // PUSH1 approve
            0x57, // JUMPI
            0x60, 0x00, 0x60, 0x00, 0xFD, // REVERT(0, 0)
            0x5B, // JUMPDEST (approve, at 0x2F)
            0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA, // APPROVE(0, 0, 3)
        ]);
        assert_eq!(code[0x2F], 0x5B);
        Bytes::from(code)
    }

    #[test]
    fn a_verify_frame_reads_a_proof_that_sets_the_read_flag() {
        let run = |flags: u8| {
            let proof = Frame {
                flags,
                ..contract_proof_frame()
            };
            run_frame_tx(
                &[
                    (
                        FUNDED_SENDER,
                        AUTO_SEED_SENDER_BALANCE,
                        0,
                        approve_if_proof_is_readable_code(),
                    ),
                    (VERIFIER, U256::zero(), 1, Bytes::from_static(&[0x00])),
                ],
                frame_tx_with_frames(vec![proof, verify_frame(FUNDED_SENDER)]),
            )
            .0
        };
        assert!(run(PROOF_READ_FLAG).is_ok());
        // Without the flag the read halts the VERIFY frame, which invalidates the
        // transaction.
        assert!(run(0).is_err());
    }

    #[test]
    fn a_contract_verifier_decides_its_claim() {
        // The claim holds if and only if the call succeeds, whatever it returns.
        assert!(run_contract_verifier(&[0x00]).is_ok());
        assert!(is_invalid_proof(&run_contract_verifier(PURE_REVERT_CODE)));
        // INVALID: an exceptional halt.
        assert!(is_invalid_proof(&run_contract_verifier(&[0xFE])));
    }

    #[test]
    fn a_contract_verifier_is_always_cold() {
        // The frame has its own access sets, so its verifier is charged as cold
        // even when the frame cannot pay for more than a warm access.
        let run = |gas_limit: u64| {
            let proof = Frame {
                gas_limit,
                ..contract_proof_frame()
            };
            run_with_proofs(
                vec![proof],
                Bytes::from_static(&[0x00]),
                &[(VERIFIER, U256::zero(), 1, Bytes::from_static(&[0x00]))],
            )
            .0
        };
        let cold = ethrex_levm::gas_cost::cold_account_access_cost(Fork::Hegota);
        assert!(run(cold).is_ok());
        assert!(is_invalid_proof(&run(cold - 1)));
    }

    #[test]
    fn an_account_without_code_is_not_a_verifier() {
        let (result, _db) = run_with_proofs(
            vec![contract_proof_frame()],
            Bytes::from_static(&[0x00]),
            &[],
        );
        assert!(is_invalid_proof(&result), "got {result:?}");
    }

    #[test]
    fn a_delegated_account_is_not_a_verifier() {
        // Code that is an EIP-7702 delegation indicator can change, so a verifier
        // holding one halts, even though its delegate would accept.
        let delegate = Address::repeat_byte(0xDE);
        let mut indicator = vec![0xEF, 0x01, 0x00];
        indicator.extend_from_slice(delegate.as_bytes());
        let (result, _db) = run_with_proofs(
            vec![contract_proof_frame()],
            Bytes::from_static(&[0x00]),
            &[
                (VERIFIER, U256::zero(), 1, Bytes::from(indicator)),
                (delegate, U256::zero(), 1, Bytes::from_static(&[0x00])),
            ],
        );
        assert!(is_invalid_proof(&result), "got {result:?}");
    }

    #[test]
    fn the_pure_context_halts_on_what_reads_beyond_the_input() {
        for (name, code) in [
            ("ADDRESS", vec![0x30, 0x00]),
            ("BALANCE", vec![0x60, 0x00, 0x31, 0x00]),
            ("ORIGIN", vec![0x32, 0x00]),
            ("CALLER", vec![0x33, 0x00]),
            ("GASPRICE", vec![0x3A, 0x00]),
            ("EXTCODESIZE", vec![0x60, 0x04, 0x3B, 0x00]),
            (
                "EXTCODECOPY",
                vec![0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x04, 0x3C, 0x00],
            ),
            ("EXTCODEHASH", vec![0x60, 0x04, 0x3F, 0x00]),
            ("BLOCKHASH", vec![0x60, 0x00, 0x40, 0x00]),
            ("COINBASE", vec![0x41, 0x00]),
            ("TIMESTAMP", vec![0x42, 0x00]),
            ("NUMBER", vec![0x43, 0x00]),
            ("PREVRANDAO", vec![0x44, 0x00]),
            ("GASLIMIT", vec![0x45, 0x00]),
            ("CHAINID", vec![0x46, 0x00]),
            ("SELFBALANCE", vec![0x47, 0x00]),
            ("BASEFEE", vec![0x48, 0x00]),
            ("BLOBHASH", vec![0x60, 0x00, 0x49, 0x00]),
            ("BLOBBASEFEE", vec![0x4A, 0x00]),
            ("SLOTNUM", vec![0x4B, 0x00]),
            ("SLOAD", vec![0x60, 0x00, 0x54, 0x00]),
            ("TLOAD", vec![0x60, 0x00, 0x5C, 0x00]),
            ("TXPARAM", vec![0x60, 0x00, 0xB0, 0x00]),
            ("FRAMEDATALOAD", vec![0x60, 0x01, 0x60, 0x00, 0xB1, 0x00]),
            (
                "FRAMEDATACOPY",
                vec![0x60, 0x01, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0xB2, 0x00],
            ),
            ("FRAMEPARAM", vec![0x60, 0x02, 0x60, 0x00, 0xB3, 0x00]),
            ("PROOFPARAM", vec![0x60, 0x00, 0x60, 0x00, 0xB4, 0x00]),
            ("APPROVE", vec![0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA]),
            // The seven operands of a CALL to the identity precompile.
            (
                "CALL",
                vec![
                    0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0x04, 0x5A, 0xF1, 0x00,
                ],
            ),
            (
                "CALLCODE",
                vec![
                    0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0x04, 0x5A, 0xF2, 0x00,
                ],
            ),
            (
                "DELEGATECALL",
                vec![
                    0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0x04, 0x5A, 0xF4, 0x00,
                ],
            ),
            // What the STATICCALL restrictions already forbid.
            ("SSTORE", vec![0x60, 0x01, 0x60, 0x00, 0x55, 0x00]),
            ("TSTORE", vec![0x60, 0x01, 0x60, 0x00, 0x5D, 0x00]),
            ("LOG0", vec![0x60, 0x00, 0x60, 0x00, 0xA0, 0x00]),
        ] {
            let result = run_contract_verifier(&code);
            assert!(is_invalid_proof(&result), "{name}: got {result:?}");
        }
    }

    #[test]
    fn the_pure_context_keeps_what_computes_on_the_input() {
        // CALLVALUE (always zero), CALLDATASIZE, CALLDATALOAD, CODESIZE, GAS,
        // KECCAK256 and memory.
        let code = [
            0x34, 0x50, // CALLVALUE; POP
            0x36, 0x50, // CALLDATASIZE; POP
            0x60, 0x00, 0x35, 0x50, // CALLDATALOAD(0); POP
            0x38, 0x50, // CODESIZE; POP
            0x5A, 0x50, // GAS; POP
            0x60, 0x20, 0x60, 0x00, 0x20, 0x50, // KECCAK256(0, 32); POP
            0x00,
        ];
        assert!(run_contract_verifier(&code).is_ok());
    }

    /// Verifier code that succeeds only if its STATICCALL to `callee` does.
    fn staticcall_code(callee: [u8; 2]) -> Vec<u8> {
        vec![
            0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, // ret and args: empty
            0x61, callee[0], callee[1], // PUSH2 callee
            0x5A, 0xFA, // GAS; STATICCALL
            0x60, 0x15, 0x57, // PUSH1 done; JUMPI
            0x60, 0x00, 0x60, 0x00, 0xFD, // REVERT(0, 0)
            0x5B, 0x00, // JUMPDEST (done, at 0x15); STOP
        ]
    }

    #[test]
    fn the_pure_context_allows_a_staticcall_only_to_a_precompile() {
        assert_eq!(staticcall_code([0, 4])[0x15], 0x5B);
        // The identity precompile.
        assert!(run_contract_verifier(&staticcall_code([0x00, 0x04])).is_ok());
        // An account that is not a precompile, with or without code: an
        // exceptional halt, not a failed call the verifier could recover from.
        let other = Address::from_low_u64_be(0x1234);
        let result = run_with_proofs(
            vec![contract_proof_frame()],
            Bytes::from_static(&[0x00]),
            &[
                (
                    VERIFIER,
                    U256::zero(),
                    1,
                    Bytes::from(staticcall_code([0x12, 0x34])),
                ),
                (other, U256::zero(), 1, Bytes::from_static(&[0x00])),
            ],
        )
        .0;
        assert!(is_invalid_proof(&result), "got {result:?}");
    }

    /// Account code of a post-quantum account controlled by the leanSPHINCS key
    /// `public_key`: approve both scopes if frame 0 claims that key, through the
    /// leanSPHINCS verifier, for the canonical signature hash.
    fn leansphincs_account_code(public_key: [u8; 32]) -> Bytes {
        let mut code = vec![
            0x60, 0x01, 0x60, 0x00, 0xB4, // PROOFPARAM(0x01, 0): verifier
            0x60, 0x14, 0x14, // == LEANSPHINCS_VERIFIER
            0x60, 0x00, 0x60, 0x00, 0xB4, // PROOFPARAM(0x00, 0): key
            0x7F, // PUSH32 public_key
        ];
        code.extend_from_slice(&public_key);
        code.extend_from_slice(&[
            0x14, 0x16, // EQ; AND
            0x60, 0x02, 0x60, 0x00, 0xB4, // PROOFPARAM(0x02, 0): msg
            0x15, 0x16, // ISZERO; AND
        ]);
        let approve = u8::try_from(code.len() + 8).unwrap();
        code.extend_from_slice(&[
            0x60, approve, 0x57, // PUSH1 approve; JUMPI
            0x60, 0x00, 0x60, 0x00, 0xFD, // REVERT(0, 0)
            0x5B, // JUMPDEST (approve)
            0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA, // APPROVE(0, 0, 3)
        ]);
        assert_eq!(code[usize::from(approve)], 0x5B);
        Bytes::from(code)
    }

    #[test]
    fn a_leansphincs_account_approves_on_its_claim() {
        let secret = [0x51u8; 32];
        let public_key = lean_sphincs::key_gen(&secret).to_bytes();
        let target = Address::from_low_u64_be(0x7A26);
        let build = || {
            frame_tx_with_frames(vec![
                proof_frame(
                    FRAME_TX_LEANSPHINCS_VERIFIER,
                    H256(public_key),
                    H256::zero(),
                    &[0u8; 7740],
                    LEANSPHINCS_VERIFIER_GAS + WARM_ACCESS,
                ),
                Frame {
                    target: None,
                    ..verify_frame(FUNDED_SENDER)
                },
                Frame {
                    mode: u8::from(FrameMode::Sender),
                    flags: 0,
                    target: Some(target),
                    gas_limit: 100_000,
                    state_gas_limit: 1_000_000,
                    value: U256::zero(),
                    data: Bytes::new(),
                },
            ])
        };
        let sign = |tx: &mut FrameTransaction, secret: &[u8; 32]| {
            let sig_hash = tx.compute_sig_hash();
            let mut data = tx.frames[0].proof_claim().to_vec();
            data.extend_from_slice(
                &lean_sphincs::sign(secret, sig_hash.as_fixed_bytes()).to_bytes(),
            );
            tx.frames[0].data = Bytes::from(data);
        };
        let accounts = [
            (
                FUNDED_SENDER,
                AUTO_SEED_SENDER_BALANCE,
                0,
                leansphincs_account_code(public_key),
            ),
            (
                target,
                U256::zero(),
                1,
                Bytes::from(SSTORE_THEN_STOP_CODE.to_vec()),
            ),
        ];

        let mut tx = build();
        sign(&mut tx, &secret);
        let (result, db) = run_frame_tx(&accounts, tx.clone());
        let report = result.expect("the account's own key signed the transaction");
        assert_eq!(report.payer_address, Some(FUNDED_SENDER));
        assert_eq!(storage_of(&db, target, 0), U256::one());
        // The proof travels in the transaction: 7,740 bytes at the calldata floor
        // dominate what it is charged.
        let floor = tx.calldata_floor_total();
        assert!(floor > 7_740 * 64);
        assert!(report.gas_spent >= floor);

        // Another key's signature proves another claim, which the protocol
        // rejects before the account's code runs.
        let mut forged = build();
        sign(&mut forged, &[0x52u8; 32]);
        let (result, _db) = run_frame_tx(&accounts, forged);
        assert!(is_invalid_proof(&result), "got {result:?}");

        // A valid claim for a key the account does not hold is not approved.
        let other_secret = [0x52u8; 32];
        let mut other = build();
        let mut data = lean_sphincs::key_gen(&other_secret).to_bytes().to_vec();
        data.extend_from_slice(&other.frames[0].data[32..]);
        other.frames[0].data = Bytes::from(data);
        sign(&mut other, &other_secret);
        let (result, _db) = run_frame_tx(&accounts, other);
        assert!(
            result.is_err() && !is_invalid_proof(&result),
            "got {result:?}"
        );
    }
}

// ==================== Default code: the proof frame a VERIFY frame names ====================

/// EIP-8141's default code approves for an account without code when the frame's
/// data is one byte, the index of a proof frame whose verifier is
/// `SECP256K1_VERIFIER`, whose `key` is the account and whose `msg` is zero.
mod default_code_proof_index_tests {
    use super::*;
    use ethrex_common::types::{
        FRAME_TX_P256_VERIFIER, FRAME_TX_SECP256K1_VERIFIER, SECP256K1_VERIFIER_GAS,
    };
    use k256::ecdsa::SigningKey;

    /// A signing key plus the address it authenticates.
    fn key_and_address(seed: u8) -> (SigningKey, Address) {
        let signing_key = SigningKey::from_bytes(&[seed; 32].into()).unwrap();
        let uncompressed = signing_key.verifying_key().to_encoded_point(false);
        let pub_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
        (signing_key, Address::from_slice(&pub_hash[12..]))
    }

    /// A proof frame claiming that `signer` signed the canonical signature hash,
    /// with a placeholder proof of the right length.
    fn secp256k1_proof_frame(signer: Address) -> Frame {
        let mut data = H256::from(signer).as_bytes().to_vec();
        data.extend_from_slice(&[0u8; 32]);
        data.extend_from_slice(&[0u8; 65]);
        Frame {
            mode: u8::from(FrameMode::Proof),
            flags: 0,
            target: Some(FRAME_TX_SECP256K1_VERIFIER),
            gas_limit: SECP256K1_VERIFIER_GAS + 100,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::from(data),
        }
    }

    /// Fill in the proof of the frame at `index` with `key`'s signature of the
    /// message the frame claims.
    fn sign(tx: &mut FrameTransaction, index: usize, key: &SigningKey) {
        let claimed = tx.frames[index].proof_msg();
        let msg = if claimed.is_zero() {
            tx.compute_sig_hash()
        } else {
            claimed
        };
        let (signature, recovery_id) = key.sign_prehash_recoverable(msg.as_bytes()).unwrap();
        let mut data = tx.frames[index].proof_claim().to_vec();
        data.push(recovery_id.to_byte());
        data.extend_from_slice(&signature.to_bytes());
        tx.frames[index].data = Bytes::from(data);
        tx.inner_hash = Default::default();
        tx.cached_canonical = Default::default();
    }

    /// `[PROOF, VERIFY]` from an account without code controlled by a secp256k1
    /// key. `adjust` edits the unsigned transaction.
    fn run_self_verify(
        adjust: impl FnOnce(&mut FrameTransaction),
    ) -> Result<ExecutionReport, VMError> {
        let (key, sender) = key_and_address(0x42);
        let mut tx = frame_tx_with_frames(vec![
            secp256k1_proof_frame(sender),
            Frame {
                target: None,
                data: Bytes::from_static(&[0x00]),
                ..verify_frame(sender)
            },
        ]);
        tx.sender = sender;
        adjust(&mut tx);
        sign(&mut tx, 0, &key);
        run_frame_tx(
            &[(
                sender,
                U256::from(10u64).pow(U256::from(18u64)),
                0,
                Bytes::new(),
            )],
            tx,
        )
        .0
    }

    #[test]
    fn default_code_approves_on_the_proof_frame_its_data_names() {
        let report = run_self_verify(|_| {}).expect("the account signed the transaction");
        assert!(report.payer_address.is_some());
    }

    #[test]
    fn default_code_reverts_unless_its_data_is_one_proof_index() {
        // No index, two bytes, an index that is not a proof frame, and one out of
        // bounds.
        for data in [vec![], vec![0x00, 0x00], vec![0x01], vec![0x09]] {
            let result = run_self_verify(|tx| tx.frames[1].data = Bytes::from(data.clone()));
            assert!(result.is_err(), "data {data:?}: got {result:?}");
        }
    }

    #[test]
    fn default_code_reverts_on_a_claim_that_is_not_the_accounts_signature() {
        // A valid claim of the same key over an explicit message does not commit
        // to the transaction.
        let result = run_self_verify(|tx| {
            let mut data = tx.frames[0].data.to_vec();
            data[32..64].fill(0x09);
            tx.frames[0].data = Bytes::from(data);
        });
        assert!(result.is_err(), "explicit msg: got {result:?}");

        // A valid claim of another key.
        let (other_key, other) = key_and_address(0x43);
        let (_, sender) = key_and_address(0x42);
        let mut tx = frame_tx_with_frames(vec![
            secp256k1_proof_frame(other),
            Frame {
                target: None,
                data: Bytes::from_static(&[0x00]),
                ..verify_frame(sender)
            },
        ]);
        tx.sender = sender;
        sign(&mut tx, 0, &other_key);
        let result = run_frame_tx(
            &[(
                sender,
                U256::from(10u64).pow(U256::from(18u64)),
                0,
                Bytes::new(),
            )],
            tx,
        )
        .0;
        assert!(result.is_err(), "other key: got {result:?}");
    }

    #[test]
    fn default_code_reverts_on_a_claim_of_another_verifier() {
        // A contract verifier that accepts everything proves nothing about the
        // account: the default code only relies on `SECP256K1_VERIFIER`.
        let (_, sender) = key_and_address(0x42);
        let accepting = Address::repeat_byte(0x7E);
        let mut proof = secp256k1_proof_frame(sender);
        proof.target = Some(accepting);
        proof.gas_limit = 50_000;
        let mut tx = frame_tx_with_frames(vec![
            proof,
            Frame {
                target: None,
                data: Bytes::from_static(&[0x00]),
                ..verify_frame(sender)
            },
        ]);
        tx.sender = sender;
        let result = run_frame_tx(
            &[
                (
                    sender,
                    U256::from(10u64).pow(U256::from(18u64)),
                    0,
                    Bytes::new(),
                ),
                (accepting, U256::zero(), 1, Bytes::from_static(&[0x00])),
            ],
            tx,
        )
        .0;
        assert!(
            matches!(
                result,
                Err(VMError::TxValidation(
                    ethrex_levm::errors::TxValidationError::InvalidFrameTransaction
                ))
            ),
            "got {result:?}"
        );
        // The P-256 verifier is a different verifier too.
        assert_ne!(FRAME_TX_P256_VERIFIER, FRAME_TX_SECP256K1_VERIFIER);
    }

    #[test]
    fn default_code_of_a_sponsor_approves_payment_on_its_own_claim() {
        // Sender VERIFY frame (contract code approves execution) + payment-only
        // VERIFY frame whose target is a codeless payer, so default code runs and
        // reads the payer's claim, the proof frame its data names.
        let (payer_key, payer) = key_and_address(0x42);
        let run = |payer_proof_index: u8| {
            let mut tx = frame_tx_with_frames(vec![
                secp256k1_proof_frame(payer),
                verify_frame(FUNDED_SENDER),
                Frame {
                    data: Bytes::from(vec![payer_proof_index]),
                    ..pay_frame(payer)
                },
            ]);
            tx.frames[1].flags = 0x02;
            sign(&mut tx, 0, &payer_key);
            run_frame_tx(
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
            )
            .0
        };
        let report = run(0).expect("the payer signed the transaction");
        assert_eq!(report.payer_address, Some(payer));
        assert!(run(1).is_err(), "frame 1 is not a proof frame");
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
        state_gas_limit: 1_000_000,
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

    // Per-frame `gas_used` is reported before refunds, so the pre-refund total is
    // the mandatory costs plus the data cost plus each frame's gas.
    let frames_gas: u64 = frame_results.iter().map(|(_, gas, ..)| *gas).sum();
    let pre_refund = tx.mandatory_gas() + tx.data_cost() + frames_gas;
    // EIP-7778: the refund lowers what the payer pays (`gas_spent`) without freeing
    // the block capacity the transaction occupied (`gas_used`). Both carry the state
    // dimension on top of execution.
    assert_eq!(
        report.gas_used - report.state_gas_used,
        pre_refund,
        "the block figure must not apply the refund"
    );
    let payer_execution = report.gas_spent - report.state_gas_used;
    assert!(
        payer_execution < pre_refund,
        "the clearing frame's refund must be applied to what the payer pays \
         (pre-refund {pre_refund}, payer {payer_execution})"
    );
    let applied = pre_refund - payer_execution;
    assert!(
        applied <= (pre_refund + report.state_gas_used) / 5,
        "the applied refund {applied} must respect the EIP-3529 one-fifth cap"
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
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::from(vec![0x11u8; 4_096]),
    }]);
    tx.sender = FUNDED_SENDER;

    assert!(
        tx.calldata_floor_total() > tx.standard_gas_limit(),
        "the floor must bind for this to test the reservation"
    );
    assert_eq!(
        tx.max_gas(),
        tx.calldata_floor_total(),
        "max_gas must be the floor when the floor binds"
    );
    assert!(
        tx.validate_static_constraints().is_ok(),
        "a floor-bound transaction is valid; it reserves the floor rather than being rejected"
    );
}

/// A floor-bound frame transaction settles at the floor: the reservation raised
/// to `calldata_floor_gas` is what the payer is charged for, and no frame gas is
/// reported as refunded because the floor already absorbs it.
#[test]
fn a_floor_bound_frame_transaction_is_charged_the_floor() {
    use ethrex_common::types::Frame;

    let tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(FUNDED_SENDER),
        gas_limit: 100_000,
        state_gas_limit: 0,
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
        tx,
    );
    let report = result.expect("valid: the self-verify frame approves execution and payment");
    assert_eq!(
        report.gas_used, floor_total,
        "a floor-bound transaction is charged max_gas, which is the floor"
    );
    assert_eq!(
        report.gas_refunded, 0,
        "the floor leaves no frame gas to report as refunded"
    );
}

// ==================== atomic batch unroll / EIP-7928 BAL ====================

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
        state_gas_limit: 1_000_000,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let plain_frame = |target: Address| Frame {
        mode: u8::from(FrameMode::Sender),
        flags: 0x00,
        target: Some(target),
        gas_limit: 100_000,
        state_gas_limit: 1_000_000,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let verify = Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(FUNDED_SENDER),
        gas_limit: 80_000,
        state_gas_limit: 1_000_000,
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
    // reverted *access* stays. Finding the entry with `if let` would let the test
    // pass if the whole address vanished from the BAL -- which is the same class of
    // stall from the other side, and exactly what swapping this call site's
    // `BlockAccessListCheckpoint` for a `TxCheckpoint` (which does roll back
    // `touched_addresses`) would produce.
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
        state_gas_limit: 1_000_000,
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
        state_gas_limit: 1_000_000,
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

/// EIP-8141: "if a frame's execution reverts, its state changes **and approval
/// context (`payer`, `sender_approved`)** are discarded."
///
/// `APPROVE` exits its own call context, so a frame cannot approve and then revert
/// directly -- but a nested call back into the frame's target can. The inner call
/// approves and returns; the outer frame then reverts. Everything `APPROVE` did to
/// state (the sender nonce increment, the `max_cost` collection) rolls back with the
/// frame, while `payer` and `sender_approved` live in the transaction context,
/// outside the substate and the cache, and used to survive.
///
/// The consequence is not a lost approval but a free transaction: the payer stays
/// bound, so the transaction is valid and settles fees against an account whose
/// `max_cost` debit was rolled back.
#[test]
fn a_reverting_frame_discards_the_approval_it_granted() {
    use ethrex_common::types::Frame;

    // CALLDATASIZE != 0 (the outer frame, which carries `data`) takes the self-call
    // branch; the inner call gets empty calldata and takes the APPROVE branch.
    //
    //   00 CALLDATASIZE ; 01 PUSH1 0x0B ; 03 JUMPI      -> outer branch at 0x0B
    //   04 PUSH1 3 ; 06 PUSH1 0 ; 08 PUSH1 0 ; 0A APPROVE   (scope 3, exits context)
    //   0B JUMPDEST ; five PUSH1 0 (ret/args/value) ; 16 ADDRESS ; 17 GAS ; 18 CALL
    //   19 POP ; 1A PUSH1 0 ; 1C PUSH1 0 ; 1E REVERT
    let approve_then_revert = Bytes::from(vec![
        0x36, 0x60, 0x0B, 0x57, // CALLDATASIZE; PUSH1 0x0B; JUMPI
        0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA, // PUSH1 3; PUSH1 0; PUSH1 0; APPROVE
        0x5B, // JUMPDEST
        0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, // ret/args/value = 0
        0x30, 0x5A, 0xF1, 0x50, // ADDRESS; GAS; CALL; POP
        0x60, 0x00, 0x60, 0x00, 0xFD, // PUSH1 0; PUSH1 0; REVERT
    ]);

    // A DEFAULT frame targeting `tx.sender`: the approval scopes require the target
    // to be `tx.sender`, and DEFAULT is the only mode that both reaches the EVM and
    // carries no precondition of its own -- a VERIFY frame invalidates the
    // transaction merely by reverting, and a SENDER frame invalidates it by running
    // before the sender approval exists. Either would mask the behaviour under test.
    let tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0x03,
        target: Some(FUNDED_SENDER),
        gas_limit: 400_000,
        state_gas_limit: 1_000_000,
        value: U256::zero(),
        data: Bytes::from_static(&[0x01]),
    }]);

    let (result, _db) = run_frame_tx(
        &[(
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            approve_then_revert,
        )],
        tx,
    );

    let err = result.expect_err(
        "the only frame reverted, so its approval is discarded and the transaction \
         has no payer -- keeping the payer would settle fees against an account whose \
         max_cost debit was rolled back",
    );
    assert!(
        matches!(err, VMError::TxValidation(_)),
        "expected a transaction-validation failure for the missing payer, got {err:?}"
    );
}

/// The account code the call-level approval tests run as `tx.sender`. Its branch
/// is picked by the calldata: empty calldata approves (scope 3); `[1]` calls itself
/// with empty calldata and then reverts; `[2]` calls itself with `[1]` and stops;
/// `[3]` calls itself with `[1]` and then approves (scope 3) itself.
///
/// With `[2]` the frame succeeds while the `[1]` call, which sits above the inner
/// `APPROVE`, reverts.
const NESTED_APPROVER_CODE: &[u8] = &[
    // 0x00: CALLDATASIZE; ISZERO; PUSH1 0x5c; JUMPI      -> approve
    0x36, 0x15, 0x60, 0x5c, 0x57, // 0x05: first calldata byte
    0x60, 0x00, 0x35, 0x60, 0xf8, 0x1c,
    // 0x0b: DUP1; PUSH1 2; EQ; PUSH1 0x2c; JUMPI          -> outer
    0x80, 0x60, 0x02, 0x14, 0x60, 0x2c, 0x57,
    // 0x12: DUP1; PUSH1 3; EQ; PUSH1 0x41; JUMPI          -> outer_again
    0x80, 0x60, 0x03, 0x14, 0x60, 0x41, 0x57,
    // 0x19: [1]: CALL(GAS, ADDRESS, 0, 0, 0, 0, 0); POP; REVERT(0, 0)
    0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x30, 0x5a, 0xf1, 0x50, 0x60, 0x00,
    0x60, 0x00, 0xfd,
    // 0x2c: outer: MSTORE8(0, 1); CALL(GAS, ADDRESS, 0, 0, 1, 0, 0); POP; STOP
    0x5b, 0x60, 0x01, 0x60, 0x00, 0x53, 0x60, 0x00, 0x60, 0x00, 0x60, 0x01, 0x60, 0x00, 0x60, 0x00,
    0x30, 0x5a, 0xf1, 0x50, 0x00,
    // 0x41: outer_again: MSTORE8(0, 1); CALL(GAS, ADDRESS, 0, 0, 1, 0, 0); POP;
    //       APPROVE(0, 0, 3)
    0x5b, 0x60, 0x01, 0x60, 0x00, 0x53, 0x60, 0x00, 0x60, 0x00, 0x60, 0x01, 0x60, 0x00, 0x60, 0x00,
    0x30, 0x5a, 0xf1, 0x50, 0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xaa,
    // 0x5c: approve: APPROVE(0, 0, 3)
    0x5b, 0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xaa,
];

/// A DEFAULT frame running `NESTED_APPROVER_CODE` as `tx.sender` with `data`.
/// DEFAULT is the mode that reaches the EVM with no precondition of its own (see
/// `a_reverting_frame_discards_the_approval_it_granted`).
fn nested_approver_frame(data: u8) -> Frame {
    Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0x03,
        target: Some(FUNDED_SENDER),
        gas_limit: 400_000,
        state_gas_limit: 1_000_000,
        value: U256::zero(),
        data: Bytes::from(vec![data]),
    }
}

/// EIP-8141 journals the approval context with the state changes that produced it,
/// and "reverting a call, frame, or atomic batch restores" it. Here the frame
/// succeeds, but the call above the `APPROVE` reverts, which rolls back the nonce
/// increment and the `max_cost` collection the approval performed. Keeping the
/// approval would bind a payer whose escrow no longer exists: a free transaction.
#[test]
fn a_call_reverting_above_approve_discards_the_approval() {
    let tx = frame_tx_with_frames(vec![nested_approver_frame(2)]);
    let (result, db) = run_frame_tx(
        &[(
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(NESTED_APPROVER_CODE.to_vec()),
        )],
        tx,
    );

    let err = result.expect_err(
        "the approval was rolled back with the call above it, so no frame approved \
         payment and the transaction is invalid",
    );
    assert!(
        matches!(err, VMError::TxValidation(_)),
        "expected a transaction-validation failure for the missing payer, got {err:?}"
    );
    assert_eq!(nonce_of(&db, FUNDED_SENDER), 0);
    assert_eq!(balance_of(&db, FUNDED_SENDER), AUTO_SEED_SENDER_BALANCE);
}

/// The rollback restores the approval context to what it was when the reverted call
/// began, so the frame can approve again afterwards, and pays once.
#[test]
fn the_approval_can_be_granted_again_after_the_call_holding_it_reverts() {
    let tx = frame_tx_with_frames(vec![nested_approver_frame(3)]);
    let (result, db) = run_frame_tx(
        &[(
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(NESTED_APPROVER_CODE.to_vec()),
        )],
        tx,
    );

    let report = result.expect(
        "the outer APPROVE must succeed: the inner approval was rolled back with the \
         call that held it",
    );
    let frame_results = report.frame_results.expect("per-frame results");
    assert_eq!(frame_results[0].0, FRAME_RECEIPT_STATUS_SUCCESS);
    assert_eq!(
        nonce_of(&db, FUNDED_SENDER),
        1,
        "only the approval that stood may increment the nonce"
    );
}

/// The account code the call-level state-gas tests run. Its branch is picked by
/// the first calldata byte, and each one stops unless it says otherwise:
///
/// - `1`: create slot 0 (SSTORE 1).
/// - `2`: call itself with `3`, then create slot 1.
/// - `3`: clear slot 0, then revert.
/// - `4`: call itself with `3`.
/// - `5`: clear slot 0, then create slot 2.
const SLOT_OWNER_CODE: &[u8] = &[
    // 0x00: first calldata byte, then one DUP1; PUSH1 n; EQ; PUSH1 label; JUMPI per branch
    0x60, 0x00, 0x35, 0x60, 0xf8, 0x1c, 0x80, 0x60, 0x01, 0x14, 0x60, 0x2a, 0x57, 0x80, 0x60, 0x02,
    0x14, 0x60, 0x31, 0x57, 0x80, 0x60, 0x03, 0x14, 0x60, 0x4b, 0x57, 0x80, 0x60, 0x04, 0x14, 0x60,
    0x56, 0x57, 0x80, 0x60, 0x05, 0x14, 0x60, 0x6b, 0x57, 0x00,
    // 0x2a: 1: SSTORE(0, 1); STOP
    0x5b, 0x60, 0x01, 0x60, 0x00, 0x55, 0x00,
    // 0x31: 2: MSTORE8(0, 3); CALL(GAS, ADDRESS, 0, 0, 1, 0, 0); POP; SSTORE(1, 1); STOP
    0x5b, 0x60, 0x03, 0x60, 0x00, 0x53, 0x60, 0x00, 0x60, 0x00, 0x60, 0x01, 0x60, 0x00, 0x60, 0x00,
    0x30, 0x5a, 0xf1, 0x50, 0x60, 0x01, 0x60, 0x01, 0x55, 0x00,
    // 0x4b: 3: SSTORE(0, 0); REVERT(0, 0)
    0x5b, 0x60, 0x00, 0x60, 0x00, 0x55, 0x60, 0x00, 0x60, 0x00, 0xfd,
    // 0x56: 4: MSTORE8(0, 3); CALL(GAS, ADDRESS, 0, 0, 1, 0, 0); POP; STOP
    0x5b, 0x60, 0x03, 0x60, 0x00, 0x53, 0x60, 0x00, 0x60, 0x00, 0x60, 0x01, 0x60, 0x00, 0x60, 0x00,
    0x30, 0x5a, 0xf1, 0x50, 0x00, // 0x6b: 5: SSTORE(0, 0); SSTORE(2, 1); STOP
    0x5b, 0x60, 0x00, 0x60, 0x00, 0x55, 0x60, 0x01, 0x60, 0x02, 0x55, 0x00,
];

/// Runs `[VERIFY, DEFAULT(SLOT_OWNER_CODE, data, state budget)...]` and returns the
/// report and the database.
fn run_slot_owner_frames(frames: &[(u8, u64)]) -> (ExecutionReport, GeneralizedDatabase, Address) {
    let owner_contract = Address::from_low_u64_be(0x8141_0e3e);
    let mut all = vec![verify_frame(FUNDED_SENDER)];
    all.extend(frames.iter().map(|&(data, state_gas_limit)| Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(owner_contract),
        gas_limit: 400_000,
        state_gas_limit,
        value: U256::zero(),
        data: Bytes::from(vec![data]),
    }));
    let (result, db) = run_frame_tx(
        &[
            (
                FUNDED_SENDER,
                AUTO_SEED_SENDER_BALANCE,
                0,
                Bytes::from(APPROVE_BOTH_CODE.to_vec()),
            ),
            (
                owner_contract,
                U256::zero(),
                0,
                Bytes::from(SLOT_OWNER_CODE.to_vec()),
            ),
        ],
        frame_tx_with_frames(all),
    );
    (
        result.expect("the transaction is valid"),
        db,
        owner_contract,
    )
}

/// A nested call clears a slot an earlier frame created, which refills that frame's
/// receipt, and then reverts. EIP-8141 journals the receipt change with the state
/// change that caused it, so the revert restores both: the earlier frame stays
/// billed for the slot that still exists, and the executing frame's state pool
/// keeps its full budget for its own creation afterwards.
#[test]
fn a_reverted_call_restores_the_refill_it_credited_to_an_earlier_frame() {
    let (report, db, owner_contract) =
        run_slot_owner_frames(&[(1, SSTORE_SET_STATE_GAS), (2, SSTORE_SET_STATE_GAS)]);
    let frame_results = report.frame_results.expect("per-frame results");
    let statuses: Vec<u8> = frame_results.iter().map(|result| result.0).collect();
    assert_eq!(statuses, vec![FRAME_RECEIPT_STATUS_SUCCESS; 3]);
    let state_gas: Vec<u64> = frame_results.iter().map(|result| result.2).collect();
    assert_eq!(
        state_gas,
        vec![0, SSTORE_SET_STATE_GAS, SSTORE_SET_STATE_GAS],
        "frame 1 still owns slot 0, and frame 2 paid for slot 1 from its own budget"
    );
    assert_eq!(report.state_gas_used, 2 * SSTORE_SET_STATE_GAS);
    assert_eq!(
        storage_slot(&db, owner_contract, H256::from_low_u64_be(0)),
        U256::one()
    );
    assert_eq!(
        storage_slot(&db, owner_contract, H256::from_low_u64_be(1)),
        U256::one()
    );
}

/// The same reverted call also consumed the slot's ownership record, and so does a
/// reverted frame. Restoring the record sends the refill of a later, successful
/// clear to the frame that paid for the slot, rather than into the clearing frame's
/// own pool.
#[test]
fn a_reversion_restores_the_ownership_record_it_consumed() {
    for (label, clearing_frame, clearing_status) in [
        ("a reverted call", 4, FRAME_RECEIPT_STATUS_SUCCESS),
        ("a reverted frame", 3, FRAME_RECEIPT_STATUS_FAILURE),
    ] {
        let (report, db, owner_contract) = run_slot_owner_frames(&[
            (1, SSTORE_SET_STATE_GAS),
            (clearing_frame, 0),
            (5, SSTORE_SET_STATE_GAS),
        ]);
        let frame_results = report.frame_results.expect("per-frame results");
        let statuses: Vec<u8> = frame_results.iter().map(|result| result.0).collect();
        assert_eq!(
            statuses,
            vec![
                FRAME_RECEIPT_STATUS_SUCCESS,
                FRAME_RECEIPT_STATUS_SUCCESS,
                clearing_status,
                FRAME_RECEIPT_STATUS_SUCCESS
            ],
            "{label}"
        );
        let state_gas: Vec<u64> = frame_results.iter().map(|result| result.2).collect();
        assert_eq!(
            state_gas,
            vec![0, 0, 0, SSTORE_SET_STATE_GAS],
            "{label}: frame 3's clear refills frame 1, which paid for slot 0, and frame 3 \
             pays for slot 2 from its own budget"
        );
        assert_eq!(report.state_gas_used, SSTORE_SET_STATE_GAS, "{label}");
        assert_eq!(
            storage_slot(&db, owner_contract, H256::from_low_u64_be(0)),
            U256::zero()
        );
        assert_eq!(
            storage_slot(&db, owner_contract, H256::from_low_u64_be(2)),
            U256::one()
        );
    }
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
        state_gas_limit: 1_000_000,
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
            state_gas_limit: 1_000_000,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ]);

    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("a reverting batch must not invalidate the tx");
    let frames = report.frame_results.expect("frame results present");

    // The frame that ran before the failure keeps its status and its gas.
    assert_eq!(
        frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS,
        "a frame that succeeded before the failure keeps its status through the unroll"
    );
    assert!(
        frames[1].1 > 0 && frames[1].1 < BATCH_FRAME_GAS,
        "it keeps the gas it used ({}), not the frame's whole gas_limit",
        frames[1].1
    );
    assert!(
        frames[1].3.is_empty(),
        "its logs go with the state changes the unroll dropped: {:?}",
        frames[1].3
    );

    // The failing frame reverted, so it is charged what it actually used.
    assert_eq!(frames[2].0, FRAME_RECEIPT_STATUS_FAILURE);
    assert!(
        frames[2].1 < BATCH_FRAME_GAS,
        "a REVERT is charged its actual gas ({}), not the full gas_limit",
        frames[2].1
    );

    // The remaining batch member never executed.
    assert_eq!(frames[3].0, FRAME_RECEIPT_STATUS_SKIPPED);
    assert_eq!(frames[3].1, 0, "a skipped frame's gas is refunded");

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

// ==================== Rejection-reason granularity ====================
//
// A frame transaction that is rejected for the right reason but reports the
// wrong one is indistinguishable, to a conformance harness, from one rejected
// by accident. These pin the reason, not just the rejection: every case below
// was already rejected before this suite existed, but every one of them
// reported `InvalidFrameTransaction` -- the VERIFY-frame-never-approved
// message -- regardless of what actually failed.

/// A frame with a reserved mode fails static validation, and the reason travels
/// with the error instead of being flattened into the approval message.
#[test]
fn static_constraint_failure_reports_the_format_reason() {
    let mut tx = frame_tx_with_frames(vec![Frame {
        mode: 0xFF, // no such mode
        flags: 0,
        target: Some(Address::from_low_u64_be(0xC0)),
        gas_limit: 100_000,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    tx.nonce_seq = 0;

    let (result, _db) = run_frame_tx(&[], tx);
    match result {
        Err(VMError::TxValidation(
            ethrex_levm::errors::TxValidationError::InvalidFrameTransactionFormat(reason),
        )) => {
            assert!(
                !reason.is_empty(),
                "the format error must carry the static-validation reason"
            );
        }
        other => panic!("expected InvalidFrameTransactionFormat, got {other:?}"),
    }
}

/// An empty frame list is a format failure, not an approval failure.
#[test]
fn empty_frame_list_reports_the_format_reason() {
    let tx = frame_tx_with_frames(Vec::new());
    let (result, _db) = run_frame_tx(&[], tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::InvalidFrameTransactionFormat(_)
            ))
        ),
        "expected InvalidFrameTransactionFormat, got {result:?}"
    );
}

/// EIP-7825 as scoped by EIP-8141: the intrinsic cost plus the frames' gas
/// limits must fit `TX_MAX_GAS_LIMIT`. Previously this transaction executed and
/// was only caught downstream.
#[test]
fn frame_gas_above_the_transaction_cap_reports_the_gas_cap() {
    let tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(Address::from_low_u64_be(0xC0)),
        gas_limit: ethrex_common::constants::TX_MAX_GAS_LIMIT_AMSTERDAM,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    assert!(
        tx.max_gas() > ethrex_common::constants::TX_MAX_GAS_LIMIT_AMSTERDAM,
        "the frame gas plus the intrinsic cost must exceed the cap for this to test anything"
    );

    let (result, _db) = run_frame_tx(&[], tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::TxMaxGasLimitExceeded { .. }
            ))
        ),
        "expected TxMaxGasLimitExceeded, got {result:?}"
    );
}

/// A transaction sized exactly to the cap stays admissible, so the check is a
/// bound rather than an off-by-one.
#[test]
fn frame_gas_exactly_at_the_transaction_cap_is_not_rejected_for_gas() {
    let cap = ethrex_common::constants::TX_MAX_GAS_LIMIT_AMSTERDAM;
    let probe = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(Address::from_low_u64_be(0xC0)),
        gas_limit: 0,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    // `standard_gas_limit()` with a zero-gas frame is the intrinsic anchor; give the
    // frame exactly the remainder so the total lands on the cap. (`max_gas()` would
    // anchor on the calldata floor, which the probe's tiny payload can exceed.)
    let headroom = cap - probe.standard_gas_limit();
    let tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(Address::from_low_u64_be(0xC0)),
        gas_limit: headroom,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    assert_eq!(tx.max_gas(), cap, "this case must sit exactly on the cap");

    let (result, _db) = run_frame_tx(&[], tx);
    assert!(
        !matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::TxMaxGasLimitExceeded { .. }
            ))
        ),
        "a transaction exactly at the cap must not be rejected for exceeding it; got {result:?}"
    );
}

/// A nonce at the u64 ceiling can never be incremented, so it is invalid on its
/// own terms rather than merely mismatched against the sender's nonce.
#[test]
fn nonce_at_the_u64_ceiling_reports_nonce_is_max() {
    let mut tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(Address::from_low_u64_be(0xC0)),
        gas_limit: 100_000,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    tx.nonce_seq = u64::MAX;

    // `run_frame_tx` seeds the sender at `tx.nonce_seq`, so the mismatch check
    // cannot fire and the ceiling rule is the only one left to report.
    let (result, _db) = run_frame_tx(&[], tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::NonceIsMax
            ))
        ),
        "expected NonceIsMax, got {result:?}"
    );
}

/// A versioned hash with an unrecognised version byte is an EIP-4844 failure,
/// not a generic frame-format one.
#[test]
fn wrong_blob_version_byte_reports_the_blob_hash_rule() {
    let mut tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(Address::from_low_u64_be(0xC0)),
        gas_limit: 100_000,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    let mut hash = [0u8; 32];
    hash[0] = 0x02; // not VERSIONED_HASH_VERSION_KZG
    tx.blob_versioned_hashes = vec![H256(hash)];

    let (result, _db) = run_frame_tx(&[], tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::Type3TxInvalidBlobVersionedHash
            ))
        ),
        "expected Type3TxInvalidBlobVersionedHash, got {result:?}"
    );
}

/// EIP-8141 §Fee budget: a `max_fee` below `max_gas` at the base fee is invalid
/// before any frame executes, however large the priority fee per gas is.
#[test]
fn max_fee_below_the_base_fee_requirement_is_rejected() {
    let mut tx = frame_tx_with_frames(vec![Frame {
        mode: u8::from(FrameMode::Default),
        flags: 0,
        target: Some(Address::from_low_u64_be(0xC0)),
        gas_limit: 100_000,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    tx.max_fee = U256::from(tx.max_gas() * HARNESS_BASE_FEE - 1);
    tx.max_priority_fee_per_gas = U256::MAX;

    let (result, _db) = run_frame_tx(&[], tx);
    assert!(
        matches!(
            result,
            Err(VMError::TxValidation(
                ethrex_levm::errors::TxValidationError::InsufficientMaxFeePerGas
            ))
        ),
        "expected InsufficientMaxFeePerGas, got {result:?}"
    );
}

// ==================== frames devnet block 41 repros ====================

fn sender_frame(target: Address, value: U256, gas_limit: u64) -> Frame {
    Frame {
        mode: u8::from(FrameMode::Sender),
        flags: 0,
        target: Some(target),
        gas_limit,
        state_gas_limit: 1_000_000,
        value,
        data: Bytes::new(),
    }
}

/// A frame that halts with items on its stack must not hand them to the next frame:
/// every frame is a fresh top-level call with an empty stack (EELS `create_evm_from_frame`).
#[test]
fn stack_does_not_leak_across_frames() {
    // JUMPDEST; PUSH0; PUSH1 0; JUMP -- grows the stack until it overflows.
    let overflow = Address::from_low_u64_be(0x0F10);
    let store = Address::from_low_u64_be(0x0F11);
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        sender_frame(overflow, U256::zero(), 100_000),
        sender_frame(store, U256::zero(), 100_000),
    ]);
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (
            overflow,
            U256::zero(),
            1,
            Bytes::from(vec![0x5b, 0x5f, 0x60, 0x00, 0x56]),
        ),
        (
            store,
            U256::zero(),
            1,
            Bytes::from(SSTORE_THEN_STOP_CODE.to_vec()),
        ),
    ];
    let (result, db) = run_frame_tx(&accounts, tx);
    let report = result.expect("tx is valid");
    let frames = report.frame_results.as_ref().expect("frame results");
    assert_ne!(
        frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS,
        "overflow frame must fail"
    );
    assert_eq!(
        frames[2].0, FRAME_RECEIPT_STATUS_SUCCESS,
        "frame 2 inherited frame 1's stack"
    );
    assert_eq!(storage_slot(&db, store, H256::zero()), U256::one());
}

/// A successful frame targeting a precompile after an earlier frame logged must not panic
/// in the per-frame log slicing (vm.rs `split_off(substate_logs_before)`).
#[test]
fn precompile_frame_after_logging_frame_does_not_panic() {
    let eoa = Address::from_low_u64_be(0xE0C);
    let identity = Address::from_low_u64_be(0x04);
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        sender_frame(eoa, U256::from(1u64), 50_000),
        sender_frame(identity, U256::zero(), 50_000),
    ]);
    let accounts = [
        (
            FUNDED_SENDER,
            AUTO_SEED_SENDER_BALANCE,
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (eoa, U256::one(), 0, Bytes::new()),
    ];
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("tx is valid");
    let frames = report.frame_results.as_ref().expect("frame results");
    assert_eq!(frames[2].0, FRAME_RECEIPT_STATUS_SUCCESS);
}

/// The target access charge comes before the balance check (EIP-8141 Behavior): a frame
/// that cannot pay the access halts with its whole limit even if `value` is also unfundable.
#[test]
fn unaffordable_access_wins_over_unfunded_value() {
    let dead = Address::from_low_u64_be(0xDEAD);
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        sender_frame(dead, U256::MAX, 1),
    ]);
    let accounts = [(
        FUNDED_SENDER,
        U256::from(10u64).pow(U256::from(18u64)),
        0,
        Bytes::from(APPROVE_BOTH_CODE.to_vec()),
    )];
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("tx is valid");
    let frames = report.frame_results.as_ref().expect("frame results");
    assert_ne!(frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS);
    assert_eq!(
        frames[1].1, 1,
        "frame must be billed its limit, not the access cost"
    );
}

/// EIP-8141 orders a frame's entry as the target's access charge, then the balance
/// check, then dispatch, which is where an EIP-7702 delegation is followed. A frame
/// whose sender cannot fund its `value` therefore reverts having paid the target's
/// access alone, even when its limit could not have covered the delegate's access
/// on top: it never reaches the delegate.
#[test]
fn unfunded_value_reverts_before_the_delegate_is_charged() {
    let recipient = Address::from_low_u64_be(0xBEEF);
    let delegate = Address::from_low_u64_be(0xDE1E);
    let mut delegation = ethrex_levm::constants::SET_CODE_DELEGATION_BYTES.to_vec();
    delegation.extend_from_slice(delegate.as_bytes());
    let target_access = ethrex_levm::gas_cost::cold_account_access_cost(Fork::Hegota);
    let tx = frame_tx_with_frames(vec![
        verify_frame(FUNDED_SENDER),
        sender_frame(recipient, U256::MAX, target_access + 1),
    ]);
    let accounts = [
        (
            FUNDED_SENDER,
            U256::from(10u64).pow(U256::from(18u64)),
            0,
            Bytes::from(APPROVE_BOTH_CODE.to_vec()),
        ),
        (recipient, U256::zero(), 0, Bytes::from(delegation)),
        (delegate, U256::zero(), 0, Bytes::from_static(&[0x00])),
    ];
    let (result, _db) = run_frame_tx(&accounts, tx);
    let report = result.expect("tx is valid");
    let frames = report.frame_results.as_ref().expect("frame results");
    assert_ne!(frames[1].0, FRAME_RECEIPT_STATUS_SUCCESS);
    assert_eq!(
        frames[1].1, target_access,
        "the unfunded transfer reverts paying the target's access, not its whole limit"
    );
}
