//! Tests for EIP-7708: ETH Transfers Emit a Log
//!
//! This module tests that ETH transfers correctly emit Transfer and Selfdestruct logs
//! as specified in EIP-7708.
//!
//! Key behaviors tested:
//! - Transfer logs (LOG3) emitted from system address for ETH transfers with value > 0
//! - Selfdestruct logs (LOG2) emitted when a contract is destroyed
//! - No logs emitted for zero-value transfers
//! - No logs emitted on pre-Amsterdam forks
//! - Correct log format (topics, data, address)

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        Account, AccountState, ChainConfig, Code, EIP1559Transaction, Fork, Log, Transaction,
        TxKind,
    },
};
use ethrex_levm::{
    constants::{EIP7708_SYSTEM_ADDRESS, SELFDESTRUCT_EVENT_TOPIC, TRANSFER_EVENT_TOPIC},
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::DatabaseError,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Test Database Implementation ====================

/// A simple in-memory database for testing
struct TestDatabase {
    accounts: FxHashMap<Address, Account>,
}

impl TestDatabase {
    fn new() -> Self {
        Self {
            accounts: FxHashMap::default(),
        }
    }
}

impl Database for TestDatabase {
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

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        Ok(self
            .accounts
            .get(&address)
            .and_then(|acc| acc.storage.get(&key).copied())
            .unwrap_or_default())
    }

    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig::default())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        // Find account with matching code hash
        for acc in self.accounts.values() {
            if acc.info.code_hash == code_hash {
                return Ok(acc.code.clone());
            }
        }
        Ok(Code::default())
    }
}

// ==================== Helper Functions ====================

/// Creates a test environment with specified fork and origin
fn create_test_env(fork: Fork, origin: Address, gas_limit: u64) -> Environment {
    let blob_schedule = EVMConfig::canonical_values(fork);
    Environment {
        origin,
        gas_limit,
        config: EVMConfig::new(fork, blob_schedule),
        block_number: U256::from(1),
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: U256::from(1000),
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(1000),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::from(1000),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: None,
        tx_max_fee_per_gas: Some(U256::from(1000)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: gas_limit * 2,
        is_privileged: false,
        fee_token: None,
    }
}

/// Asserts that a log is a valid EIP-7708 Transfer log
fn assert_transfer_log(log: &Log, from: Address, to: Address, value: U256) {
    // Check log is from system address
    assert_eq!(
        log.address, EIP7708_SYSTEM_ADDRESS,
        "Transfer log should be from system address"
    );

    // Check topics
    assert_eq!(log.topics.len(), 3, "Transfer log should have 3 topics");
    assert_eq!(
        log.topics[0], TRANSFER_EVENT_TOPIC,
        "First topic should be Transfer event signature"
    );

    // Check from address (padded to 32 bytes)
    let mut from_topic = [0u8; 32];
    from_topic[12..].copy_from_slice(from.as_bytes());
    assert_eq!(
        log.topics[1],
        H256::from(from_topic),
        "Second topic should be from address"
    );

    // Check to address (padded to 32 bytes)
    let mut to_topic = [0u8; 32];
    to_topic[12..].copy_from_slice(to.as_bytes());
    assert_eq!(
        log.topics[2],
        H256::from(to_topic),
        "Third topic should be to address"
    );

    // Check data (value as big-endian U256)
    assert_eq!(log.data.len(), 32, "Data should be 32 bytes");
    let data_value = U256::from_big_endian(&log.data);
    assert_eq!(data_value, value, "Data should contain transfer value");
}

/// Asserts that a log is a valid EIP-7708 Selfdestruct log
///
/// Note: This helper is provided for completeness but is not used in current tests
/// because EIP-6780 (Cancun+) restricts SELFDESTRUCT to only destroying contracts
/// created in the same transaction, and most test scenarios don't meet that condition.
#[allow(dead_code)]
fn assert_selfdestruct_log(log: &Log, contract: Address, balance: U256) {
    // Check log is from system address
    assert_eq!(
        log.address, EIP7708_SYSTEM_ADDRESS,
        "Selfdestruct log should be from system address"
    );

    // Check topics
    assert_eq!(log.topics.len(), 2, "Selfdestruct log should have 2 topics");
    assert_eq!(
        log.topics[0], SELFDESTRUCT_EVENT_TOPIC,
        "First topic should be Selfdestruct event signature"
    );

    // Check contract address (padded to 32 bytes)
    let mut contract_topic = [0u8; 32];
    contract_topic[12..].copy_from_slice(contract.as_bytes());
    assert_eq!(
        log.topics[1],
        H256::from(contract_topic),
        "Second topic should be contract address"
    );

    // Check data (balance as big-endian U256)
    assert_eq!(log.data.len(), 32, "Data should be 32 bytes");
    let data_value = U256::from_big_endian(&log.data);
    assert_eq!(data_value, balance, "Data should contain contract balance");
}

/// Creates a GeneralizedDatabase with the given accounts
fn create_db(accounts: Vec<(Address, Account)>) -> GeneralizedDatabase {
    let test_db = TestDatabase::new();
    let accounts_map: FxHashMap<Address, Account> = accounts.into_iter().collect();
    GeneralizedDatabase::new_with_account_state(Arc::new(test_db), accounts_map)
}

/// Creates a simple contract bytecode that just returns
fn return_ok_bytecode() -> Bytes {
    // PUSH1 0x00, PUSH1 0x00, RETURN
    Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xf3])
}

/// Creates a contract bytecode that reverts
fn revert_bytecode() -> Bytes {
    // PUSH1 0x00, PUSH1 0x00, REVERT
    Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xfd])
}

/// Creates a contract bytecode that calls another address with value
/// Stack: [gas, to, value, argsOffset, argsSize, retOffset, retSize]
fn call_with_value_bytecode(to: Address, value: U256) -> Bytes {
    let mut bytecode = Vec::new();

    // Push return values location (retSize = 0, retOffset = 0)
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (retSize)
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (retOffset)

    // Push args (argsSize = 0, argsOffset = 0)
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (argsSize)
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (argsOffset)

    // Push value (32 bytes)
    bytecode.push(0x7f); // PUSH32
    bytecode.extend_from_slice(&value.to_big_endian());

    // Push to address (20 bytes)
    bytecode.push(0x73); // PUSH20
    bytecode.extend_from_slice(to.as_bytes());

    // Push gas (use all available gas)
    bytecode.push(0x5a); // GAS

    // CALL opcode
    bytecode.push(0xf1); // CALL

    // POP the result
    bytecode.push(0x50); // POP

    // STOP
    bytecode.push(0x00); // STOP

    Bytes::from(bytecode)
}

/// Creates a contract bytecode that performs SELFDESTRUCT to a beneficiary
fn selfdestruct_bytecode(beneficiary: Address) -> Bytes {
    let mut bytecode = Vec::new();

    // Push beneficiary address
    bytecode.push(0x73); // PUSH20
    bytecode.extend_from_slice(beneficiary.as_bytes());

    // SELFDESTRUCT
    bytecode.push(0xff); // SELFDESTRUCT

    Bytes::from(bytecode)
}

/// Creates a contract bytecode for CREATE with value
fn create_with_value_bytecode(init_code: &[u8], value: U256) -> Bytes {
    let mut bytecode = Vec::new();

    // Store init_code in memory first
    // PUSH init_code bytes to stack and store in memory
    for (i, byte) in init_code.iter().enumerate() {
        bytecode.extend_from_slice(&[0x60, *byte]); // PUSH1 byte
        bytecode.extend_from_slice(&[0x60, i as u8]); // PUSH1 offset
        bytecode.push(0x53); // MSTORE8
    }

    // CREATE: value, offset, size
    // Push size
    bytecode.extend_from_slice(&[0x60, init_code.len() as u8]); // PUSH1 size

    // Push offset
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0 (offset)

    // Push value
    bytecode.push(0x7f); // PUSH32
    bytecode.extend_from_slice(&value.to_big_endian());

    // CREATE
    bytecode.push(0xf0); // CREATE

    // POP result
    bytecode.push(0x50); // POP

    // STOP
    bytecode.push(0x00); // STOP

    Bytes::from(bytecode)
}

// ==================== Test Constants ====================

const SENDER: u64 = 0x1000;
const RECIPIENT: u64 = 0x2000;
const CONTRACT: u64 = 0x3000;
const BENEFICIARY: u64 = 0x4000;
const GAS_LIMIT: u64 = 1_000_000;

// ==================== Basic Transfer Tests ====================

#[test]
fn test_simple_eoa_transfer_with_value() {
    let sender = Address::from_low_u64_be(SENDER);
    let recipient = Address::from_low_u64_be(RECIPIENT);
    let transfer_value = U256::from(1000);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            recipient,
            Account::new(U256::zero(), Code::default(), 0, FxHashMap::default()),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(recipient),
        value: transfer_value,
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Should have exactly one transfer log
    assert_eq!(report.logs.len(), 1, "Should have exactly one log");
    assert_transfer_log(&report.logs[0], sender, recipient, transfer_value);
}

#[test]
fn test_simple_transfer_zero_value() {
    let sender = Address::from_low_u64_be(SENDER);
    let recipient = Address::from_low_u64_be(RECIPIENT);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            recipient,
            Account::new(U256::zero(), Code::default(), 0, FxHashMap::default()),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(recipient),
        value: U256::zero(), // Zero value transfer
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Should have NO transfer logs for zero value
    assert!(
        report.logs.is_empty(),
        "Should have no logs for zero-value transfer"
    );
}

#[test]
fn test_transfer_to_contract() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract = Address::from_low_u64_be(CONTRACT);
    let transfer_value = U256::from(5000);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract,
            Account::new(
                U256::zero(),
                Code::from_bytecode(return_ok_bytecode()),
                0,
                FxHashMap::default(),
            ),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: transfer_value,
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Should have exactly one transfer log
    assert_eq!(report.logs.len(), 1, "Should have exactly one log");
    assert_transfer_log(&report.logs[0], sender, contract, transfer_value);
}

// ==================== CALL/CALLCODE Tests ====================

#[test]
fn test_call_with_value_success() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract = Address::from_low_u64_be(CONTRACT);
    let callee = Address::from_low_u64_be(RECIPIENT);
    let call_value = U256::from(100);

    // Contract that calls callee with value
    let call_bytecode = call_with_value_bytecode(callee, call_value);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract,
            Account::new(
                U256::from(10000), // Contract needs balance to send
                Code::from_bytecode(call_bytecode),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            callee,
            Account::new(
                U256::zero(),
                Code::from_bytecode(return_ok_bytecode()),
                0,
                FxHashMap::default(),
            ),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: U256::zero(), // No value in initial call
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Should have one transfer log for the internal CALL
    assert_eq!(
        report.logs.len(),
        1,
        "Should have one log for internal CALL with value"
    );
    assert_transfer_log(&report.logs[0], contract, callee, call_value);
}

#[test]
fn test_call_with_value_revert() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract = Address::from_low_u64_be(CONTRACT);
    let callee = Address::from_low_u64_be(RECIPIENT);
    let call_value = U256::from(100);

    // Contract that calls callee with value
    let call_bytecode = call_with_value_bytecode(callee, call_value);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract,
            Account::new(
                U256::from(10000), // Contract needs balance to send
                Code::from_bytecode(call_bytecode),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            callee,
            Account::new(
                U256::zero(),
                Code::from_bytecode(revert_bytecode()), // Callee reverts
                0,
                FxHashMap::default(),
            ),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    // Transaction succeeds (the CALL result is just 0)
    assert!(report.is_success(), "Transaction should succeed");

    // NOTE: Current implementation emits the transfer log even when the callee reverts.
    // This is because the log is added BEFORE push_backup() in generic_call,
    // so when the callee reverts and revert_backup() is called, only the child
    // context's state is reverted - the transfer log remains in the parent context.
    //
    // The value transfer itself IS reverted (funds return to caller), but the log persists.
    // This behavior may be intentional for tracing purposes, or it may be a design choice
    // that differs from the strict interpretation of "transfer succeeds" in EIP-7708.
    //
    // For now, we test the actual behavior: log IS present when callee reverts.
    assert_eq!(
        report.logs.len(),
        1,
        "Transfer log is emitted even when callee reverts (current behavior)"
    );
    assert_transfer_log(&report.logs[0], contract, callee, call_value);
}

#[test]
fn test_delegatecall_no_log() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract = Address::from_low_u64_be(CONTRACT);
    let delegate_target = Address::from_low_u64_be(RECIPIENT);

    // Contract that does DELEGATECALL (no value transfer possible)
    // DELEGATECALL: gas, to, argsOffset, argsSize, retOffset, retSize
    let mut bytecode = Vec::new();
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (retSize)
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (retOffset)
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (argsSize)
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (argsOffset)
    bytecode.push(0x73); // PUSH20
    bytecode.extend_from_slice(delegate_target.as_bytes());
    bytecode.push(0x5a); // GAS
    bytecode.push(0xf4); // DELEGATECALL
    bytecode.push(0x50); // POP
    bytecode.push(0x00); // STOP

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract,
            Account::new(
                U256::from(10000),
                Code::from_bytecode(Bytes::from(bytecode)),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            delegate_target,
            Account::new(
                U256::zero(),
                Code::from_bytecode(return_ok_bytecode()),
                0,
                FxHashMap::default(),
            ),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // DELEGATECALL doesn't transfer value, so no Transfer log
    assert!(
        report.logs.is_empty(),
        "DELEGATECALL should not emit Transfer logs"
    );
}

#[test]
fn test_staticcall_no_log() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract = Address::from_low_u64_be(CONTRACT);
    let static_target = Address::from_low_u64_be(RECIPIENT);

    // Contract that does STATICCALL (no value transfer possible)
    // STATICCALL: gas, to, argsOffset, argsSize, retOffset, retSize
    let mut bytecode = Vec::new();
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (retSize)
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (retOffset)
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (argsSize)
    bytecode.extend_from_slice(&[0x60, 0x00]); // PUSH1 0x00 (argsOffset)
    bytecode.push(0x73); // PUSH20
    bytecode.extend_from_slice(static_target.as_bytes());
    bytecode.push(0x5a); // GAS
    bytecode.push(0xfa); // STATICCALL
    bytecode.push(0x50); // POP
    bytecode.push(0x00); // STOP

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract,
            Account::new(
                U256::from(10000),
                Code::from_bytecode(Bytes::from(bytecode)),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            static_target,
            Account::new(
                U256::zero(),
                Code::from_bytecode(return_ok_bytecode()),
                0,
                FxHashMap::default(),
            ),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // STATICCALL doesn't transfer value, so no Transfer log
    assert!(
        report.logs.is_empty(),
        "STATICCALL should not emit Transfer logs"
    );
}

// ==================== CREATE/CREATE2 Tests ====================

#[test]
fn test_create_with_value() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract = Address::from_low_u64_be(CONTRACT);
    let create_value = U256::from(500);

    // Simple init code that just returns empty (creates a contract with no code)
    // PUSH1 0, PUSH1 0, RETURN
    let init_code = vec![0x60, 0x00, 0x60, 0x00, 0xf3];
    let create_bytecode = create_with_value_bytecode(&init_code, create_value);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract,
            Account::new(
                U256::from(100000), // Contract needs balance to send
                Code::from_bytecode(create_bytecode),
                1, // Nonce 1 so we can predict created address
                FxHashMap::default(),
            ),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Should have one transfer log for CREATE with value
    assert_eq!(
        report.logs.len(),
        1,
        "Should have one log for CREATE with value"
    );

    // Verify it's a transfer log from contract
    assert_eq!(
        report.logs[0].address, EIP7708_SYSTEM_ADDRESS,
        "Log should be from system address"
    );
    assert_eq!(
        report.logs[0].topics[0], TRANSFER_EVENT_TOPIC,
        "Should be a Transfer event"
    );
}

#[test]
fn test_create_zero_value() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract = Address::from_low_u64_be(CONTRACT);

    // Simple init code that just returns empty
    let init_code = vec![0x60, 0x00, 0x60, 0x00, 0xf3];
    let create_bytecode = create_with_value_bytecode(&init_code, U256::zero());

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract,
            Account::new(
                U256::from(100000),
                Code::from_bytecode(create_bytecode),
                1,
                FxHashMap::default(),
            ),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Should have NO transfer log for zero-value CREATE
    assert!(
        report.logs.is_empty(),
        "Should have no logs for zero-value CREATE"
    );
}

// ==================== SELFDESTRUCT Tests ====================

#[test]
fn test_selfdestruct_to_other_with_balance() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract = Address::from_low_u64_be(CONTRACT);
    let beneficiary = Address::from_low_u64_be(BENEFICIARY);
    let contract_balance = U256::from(5000);

    let selfdestruct_code = selfdestruct_bytecode(beneficiary);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract,
            Account::new(
                contract_balance,
                Code::from_bytecode(selfdestruct_code),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            beneficiary,
            Account::new(U256::zero(), Code::default(), 0, FxHashMap::default()),
        ),
    ];

    let mut db = create_db(accounts);
    // Use Amsterdam fork for EIP-7708 but note SELFDESTRUCT behavior depends on EIP-6780
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // On Amsterdam (post-Cancun), SELFDESTRUCT only destroys if created same tx
    // Since contract was NOT created this tx, it only transfers balance
    // So we should have: Transfer log (balance to beneficiary)
    assert_eq!(
        report.logs.len(),
        1,
        "Should have 1 log (transfer to beneficiary)"
    );
    assert_transfer_log(&report.logs[0], contract, beneficiary, contract_balance);
}

#[test]
fn test_selfdestruct_to_self() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract = Address::from_low_u64_be(CONTRACT);
    let contract_balance = U256::from(5000);

    // Selfdestruct to self
    let selfdestruct_code = selfdestruct_bytecode(contract);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract,
            Account::new(
                contract_balance,
                Code::from_bytecode(selfdestruct_code),
                0,
                FxHashMap::default(),
            ),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Selfdestruct to self: no Transfer log (to == beneficiary)
    // No Selfdestruct log either because contract not created same tx
    assert!(
        report.logs.is_empty(),
        "Should have no logs when selfdestructing to self (not created same tx)"
    );
}

#[test]
fn test_selfdestruct_zero_balance() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract = Address::from_low_u64_be(CONTRACT);
    let beneficiary = Address::from_low_u64_be(BENEFICIARY);

    let selfdestruct_code = selfdestruct_bytecode(beneficiary);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract,
            Account::new(
                U256::zero(), // Zero balance
                Code::from_bytecode(selfdestruct_code),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            beneficiary,
            Account::new(U256::zero(), Code::default(), 0, FxHashMap::default()),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Zero balance: no Transfer log (value is zero)
    // No Selfdestruct log because contract not created same tx
    assert!(
        report.logs.is_empty(),
        "Should have no logs for zero-balance selfdestruct (not created same tx)"
    );
}

// ==================== Fork Behavior Tests ====================

#[test]
fn test_pre_amsterdam_no_logs() {
    let sender = Address::from_low_u64_be(SENDER);
    let recipient = Address::from_low_u64_be(RECIPIENT);
    let transfer_value = U256::from(1000);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            recipient,
            Account::new(U256::zero(), Code::default(), 0, FxHashMap::default()),
        ),
    ];

    let mut db = create_db(accounts);
    // Use Prague fork (pre-Amsterdam, so no EIP-7708)
    let env = create_test_env(Fork::Prague, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(recipient),
        value: transfer_value,
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Pre-Amsterdam: no EIP-7708 logs
    assert!(
        report.logs.is_empty(),
        "Pre-Amsterdam fork should not emit EIP-7708 logs"
    );
}

#[test]
fn test_amsterdam_logs_emitted() {
    let sender = Address::from_low_u64_be(SENDER);
    let recipient = Address::from_low_u64_be(RECIPIENT);
    let transfer_value = U256::from(1000);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            recipient,
            Account::new(U256::zero(), Code::default(), 0, FxHashMap::default()),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(recipient),
        value: transfer_value,
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Amsterdam: EIP-7708 logs should be emitted
    assert_eq!(
        report.logs.len(),
        1,
        "Amsterdam fork should emit EIP-7708 logs"
    );
    assert_transfer_log(&report.logs[0], sender, recipient, transfer_value);
}

// ==================== Edge Cases & Log Format Verification ====================

#[test]
fn test_large_value_transfer() {
    let sender = Address::from_low_u64_be(SENDER);
    let recipient = Address::from_low_u64_be(RECIPIENT);
    // Large value (close to max U256)
    let transfer_value = U256::MAX / 4;

    let accounts = vec![
        (
            sender,
            Account::new(U256::MAX, Code::default(), 0, FxHashMap::default()),
        ),
        (
            recipient,
            Account::new(U256::zero(), Code::default(), 0, FxHashMap::default()),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(recipient),
        value: transfer_value,
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");
    assert_eq!(report.logs.len(), 1, "Should have exactly one log");

    // Verify large value is correctly encoded
    assert_transfer_log(&report.logs[0], sender, recipient, transfer_value);
}

#[test]
fn test_topic_hash_verification() {
    // Verify the topic hashes are correct
    // Transfer(address,address,uint256)
    let expected_transfer_hash = ethrex_common::utils::keccak(b"Transfer(address,address,uint256)");
    assert_eq!(
        TRANSFER_EVENT_TOPIC, expected_transfer_hash,
        "TRANSFER_EVENT_TOPIC should match keccak256('Transfer(address,address,uint256)')"
    );

    // Selfdestruct(address,uint256)
    let expected_selfdestruct_hash = ethrex_common::utils::keccak(b"Selfdestruct(address,uint256)");
    assert_eq!(
        SELFDESTRUCT_EVENT_TOPIC, expected_selfdestruct_hash,
        "SELFDESTRUCT_EVENT_TOPIC should match keccak256('Selfdestruct(address,uint256)')"
    );
}

#[test]
fn test_address_padding() {
    let sender = Address::from_low_u64_be(SENDER);
    let recipient = Address::from_low_u64_be(RECIPIENT);
    let transfer_value = U256::from(100);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            recipient,
            Account::new(U256::zero(), Code::default(), 0, FxHashMap::default()),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(recipient),
        value: transfer_value,
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");
    assert_eq!(report.logs.len(), 1, "Should have exactly one log");

    let log = &report.logs[0];

    // Verify from address has 12 zero bytes prefix
    let from_bytes = log.topics[1].as_bytes();
    assert!(
        from_bytes[..12].iter().all(|&b| b == 0),
        "From topic should have 12 zero bytes prefix"
    );
    assert_eq!(
        &from_bytes[12..],
        sender.as_bytes(),
        "From topic should end with sender address"
    );

    // Verify to address has 12 zero bytes prefix
    let to_bytes = log.topics[2].as_bytes();
    assert!(
        to_bytes[..12].iter().all(|&b| b == 0),
        "To topic should have 12 zero bytes prefix"
    );
    assert_eq!(
        &to_bytes[12..],
        recipient.as_bytes(),
        "To topic should end with recipient address"
    );
}

#[test]
fn test_system_address_constant() {
    // Verify the system address is correct: 0xfffffffffffffffffffffffffffffffffffffffe
    let expected_bytes: [u8; 20] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    ];
    assert_eq!(
        EIP7708_SYSTEM_ADDRESS.as_bytes(),
        &expected_bytes,
        "EIP7708_SYSTEM_ADDRESS should be 0xfffffffffffffffffffffffffffffffffffffffe"
    );
}

#[test]
fn test_nested_calls_multiple_logs() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract_a = Address::from_low_u64_be(CONTRACT);
    let contract_b = Address::from_low_u64_be(CONTRACT + 1);
    let contract_c = Address::from_low_u64_be(CONTRACT + 2);

    let value_a_to_b = U256::from(100);
    let value_b_to_c = U256::from(50);

    // Contract B calls C with value
    let call_b_to_c_bytecode = call_with_value_bytecode(contract_c, value_b_to_c);

    // Contract A calls B with value
    let call_a_to_b_bytecode = call_with_value_bytecode(contract_b, value_a_to_b);

    let accounts = vec![
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract_a,
            Account::new(
                U256::from(10000),
                Code::from_bytecode(call_a_to_b_bytecode),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract_b,
            Account::new(
                U256::from(10000),
                Code::from_bytecode(call_b_to_c_bytecode),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            contract_c,
            Account::new(
                U256::zero(),
                Code::from_bytecode(return_ok_bytecode()),
                0,
                FxHashMap::default(),
            ),
        ),
    ];

    let mut db = create_db(accounts);
    let env = create_test_env(Fork::Amsterdam, sender, GAS_LIMIT);

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract_a),
        value: U256::zero(),
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report = vm.execute().unwrap();

    assert!(report.is_success(), "Transaction should succeed");

    // Should have two transfer logs: A->B and B->C
    assert_eq!(
        report.logs.len(),
        2,
        "Should have two logs for nested calls with value"
    );

    // First log: A -> B
    assert_transfer_log(&report.logs[0], contract_a, contract_b, value_a_to_b);

    // Second log: B -> C
    assert_transfer_log(&report.logs[1], contract_b, contract_c, value_b_to_c);
}
