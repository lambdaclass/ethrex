//! Integration tests for EIP-8141 Frame Transaction execution.
//!
//! These tests exercise the frame execution loop, opcode handlers (APPROVE, TXPARAMLOAD),
//! and frame isolation semantics (transient storage, warm/cold journal, gas, reverts).
//!
//! Run with: `cargo test -p ethrex-levm --features eip-8141`

#![cfg(feature = "eip-8141")]

use bytes::Bytes;
use ethrex_common::{
    types::{
        transaction::{EIP8141Transaction, Frame, FrameMode},
        AccountState, ChainConfig, Code, CodeMetadata,
    },
    Address, H160, H256, U256,
};
use ethrex_levm::{
    db::Database,
    errors::DatabaseError,
};
use rustc_hash::FxHashMap;

// ---------------------------------------------------------------------------
// Test database
// ---------------------------------------------------------------------------

/// Minimal in-memory database for integration tests.
#[derive(Clone)]
struct TestDB {
    accounts: FxHashMap<Address, AccountState>,
    code: FxHashMap<H256, Code>,
    storage: FxHashMap<(Address, H256), U256>,
}

impl TestDB {
    fn new() -> Self {
        Self {
            accounts: FxHashMap::default(),
            code: FxHashMap::default(),
            storage: FxHashMap::default(),
        }
    }

    fn with_account(mut self, address: Address, balance: U256, nonce: u64, code: Bytes) -> Self {
        let code_obj = Code::from_bytecode(code);
        let code_hash = code_obj.hash;
        self.accounts.insert(
            address,
            AccountState {
                balance,
                nonce,
                code_hash,
                storage_root: H256::zero(),
            },
        );
        if !code_obj.bytecode.is_empty() {
            self.code.insert(code_hash, code_obj);
        }
        self
    }

    fn with_storage(mut self, address: Address, slot: H256, value: U256) -> Self {
        self.storage.insert((address, slot), value);
        self
    }
}

impl Database for TestDB {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        Ok(self.accounts.get(&address).copied().unwrap_or_default())
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        Ok(self.storage.get(&(address, key)).copied().unwrap_or_default())
    }

    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig::default())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        Ok(self
            .code
            .get(&code_hash)
            .cloned()
            .unwrap_or_else(|| Code::from_bytecode(Bytes::new())))
    }

    fn get_code_metadata(&self, _code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        Ok(CodeMetadata { length: 0 })
    }
}

// ---------------------------------------------------------------------------
// EVM bytecode helpers
// ---------------------------------------------------------------------------

/// APPROVE opcode byte
const OP_APPROVE: u8 = 0xAA;
/// TXPARAMLOAD opcode byte
const OP_TXPARAMLOAD: u8 = 0xB0;
/// TXPARAMSIZE opcode byte
const OP_TXPARAMSIZE: u8 = 0xB1;
/// TXPARAMCOPY opcode byte
const OP_TXPARAMCOPY: u8 = 0xB2;

// Standard opcodes
const OP_PUSH1: u8 = 0x60;
const OP_PUSH32: u8 = 0x7F;
const OP_STOP: u8 = 0x00;
const OP_RETURN: u8 = 0xF3;
const OP_REVERT: u8 = 0xFD;
const OP_SSTORE: u8 = 0x55;
const OP_SLOAD: u8 = 0x54;
const OP_TSTORE: u8 = 0x5D;
const OP_TLOAD: u8 = 0x5C;
const OP_ORIGIN: u8 = 0x32;
const OP_MSTORE: u8 = 0x52;
const OP_CALLER: u8 = 0x33;

/// Build bytecode that calls APPROVE with the given scope (0x0, 0x1, or 0x2).
/// APPROVE pops: offset, length, scope from stack.
/// This bytecode: PUSH1 scope, PUSH1 0 (length), PUSH1 0 (offset) → APPROVE
fn approve_bytecode(scope: u8) -> Bytes {
    Bytes::from(vec![
        OP_PUSH1, scope,   // scope
        OP_PUSH1, 0x00,    // length = 0
        OP_PUSH1, 0x00,    // offset = 0
        OP_APPROVE,
    ])
}

/// Build bytecode that does TSTORE(key, value) then STOP.
fn tstore_bytecode(key: u8, value: u8) -> Bytes {
    Bytes::from(vec![
        OP_PUSH1, value,   // value
        OP_PUSH1, key,     // key
        OP_TSTORE,
        OP_STOP,
    ])
}

/// Build bytecode that does TLOAD(key) → SSTORE(slot, result) then STOP.
/// Stores the TLOAD result into persistent storage at the given slot for assertion.
fn tload_and_store_bytecode(key: u8, result_slot: u8) -> Bytes {
    Bytes::from(vec![
        OP_PUSH1, key,          // key
        OP_TLOAD,               // loads value from transient storage
        OP_PUSH1, result_slot,  // storage slot
        OP_SSTORE,              // store to persistent storage
        OP_STOP,
    ])
}

/// Build bytecode that reverts.
fn revert_bytecode() -> Bytes {
    Bytes::from(vec![
        OP_PUSH1, 0x00,   // length
        OP_PUSH1, 0x00,   // offset
        OP_REVERT,
    ])
}

/// Build bytecode that stores ORIGIN to storage slot 0.
fn store_origin_bytecode() -> Bytes {
    Bytes::from(vec![
        OP_ORIGIN,         // push msg.origin
        OP_PUSH1, 0x00,   // slot 0
        OP_SSTORE,
        OP_STOP,
    ])
}

/// Build bytecode that does SSTORE(slot, value) then STOP.
fn sstore_bytecode(slot: u8, value: u8) -> Bytes {
    Bytes::from(vec![
        OP_PUSH1, value,
        OP_PUSH1, slot,
        OP_SSTORE,
        OP_STOP,
    ])
}

// ---------------------------------------------------------------------------
// Helper to create a frame transaction
// ---------------------------------------------------------------------------

fn make_frame_tx(sender: Address, nonce: u64, frames: Vec<Frame>) -> EIP8141Transaction {
    EIP8141Transaction {
        chain_id: 1,
        nonce,
        sender,
        frames,
        max_priority_fee_per_gas: U256::from(1_000_000_000u64),
        max_fee_per_gas: U256::from(50_000_000_000u64),
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
    }
}

fn frame(mode: FrameMode, target: Address, gas_limit: u64, data: Bytes) -> Frame {
    Frame {
        mode,
        target: Some(target),
        gas_limit,
        data,
    }
}

// ---------------------------------------------------------------------------
// Test constants
// ---------------------------------------------------------------------------

const SENDER_ADDR: Address = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xAB, 0xCD]);
const VALIDATOR_ADDR: Address = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x11, 0x11]);
const RECIPIENT_ADDR: Address = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x22, 0x22]);
const PAYER_ADDR: Address = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x33, 0x33]);
#[allow(dead_code)]
const TEST_ENTRY_POINT: Address = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xAA]);

// ========================================================================
// T8.1: Happy path - VERIFY + SENDER frames
// ========================================================================

// NOTE: These integration tests require the frame execution loop (T6) to be
// implemented. They set up a TestDB with accounts and bytecode, construct a
// frame transaction, and execute it through the VM.
//
// The tests below define the expected behavior. Once T6 is merged and the
// execute_frame_tx function is available, uncomment and wire them up.

// TODO: Wire up once execute_frame_tx is available from T6.
// The test structure is ready — just needs the execution entry point.

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn happy_path_verify_and_sender() {
    // Setup: validator account has code that calls APPROVE(0x2) (sender + payer combined)
    // Recipient account has simple code.
    // sender has enough balance.
    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, approve_bytecode(0x02))
        .with_account(RECIPIENT_ADDR, U256::zero(), 0, sstore_bytecode(0, 1));

    let _tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
        frame(FrameMode::Sender, RECIPIENT_ADDR, 200_000, Bytes::new()),
    ]);

    // TODO: Execute through frame execution loop and assert:
    // - Transaction succeeds
    // - Nonce incremented
    // - RECIPIENT storage slot 0 == 1
}

// ========================================================================
// T8.2: Missing APPROVE -> tx invalid
// ========================================================================

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn missing_approve_tx_invalid() {
    // Validator code does NOT call APPROVE — just STOPs
    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, Bytes::from(vec![OP_STOP]));

    let _tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
    ]);

    // TODO: Execute and assert tx is invalid (VERIFY completed without APPROVE)
}

// ========================================================================
// T8.3: Missing payer approval -> tx invalid
// ========================================================================

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn missing_payer_approval_tx_invalid() {
    // Validator calls APPROVE(0x0) (sender only, not payer)
    // No subsequent frame calls APPROVE(0x1) for payment
    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, approve_bytecode(0x00));

    let _tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
    ]);

    // TODO: Execute and assert tx is invalid (payer not approved)
}

// ========================================================================
// T8.4: SENDER without prior approval -> fail
// ========================================================================

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn sender_without_approval_fails() {
    // SENDER frame comes before any VERIFY → should fail
    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(RECIPIENT_ADDR, U256::zero(), 0, sstore_bytecode(0, 1));

    let _tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Sender, RECIPIENT_ADDR, 200_000, Bytes::new()),
    ]);

    // TODO: Execute and assert SENDER frame fails (no sender_approved)
}

// ========================================================================
// T8.5: Frame revert isolation
// ========================================================================

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn frame_revert_isolation() {
    // Frame 0 (DEFAULT): writes storage then reverts
    // Frame 1 (DEFAULT): writes storage normally
    // Frame 0's changes should be rolled back; Frame 1's should persist.
    let reverting_code = {
        let mut code = Vec::new();
        // SSTORE(0, 0x42) then REVERT
        code.extend_from_slice(&[OP_PUSH1, 0x42, OP_PUSH1, 0x00, OP_SSTORE]);
        code.extend_from_slice(&[OP_PUSH1, 0x00, OP_PUSH1, 0x00, OP_REVERT]);
        Bytes::from(code)
    };

    let account_a = Address::from_low_u64_be(0xAAAA);
    let account_b = Address::from_low_u64_be(0xBBBB);

    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, approve_bytecode(0x02))
        .with_account(account_a, U256::zero(), 0, reverting_code)
        .with_account(account_b, U256::zero(), 0, sstore_bytecode(0, 0x99));

    let _tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
        frame(FrameMode::Sender, account_a, 200_000, Bytes::new()),  // reverts
        frame(FrameMode::Sender, account_b, 200_000, Bytes::new()),  // succeeds
    ]);

    // TODO: Execute and assert:
    // - account_a storage slot 0 == 0 (reverted)
    // - account_b storage slot 0 == 0x99 (persisted)
    // - Overall tx still succeeds (frame revert != tx failure for non-VERIFY)
}

// ========================================================================
// T8.6: Transient storage clearing between frames
// ========================================================================

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn transient_storage_cleared_between_frames() {
    // Frame 0: TSTORE(key=1, value=0xFF)
    // Frame 1: TLOAD(key=1), store result to persistent storage slot 0
    // If transient storage is cleared between frames, result should be 0.
    let contract = Address::from_low_u64_be(0x4444);

    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, approve_bytecode(0x02))
        .with_account(contract, U256::zero(), 0, Bytes::new()); // Code set per frame

    // NOTE: Each frame targets the same contract but with different data.
    // Frame 0 runs tstore_bytecode, Frame 1 runs tload_and_store_bytecode.
    // In practice, the contract would need to dispatch based on calldata,
    // or we use different contract addresses.

    let store_contract = Address::from_low_u64_be(0x4444);
    let load_contract = Address::from_low_u64_be(0x5555);

    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, approve_bytecode(0x02))
        .with_account(store_contract, U256::zero(), 0, tstore_bytecode(1, 0xFF))
        .with_account(load_contract, U256::zero(), 0, tload_and_store_bytecode(1, 0));

    let _tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
        frame(FrameMode::Sender, store_contract, 100_000, Bytes::new()),
        frame(FrameMode::Sender, load_contract, 100_000, Bytes::new()),
    ]);

    // TODO: Execute and assert:
    // - load_contract storage slot 0 == 0 (transient storage was cleared)
}

// ========================================================================
// T8.7: Warm/cold journal sharing across frames
// ========================================================================

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn warm_cold_journal_shared() {
    // Frame 0 accesses address X (cold → 2600 gas)
    // Frame 1 accesses address X (warm → 100 gas)
    // The warm/cold journal should carry over between frames.
    //
    // This test would compare gas used in Frame 0 vs Frame 1 for the same
    // SLOAD operation on the same address. The exact assertion depends on
    // the gas metering available in FrameResult.

    // TODO: Implement once execute_frame_tx with gas reporting is available
}

// ========================================================================
// T8.8: TXPARAMLOAD reading tx parameters
// ========================================================================

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn txparamload_reads_sender() {
    // Deploy contract that uses TXPARAMLOAD to read sender (param 0x02)
    // and stores it to storage slot 0.
    let txparamload_code = Bytes::from(vec![
        OP_PUSH1, 0x00,        // in2 = 0 (sub-selector)
        OP_PUSH1, 0x02,        // in1 = 0x02 (sender parameter)
        OP_TXPARAMLOAD,        // push sender to stack
        OP_PUSH1, 0x00,        // storage slot 0
        OP_SSTORE,
        OP_STOP,
    ]);

    let contract = Address::from_low_u64_be(0x6666);
    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, approve_bytecode(0x02))
        .with_account(contract, U256::zero(), 0, txparamload_code);

    let _tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
        frame(FrameMode::Sender, contract, 200_000, Bytes::new()),
    ]);

    // TODO: Execute and assert:
    // - contract storage slot 0 == SENDER_ADDR (as U256)
}

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn txparamload_reads_nonce() {
    // TXPARAMLOAD with in1=0x01 should return the tx nonce
    let txparamload_code = Bytes::from(vec![
        OP_PUSH1, 0x00,        // in2 = 0
        OP_PUSH1, 0x01,        // in1 = 0x01 (nonce)
        OP_TXPARAMLOAD,
        OP_PUSH1, 0x00,        // slot
        OP_SSTORE,
        OP_STOP,
    ]);

    let contract = Address::from_low_u64_be(0x7777);
    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 5, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, approve_bytecode(0x02))
        .with_account(contract, U256::zero(), 0, txparamload_code);

    let _tx = make_frame_tx(SENDER_ADDR, 5, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
        frame(FrameMode::Sender, contract, 200_000, Bytes::new()),
    ]);

    // TODO: Execute and assert:
    // - contract storage slot 0 == 5 (nonce)
}

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn txparamload_reads_sig_hash() {
    // TXPARAMLOAD with in1=0x08 should return compute_sig_hash()
    let txparamload_code = Bytes::from(vec![
        OP_PUSH1, 0x00,        // in2 = 0
        OP_PUSH1, 0x08,        // in1 = 0x08 (sig_hash)
        OP_TXPARAMLOAD,
        OP_PUSH1, 0x00,        // slot
        OP_SSTORE,
        OP_STOP,
    ]);

    let contract = Address::from_low_u64_be(0x8888);
    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, approve_bytecode(0x02))
        .with_account(contract, U256::zero(), 0, txparamload_code);

    let tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
        frame(FrameMode::Sender, contract, 200_000, Bytes::new()),
    ]);

    let _expected_hash = tx.compute_sig_hash();

    // TODO: Execute and assert:
    // - contract storage slot 0 == expected_hash (as U256)
}

// ========================================================================
// T8.9: ORIGIN behavior per frame mode
// ========================================================================

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn origin_returns_entry_point_in_default_frame() {
    // In a DEFAULT frame, ORIGIN should return ENTRY_POINT_ADDRESS
    let contract = Address::from_low_u64_be(0x9999);
    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, approve_bytecode(0x02))
        .with_account(contract, U256::zero(), 0, store_origin_bytecode());

    let _tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
        frame(FrameMode::Default, contract, 200_000, Bytes::new()),
    ]);

    // TODO: Execute and assert:
    // - contract storage slot 0 == ENTRY_POINT_ADDRESS (as U256)
}

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn origin_returns_sender_in_sender_frame() {
    // In a SENDER frame, ORIGIN should return tx.sender
    let contract = Address::from_low_u64_be(0xAAAA);
    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, approve_bytecode(0x02))
        .with_account(contract, U256::zero(), 0, store_origin_bytecode());

    let _tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
        frame(FrameMode::Sender, contract, 200_000, Bytes::new()),
    ]);

    // TODO: Execute and assert:
    // - contract storage slot 0 == SENDER_ADDR (as U256)
}

// ========================================================================
// T8.10: Gas isolation between frames
// ========================================================================

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn gas_isolation_between_frames() {
    // Frame 0: gas_limit=100_000, uses ~21_000
    // Frame 1: gas_limit=1_000
    // Frame 1 should NOT be able to use Frame 0's leftover gas.
    // If Frame 1 tries to do expensive operations, it should run out of gas.

    // TODO: Implement with specific gas-consuming bytecode and assert
    // Frame 1 runs out of gas if it tries to exceed its own limit.
}

// ========================================================================
// T8.11: APPROVE scope re-entry rejection
// ========================================================================

#[test]
#[ignore = "blocked on T6: execute_frame_tx not yet implemented"]
fn approve_scope_reentry_rejected() {
    // Call APPROVE(0x0) twice in the same VERIFY frame.
    // The second call should revert.
    let double_approve_code = Bytes::from(vec![
        // First APPROVE(0x0)
        OP_PUSH1, 0x00,   // scope = 0
        OP_PUSH1, 0x00,   // length
        OP_PUSH1, 0x00,   // offset
        OP_APPROVE,
        // Second APPROVE(0x0) — should revert
        OP_PUSH1, 0x00,
        OP_PUSH1, 0x00,
        OP_PUSH1, 0x00,
        OP_APPROVE,
    ]);

    let _db = TestDB::new()
        .with_account(SENDER_ADDR, U256::from(10u64.pow(18)), 0, Bytes::new())
        .with_account(VALIDATOR_ADDR, U256::zero(), 0, double_approve_code);

    let _tx = make_frame_tx(SENDER_ADDR, 0, vec![
        frame(FrameMode::Verify, VALIDATOR_ADDR, 100_000, Bytes::new()),
    ]);

    // TODO: Execute and assert:
    // - The second APPROVE causes a revert
    // - Since VERIFY frame failed, tx is invalid
}
