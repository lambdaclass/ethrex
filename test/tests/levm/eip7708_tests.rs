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
    constants::{EMPTY_TRIE_HASH, SYSTEM_ADDRESS},
    types::{
        Account, AccountState, ChainConfig, Code, CodeMetadata, EIP1559Transaction, Fork, Log,
        Transaction, TxKind,
    },
};
use ethrex_levm::{
    constants::{SELFDESTRUCT_EVENT_TOPIC, TRANSFER_EVENT_TOPIC},
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::{DatabaseError, ExecutionReport},
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
                    length: acc.code.bytecode.len() as u64,
                });
            }
        }
        Ok(CodeMetadata { length: 0 })
    }
}

// ==================== Test Constants ====================

const DEFAULT_BALANCE: u64 = 10_000_000_000;
const SENDER: u64 = 0x1000;
const RECIPIENT: u64 = 0x2000;
const CONTRACT: u64 = 0x3000;
const BENEFICIARY: u64 = 0x4000;
const GAS_LIMIT: u64 = 1_000_000;

// ==================== Account Helpers ====================

fn eoa(balance: U256) -> Account {
    Account::new(balance, Code::default(), 0, FxHashMap::default())
}

fn contract(code: Bytes) -> Account {
    Account::new(
        U256::zero(),
        Code::from_bytecode(code),
        0,
        FxHashMap::default(),
    )
}

fn contract_funded(balance: U256, code: Bytes, nonce: u64) -> Account {
    Account::new(
        balance,
        Code::from_bytecode(code),
        nonce,
        FxHashMap::default(),
    )
}

// ==================== TestBuilder ====================

struct TestBuilder {
    accounts: Vec<(Address, Account)>,
    fork: Fork,
    sender: Address,
    to: Address,
    value: U256,
}

impl TestBuilder {
    fn new() -> Self {
        Self {
            accounts: Vec::new(),
            fork: Fork::Amsterdam,
            sender: Address::from_low_u64_be(SENDER),
            to: Address::from_low_u64_be(RECIPIENT),
            value: U256::zero(),
        }
    }

    fn fork(mut self, fork: Fork) -> Self {
        self.fork = fork;
        self
    }

    fn account(mut self, addr: Address, acc: Account) -> Self {
        self.accounts.push((addr, acc));
        self
    }

    fn to(mut self, addr: Address) -> Self {
        self.to = addr;
        self
    }

    fn value(mut self, v: U256) -> Self {
        self.value = v;
        self
    }

    fn execute(self) -> ExecutionReport {
        let test_db = TestDatabase::new();
        let accounts_map: FxHashMap<Address, Account> = self.accounts.into_iter().collect();
        let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(test_db), accounts_map);

        let blob_schedule = EVMConfig::canonical_values(self.fork);
        let env = Environment {
            origin: self.sender,
            gas_limit: GAS_LIMIT,
            config: EVMConfig::new(self.fork, blob_schedule),
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
            block_gas_limit: GAS_LIMIT * 2,
            is_privileged: false,
            fee_token: None,
        };

        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(self.to),
            value: self.value,
            data: Bytes::new(),
            gas_limit: GAS_LIMIT,
            max_fee_per_gas: 1000,
            max_priority_fee_per_gas: 1,
            ..Default::default()
        });

        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1).unwrap();
        vm.execute().unwrap()
    }
}

// ==================== Bytecode Helpers ====================

fn return_ok_bytecode() -> Bytes {
    Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xf3]) // PUSH1 0, PUSH1 0, RETURN
}

fn revert_bytecode() -> Bytes {
    Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xfd]) // PUSH1 0, PUSH1 0, REVERT
}

fn call_with_value_bytecode(to: Address, value: U256) -> Bytes {
    let mut bytecode = Vec::new();
    bytecode.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]); // retSize, retOffset, argsSize, argsOffset
    bytecode.push(0x7f); // PUSH32 value
    bytecode.extend_from_slice(&value.to_big_endian());
    bytecode.push(0x73); // PUSH20 to
    bytecode.extend_from_slice(to.as_bytes());
    bytecode.push(0x5a); // GAS
    bytecode.push(0xf1); // CALL
    bytecode.push(0x50); // POP
    bytecode.push(0x00); // STOP
    Bytes::from(bytecode)
}

/// Creates bytecode for DELEGATECALL (0xf4) or STATICCALL (0xfa)
fn call_no_value_bytecode(target: Address, opcode: u8) -> Bytes {
    let mut bytecode = Vec::new();
    bytecode.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]); // retSize, retOffset, argsSize, argsOffset
    bytecode.push(0x73); // PUSH20 target
    bytecode.extend_from_slice(target.as_bytes());
    bytecode.push(0x5a); // GAS
    bytecode.push(opcode);
    bytecode.push(0x50); // POP
    bytecode.push(0x00); // STOP
    Bytes::from(bytecode)
}

/// Creates bytecode for a contract that CALLs itself (ADDRESS) with a given value
fn call_self_with_value_bytecode(value: U256) -> Bytes {
    let mut bytecode = Vec::new();
    bytecode.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]); // retSize, retOffset, argsSize, argsOffset
    bytecode.push(0x7f); // PUSH32 value
    bytecode.extend_from_slice(&value.to_big_endian());
    bytecode.push(0x30); // ADDRESS - pushes current contract address
    bytecode.push(0x5a); // GAS
    bytecode.push(0xf1); // CALL
    bytecode.push(0x50); // POP
    bytecode.push(0x00); // STOP
    Bytes::from(bytecode)
}

fn selfdestruct_bytecode(beneficiary: Address) -> Bytes {
    let mut bytecode = Vec::new();
    bytecode.push(0x73); // PUSH20
    bytecode.extend_from_slice(beneficiary.as_bytes());
    bytecode.push(0xff); // SELFDESTRUCT
    Bytes::from(bytecode)
}

fn create_with_value_bytecode(init_code: &[u8], value: U256) -> Bytes {
    let mut bytecode = Vec::new();
    for (i, byte) in init_code.iter().enumerate() {
        bytecode.extend_from_slice(&[0x60, *byte, 0x60, i as u8, 0x53]); // PUSH1 byte, PUSH1 offset, MSTORE8
    }
    bytecode.extend_from_slice(&[0x60, init_code.len() as u8, 0x60, 0x00]); // size, offset
    bytecode.push(0x7f); // PUSH32 value
    bytecode.extend_from_slice(&value.to_big_endian());
    bytecode.push(0xf0); // CREATE
    bytecode.push(0x50); // POP
    bytecode.push(0x00); // STOP
    Bytes::from(bytecode)
}

// ==================== Assertion Helpers ====================

fn assert_transfer_log(log: &Log, from: Address, to: Address, value: U256) {
    assert_eq!(
        log.address, SYSTEM_ADDRESS,
        "Log should be from system address"
    );
    assert_eq!(log.topics.len(), 3, "Transfer log should have 3 topics");
    assert_eq!(
        log.topics[0], TRANSFER_EVENT_TOPIC,
        "First topic should be Transfer event"
    );

    let mut from_topic = [0u8; 32];
    from_topic[12..].copy_from_slice(from.as_bytes());
    assert_eq!(
        log.topics[1],
        H256::from(from_topic),
        "Second topic should be from address"
    );

    let mut to_topic = [0u8; 32];
    to_topic[12..].copy_from_slice(to.as_bytes());
    assert_eq!(
        log.topics[2],
        H256::from(to_topic),
        "Third topic should be to address"
    );

    assert_eq!(log.data.len(), 32, "Data should be 32 bytes");
    assert_eq!(
        U256::from_big_endian(&log.data),
        value,
        "Data should contain transfer value"
    );
}

#[allow(dead_code)]
fn assert_selfdestruct_log(log: &Log, contract: Address, balance: U256) {
    assert_eq!(
        log.address, SYSTEM_ADDRESS,
        "Log should be from system address"
    );
    assert_eq!(log.topics.len(), 2, "Selfdestruct log should have 2 topics");
    assert_eq!(
        log.topics[0], SELFDESTRUCT_EVENT_TOPIC,
        "First topic should be Selfdestruct event"
    );

    let mut contract_topic = [0u8; 32];
    contract_topic[12..].copy_from_slice(contract.as_bytes());
    assert_eq!(
        log.topics[1],
        H256::from(contract_topic),
        "Second topic should be contract address"
    );

    assert_eq!(log.data.len(), 32, "Data should be 32 bytes");
    assert_eq!(
        U256::from_big_endian(&log.data),
        balance,
        "Data should contain contract balance"
    );
}

// ==================== Parameterized Test Helpers ====================

fn run_simple_transfer_test(fork: Fork, transfer_value: U256, expect_log: bool) {
    let sender = Address::from_low_u64_be(SENDER);
    let recipient = Address::from_low_u64_be(RECIPIENT);

    let report = TestBuilder::new()
        .fork(fork)
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(recipient, eoa(U256::zero()))
        .to(recipient)
        .value(transfer_value)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    if expect_log {
        assert_eq!(report.logs.len(), 1, "Should have exactly one log");
        assert_transfer_log(&report.logs[0], sender, recipient, transfer_value);
    } else {
        assert!(report.logs.is_empty(), "Should have no logs");
    }
}

fn run_selfdestruct_test(contract_balance: U256, beneficiary: Address, expect_log: bool) {
    let sender = Address::from_low_u64_be(SENDER);
    let contract_addr = Address::from_low_u64_be(CONTRACT);
    let selfdestruct_code = selfdestruct_bytecode(beneficiary);

    let mut builder = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            contract_addr,
            contract_funded(contract_balance, selfdestruct_code, 0),
        )
        .to(contract_addr);

    if beneficiary != contract_addr {
        builder = builder.account(beneficiary, eoa(U256::zero()));
    }

    let report = builder.execute();
    assert!(report.is_success(), "Transaction should succeed");

    if expect_log {
        assert_eq!(report.logs.len(), 1, "Should have 1 log");
        assert_transfer_log(
            &report.logs[0],
            contract_addr,
            beneficiary,
            contract_balance,
        );
    } else {
        assert!(report.logs.is_empty(), "Should have no logs");
    }
}

// ==================== Basic Transfer Tests ====================

#[test]
fn test_simple_eoa_transfer_with_value() {
    run_simple_transfer_test(Fork::Amsterdam, U256::from(1000), true);
}

#[test]
fn test_simple_transfer_zero_value() {
    run_simple_transfer_test(Fork::Amsterdam, U256::zero(), false);
}

#[test]
fn test_transfer_to_contract() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract_addr = Address::from_low_u64_be(CONTRACT);
    let transfer_value = U256::from(5000);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(contract_addr, contract(return_ok_bytecode()))
        .to(contract_addr)
        .value(transfer_value)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    assert_eq!(report.logs.len(), 1, "Should have exactly one log");
    assert_transfer_log(&report.logs[0], sender, contract_addr, transfer_value);
}

#[test]
fn test_self_transfer_no_log() {
    // EIP-7708: Transfer logs should only be emitted for transfers to DIFFERENT accounts
    // A transaction where origin == to (self-transfer) should NOT emit a log
    let sender = Address::from_low_u64_be(SENDER);
    let transfer_value = U256::from(1000);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .to(sender) // Self-transfer: sender sends to themselves
        .value(transfer_value)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    assert!(
        report.logs.is_empty(),
        "Self-transfer should NOT emit a Transfer log"
    );
}

// ==================== CALL/CALLCODE Tests ====================

#[test]
fn test_call_with_value_success() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract_addr = Address::from_low_u64_be(CONTRACT);
    let callee = Address::from_low_u64_be(RECIPIENT);
    let call_value = U256::from(100);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            contract_addr,
            contract_funded(
                U256::from(10000),
                call_with_value_bytecode(callee, call_value),
                0,
            ),
        )
        .account(callee, contract(return_ok_bytecode()))
        .to(contract_addr)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    assert_eq!(
        report.logs.len(),
        1,
        "Should have one log for internal CALL with value"
    );
    assert_transfer_log(&report.logs[0], contract_addr, callee, call_value);
}

#[test]
fn test_call_with_value_revert() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract_addr = Address::from_low_u64_be(CONTRACT);
    let callee = Address::from_low_u64_be(RECIPIENT);
    let call_value = U256::from(100);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            contract_addr,
            contract_funded(
                U256::from(10000),
                call_with_value_bytecode(callee, call_value),
                0,
            ),
        )
        .account(callee, contract(revert_bytecode()))
        .to(contract_addr)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    // EIP-7708: When callee reverts, the transfer log should also revert.
    // The log is added AFTER push_backup(), so it correctly reverts with the child context.
    assert!(
        report.logs.is_empty(),
        "Transfer log should NOT be emitted when callee reverts"
    );
}

#[test]
fn test_top_level_transaction_revert_no_transfer_log() {
    // When a top-level transaction with value reverts, the EIP-7708 Transfer log
    // should NOT be included in the transaction receipt.
    let sender = Address::from_low_u64_be(SENDER);
    let contract_addr = Address::from_low_u64_be(CONTRACT);
    let transfer_value = U256::from(1000);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(contract_addr, contract(revert_bytecode()))
        .to(contract_addr)
        .value(transfer_value)
        .execute();

    // Transaction should fail (revert)
    assert!(!report.is_success(), "Transaction should revert");
    // No logs should be emitted when transaction reverts
    assert!(
        report.logs.is_empty(),
        "Transfer log should NOT be emitted when top-level transaction reverts"
    );
}

#[test]
fn test_call_self_with_value_no_log() {
    // EIP-7708: Transfer logs should only be emitted for CALLs to DIFFERENT accounts
    // A contract CALLing itself with value should NOT emit a Transfer log
    let sender = Address::from_low_u64_be(SENDER);
    let contract_addr = Address::from_low_u64_be(CONTRACT);
    let call_value = U256::from(100);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            contract_addr,
            contract_funded(
                U256::from(10000),
                call_self_with_value_bytecode(call_value),
                0,
            ),
        )
        .to(contract_addr)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    // No Transfer log should be emitted because the contract is CALLing itself
    assert!(
        report.logs.is_empty(),
        "CALL to self should NOT emit a Transfer log"
    );
}

#[test]
fn test_delegatecall_no_log() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract_addr = Address::from_low_u64_be(CONTRACT);
    let delegate_target = Address::from_low_u64_be(RECIPIENT);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            contract_addr,
            contract_funded(
                U256::from(10000),
                call_no_value_bytecode(delegate_target, 0xf4),
                0,
            ),
        )
        .account(delegate_target, contract(return_ok_bytecode()))
        .to(contract_addr)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    assert!(
        report.logs.is_empty(),
        "DELEGATECALL should not emit Transfer logs"
    );
}

#[test]
fn test_staticcall_no_log() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract_addr = Address::from_low_u64_be(CONTRACT);
    let static_target = Address::from_low_u64_be(RECIPIENT);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            contract_addr,
            contract_funded(
                U256::from(10000),
                call_no_value_bytecode(static_target, 0xfa),
                0,
            ),
        )
        .account(static_target, contract(return_ok_bytecode()))
        .to(contract_addr)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    assert!(
        report.logs.is_empty(),
        "STATICCALL should not emit Transfer logs"
    );
}

// ==================== CREATE/CREATE2 Tests ====================

#[test]
fn test_create_with_value() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract_addr = Address::from_low_u64_be(CONTRACT);
    let create_value = U256::from(500);
    let init_code = vec![0x60, 0x00, 0x60, 0x00, 0xf3]; // PUSH1 0, PUSH1 0, RETURN

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            contract_addr,
            contract_funded(
                U256::from(100000),
                create_with_value_bytecode(&init_code, create_value),
                1,
            ),
        )
        .to(contract_addr)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    assert_eq!(
        report.logs.len(),
        1,
        "Should have one log for CREATE with value"
    );
    assert_eq!(
        report.logs[0].address, SYSTEM_ADDRESS,
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
    let contract_addr = Address::from_low_u64_be(CONTRACT);
    let init_code = vec![0x60, 0x00, 0x60, 0x00, 0xf3];

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            contract_addr,
            contract_funded(
                U256::from(100000),
                create_with_value_bytecode(&init_code, U256::zero()),
                1,
            ),
        )
        .to(contract_addr)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    assert!(
        report.logs.is_empty(),
        "Should have no logs for zero-value CREATE"
    );
}

// ==================== SELFDESTRUCT Tests ====================

#[test]
fn test_selfdestruct_to_other_with_balance() {
    run_selfdestruct_test(
        U256::from(5000),
        Address::from_low_u64_be(BENEFICIARY),
        true,
    );
}

#[test]
fn test_selfdestruct_to_self() {
    run_selfdestruct_test(U256::from(5000), Address::from_low_u64_be(CONTRACT), false);
}

#[test]
fn test_selfdestruct_zero_balance() {
    run_selfdestruct_test(U256::zero(), Address::from_low_u64_be(BENEFICIARY), false);
}

// ==================== Fork Behavior Tests ====================

#[test]
fn test_pre_amsterdam_no_logs() {
    run_simple_transfer_test(Fork::Prague, U256::from(1000), false);
}

#[test]
fn test_amsterdam_logs_emitted() {
    run_simple_transfer_test(Fork::Amsterdam, U256::from(1000), true);
}

// ==================== Edge Cases & Log Format Verification ====================

#[test]
fn test_large_value_transfer() {
    let sender = Address::from_low_u64_be(SENDER);
    let recipient = Address::from_low_u64_be(RECIPIENT);
    let transfer_value = U256::MAX / 4;

    let report = TestBuilder::new()
        .account(sender, eoa(U256::MAX))
        .account(recipient, eoa(U256::zero()))
        .to(recipient)
        .value(transfer_value)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    assert_eq!(report.logs.len(), 1, "Should have exactly one log");
    assert_transfer_log(&report.logs[0], sender, recipient, transfer_value);
}

#[test]
fn test_topic_hash_and_system_address_constants() {
    // Verify Transfer topic hash
    let expected_transfer_hash = ethrex_common::utils::keccak(b"Transfer(address,address,uint256)");
    assert_eq!(
        TRANSFER_EVENT_TOPIC, expected_transfer_hash,
        "TRANSFER_EVENT_TOPIC should match keccak256('Transfer(address,address,uint256)')"
    );

    // Verify Selfdestruct topic hash
    let expected_selfdestruct_hash = ethrex_common::utils::keccak(b"Selfdestruct(address,uint256)");
    assert_eq!(
        SELFDESTRUCT_EVENT_TOPIC, expected_selfdestruct_hash,
        "SELFDESTRUCT_EVENT_TOPIC should match keccak256('Selfdestruct(address,uint256)')"
    );

    // Verify system address
    let expected_bytes: [u8; 20] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    ];
    assert_eq!(
        SYSTEM_ADDRESS.as_bytes(),
        &expected_bytes,
        "SYSTEM_ADDRESS should be 0xfffffffffffffffffffffffffffffffffffffffe"
    );
}

#[test]
fn test_address_padding() {
    let sender = Address::from_low_u64_be(SENDER);
    let recipient = Address::from_low_u64_be(RECIPIENT);
    let transfer_value = U256::from(100);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(recipient, eoa(U256::zero()))
        .to(recipient)
        .value(transfer_value)
        .execute();

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
fn test_nested_calls_multiple_logs() {
    let sender = Address::from_low_u64_be(SENDER);
    let contract_a = Address::from_low_u64_be(CONTRACT);
    let contract_b = Address::from_low_u64_be(CONTRACT + 1);
    let contract_c = Address::from_low_u64_be(CONTRACT + 2);

    let value_a_to_b = U256::from(100);
    let value_b_to_c = U256::from(50);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            contract_a,
            contract_funded(
                U256::from(10000),
                call_with_value_bytecode(contract_b, value_a_to_b),
                0,
            ),
        )
        .account(
            contract_b,
            contract_funded(
                U256::from(10000),
                call_with_value_bytecode(contract_c, value_b_to_c),
                0,
            ),
        )
        .account(contract_c, contract(return_ok_bytecode()))
        .to(contract_a)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");
    assert_eq!(
        report.logs.len(),
        2,
        "Should have two logs for nested calls with value"
    );
    assert_transfer_log(&report.logs[0], contract_a, contract_b, value_a_to_b);
    assert_transfer_log(&report.logs[1], contract_b, contract_c, value_b_to_c);
}

/// Creates init code that immediately SELFDESTRUCTs to the given beneficiary.
/// Bytecode: PUSH20 beneficiary, SELFDESTRUCT
fn selfdestruct_init_code(beneficiary: Address) -> Vec<u8> {
    let mut code = Vec::new();
    code.push(0x73); // PUSH20
    code.extend_from_slice(beneficiary.as_bytes());
    code.push(0xff); // SELFDESTRUCT
    code
}

/// Creates bytecode that:
/// 1. Stores init_code in memory
/// 2. CREATEs a contract with create_value
/// 3. STOREs the created address at memory offset 200
/// 4. CALLs the created address with call_value
fn create_and_call_bytecode(init_code: &[u8], create_value: U256, call_value: U256) -> Bytes {
    let mut bytecode = Vec::new();

    // Store init_code in memory byte by byte
    for (i, byte) in init_code.iter().enumerate() {
        bytecode.extend_from_slice(&[0x60, *byte, 0x60, i as u8, 0x53]); // PUSH1 byte, PUSH1 offset, MSTORE8
    }

    // CREATE: stack needs [value, offset, size]
    // PUSH1 size, PUSH1 0 (offset), PUSH32 value
    bytecode.extend_from_slice(&[0x60, init_code.len() as u8, 0x60, 0x00]); // size, offset
    bytecode.push(0x7f); // PUSH32 value
    bytecode.extend_from_slice(&create_value.to_big_endian());
    bytecode.push(0xf0); // CREATE - leaves created address on stack

    // Store address at memory offset 200 for CALL
    bytecode.extend_from_slice(&[0x60, 200, 0x52]); // PUSH1 200, MSTORE

    // Now stack is empty, build CALL args
    // CALL: pops [gas, address, value, argsOffset, argsSize, retOffset, retSize]
    // Build stack (top to bottom): [gas, address, value, 0, 0, 0, 0]

    // Push in reverse order (they go to top):
    bytecode.extend_from_slice(&[0x60, 0x00]); // retSize = 0
    bytecode.extend_from_slice(&[0x60, 0x00]); // retOffset = 0
    bytecode.extend_from_slice(&[0x60, 0x00]); // argsSize = 0
    bytecode.extend_from_slice(&[0x60, 0x00]); // argsOffset = 0
    bytecode.push(0x7f); // PUSH32 call_value
    bytecode.extend_from_slice(&call_value.to_big_endian());
    bytecode.extend_from_slice(&[0x60, 200, 0x51]); // PUSH1 200, MLOAD (load address)
    bytecode.push(0x5a); // GAS
    bytecode.push(0xf1); // CALL
    bytecode.push(0x50); // POP (call result)
    bytecode.push(0x00); // STOP

    Bytes::from(bytecode)
}

/// When a contract created in the same transaction calls SELFDESTRUCT to a DIFFERENT address,
/// only a Transfer log should be emitted (not a Selfdestruct log).
/// Transfer and Selfdestruct logs are mutually exclusive per EIP-7708.
#[test]
fn test_created_contract_selfdestruct_to_other_only_transfer_log() {
    let sender = Address::from_low_u64_be(SENDER);
    let factory = Address::from_low_u64_be(CONTRACT);
    let beneficiary = Address::from_low_u64_be(BENEFICIARY);
    let create_value = U256::from(1000);

    // Init code that selfdestructs to beneficiary (different address)
    let init_code = selfdestruct_init_code(beneficiary);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            factory,
            contract_funded(
                U256::from(100000),
                create_with_value_bytecode(&init_code, create_value),
                1,
            ),
        )
        .account(beneficiary, eoa(U256::zero()))
        .to(factory)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");

    // Should have exactly 2 Transfer logs:
    // 1. Transfer(factory -> child, 1000) from CREATE
    // 2. Transfer(child -> beneficiary, 1000) from SELFDESTRUCT
    // NO Selfdestruct log should be emitted because beneficiary != child
    assert_eq!(
        report.logs.len(),
        2,
        "Should have exactly 2 logs (both Transfer, no Selfdestruct)"
    );

    // First log: CREATE transfer from factory to child
    assert_eq!(
        report.logs[0].topics[0], TRANSFER_EVENT_TOPIC,
        "First log should be Transfer event"
    );

    // Second log: SELFDESTRUCT transfer from child to beneficiary
    assert_eq!(
        report.logs[1].topics[0], TRANSFER_EVENT_TOPIC,
        "Second log should be Transfer event (not Selfdestruct)"
    );
    // Verify the second log goes to beneficiary
    let mut beneficiary_topic = [0u8; 32];
    beneficiary_topic[12..].copy_from_slice(beneficiary.as_bytes());
    assert_eq!(
        report.logs[1].topics[2],
        H256::from(beneficiary_topic),
        "Second Transfer log should go to beneficiary"
    );
}

/// When a contract created in the same transaction calls SELFDESTRUCT to ITSELF,
/// a Selfdestruct log should be emitted (balance is burned, not transferred).
#[test]
fn test_created_contract_selfdestruct_to_self_emits_selfdestruct_log() {
    let sender = Address::from_low_u64_be(SENDER);
    let factory = Address::from_low_u64_be(CONTRACT);
    let create_value = U256::from(1000);

    // The child contract address is deterministic based on factory address and nonce
    // Factory nonce is 1, so child = keccak256(rlp([factory, 1]))[12..]
    let child_address = ethrex_common::evm::calculate_create_address(factory, 1);

    // Init code that selfdestructs to itself (the child address)
    let init_code = selfdestruct_init_code(child_address);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            factory,
            contract_funded(
                U256::from(100000),
                create_with_value_bytecode(&init_code, create_value),
                1,
            ),
        )
        .to(factory)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");

    // Should have exactly 2 logs:
    // 1. Transfer(factory -> child, 1000) from CREATE
    // 2. Selfdestruct(child, 1000) from SELFDESTRUCT to self (balance burned)
    // NO Transfer log for the selfdestruct because beneficiary == child
    assert_eq!(
        report.logs.len(),
        2,
        "Should have exactly 2 logs (Transfer from CREATE, Selfdestruct from self-destruct)"
    );

    // First log: CREATE transfer from factory to child
    assert_eq!(
        report.logs[0].topics[0], TRANSFER_EVENT_TOPIC,
        "First log should be Transfer event"
    );
    // Verify child address in the transfer
    let mut child_topic = [0u8; 32];
    child_topic[12..].copy_from_slice(child_address.as_bytes());
    assert_eq!(
        report.logs[0].topics[2],
        H256::from(child_topic),
        "Transfer should go to child address"
    );

    // Second log: Selfdestruct log for the contract
    assert_eq!(
        report.logs[1].topics[0], SELFDESTRUCT_EVENT_TOPIC,
        "Second log should be Selfdestruct event"
    );
    assert_selfdestruct_log(&report.logs[1], child_address, create_value);
}

/// When a contract is flagged for SELFDESTRUCT and then receives ETH,
/// a Selfdestruct closure log should be emitted at end of transaction
/// for the non-zero balance remaining at account closure.
#[test]
fn test_eth_received_after_selfdestruct_emits_closure_log() {
    let sender = Address::from_low_u64_be(SENDER);
    let factory = Address::from_low_u64_be(CONTRACT);
    let beneficiary = Address::from_low_u64_be(BENEFICIARY);
    let create_value = U256::from(1000);
    let call_value = U256::from(500);

    // The child contract address
    let child_address = ethrex_common::evm::calculate_create_address(factory, 1);

    // Init code that selfdestructs to beneficiary (transferring away all balance)
    let init_code = selfdestruct_init_code(beneficiary);

    // Factory bytecode that:
    // 1. CREATEs child with 1000 wei (child selfdestructs to beneficiary immediately)
    // 2. CALLs child with 500 wei (child receives ETH after being flagged for destruction)
    let factory_code = create_and_call_bytecode(&init_code, create_value, call_value);

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            factory,
            contract_funded(U256::from(100000), factory_code, 1),
        )
        .account(beneficiary, eoa(U256::zero()))
        .to(factory)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");

    // Expected logs:
    // 1. Transfer(factory -> child, 1000) from CREATE
    // 2. Transfer(child -> beneficiary, 1000) from SELFDESTRUCT
    // 3. Transfer(factory -> child, 500) from CALL (child receives ETH after being flagged)
    // 4. Selfdestruct(child, 500) - closure log at end of tx (non-zero balance at destruction)
    assert_eq!(
        report.logs.len(),
        4,
        "Should have 4 logs: 2 Transfers from CREATE+SELFDESTRUCT, 1 Transfer from CALL, 1 Selfdestruct closure"
    );

    // First log: CREATE transfer
    assert_eq!(
        report.logs[0].topics[0], TRANSFER_EVENT_TOPIC,
        "First log should be Transfer (CREATE)"
    );
    assert_transfer_log(&report.logs[0], factory, child_address, create_value);

    // Second log: SELFDESTRUCT transfer to beneficiary
    assert_eq!(
        report.logs[1].topics[0], TRANSFER_EVENT_TOPIC,
        "Second log should be Transfer (SELFDESTRUCT to beneficiary)"
    );
    assert_transfer_log(&report.logs[1], child_address, beneficiary, create_value);

    // Third log: CALL transfer (ETH sent to child after it's flagged for destruction)
    assert_eq!(
        report.logs[2].topics[0], TRANSFER_EVENT_TOPIC,
        "Third log should be Transfer (CALL)"
    );
    assert_transfer_log(&report.logs[2], factory, child_address, call_value);

    // Fourth log: Selfdestruct closure log (emitted at end of tx for non-zero balance)
    assert_eq!(
        report.logs[3].topics[0], SELFDESTRUCT_EVENT_TOPIC,
        "Fourth log should be Selfdestruct (closure)"
    );
    assert_selfdestruct_log(&report.logs[3], child_address, call_value);
}

/// When multiple contracts are flagged for SELFDESTRUCT and receive ETH,
/// their closure logs should be emitted in lexicographical order of address.
#[test]
fn test_closure_logs_lexicographical_order() {
    // This test creates two contracts with predictable addresses and verifies
    // that their closure logs are emitted in lexicographical order.

    let sender = Address::from_low_u64_be(SENDER);
    let factory = Address::from_low_u64_be(CONTRACT);
    let beneficiary = Address::from_low_u64_be(BENEFICIARY);

    // Calculate child addresses based on factory nonce
    // First CREATE uses nonce 1, second uses nonce 2
    let child1 = ethrex_common::evm::calculate_create_address(factory, 1);
    let child2 = ethrex_common::evm::calculate_create_address(factory, 2);

    // Determine which address is lower (lexicographically first)
    let (lower_addr, higher_addr) = if child1 < child2 {
        (child1, child2)
    } else {
        (child2, child1)
    };

    // Create bytecode that:
    // 1. Creates child1 with 100 wei (selfdestructs to beneficiary)
    // 2. Creates child2 with 100 wei (selfdestructs to beneficiary)
    // 3. Calls child1 with 50 wei
    // 4. Calls child2 with 50 wei
    // Both children should have closure logs, in lexicographical order

    let init_code = selfdestruct_init_code(beneficiary);
    let create_value = U256::from(100);
    let call_value = U256::from(50);

    // Build complex factory bytecode
    let mut factory_code = Vec::new();

    // Store init_code in memory (same for both children)
    for (i, byte) in init_code.iter().enumerate() {
        factory_code.extend_from_slice(&[0x60, *byte, 0x60, i as u8, 0x53]);
    }

    // CREATE child1: stack needs [value, offset, size]
    factory_code.extend_from_slice(&[0x60, init_code.len() as u8, 0x60, 0x00]); // size, offset
    factory_code.push(0x7f);
    factory_code.extend_from_slice(&create_value.to_big_endian());
    factory_code.push(0xf0); // CREATE - leaves child1 address on stack

    // Store child1 at memory offset 100 for later use
    factory_code.extend_from_slice(&[0x60, 100, 0x52]); // PUSH1 100, MSTORE

    // Restore init_code in memory (it was overwritten by MSTORE)
    for (i, byte) in init_code.iter().enumerate() {
        factory_code.extend_from_slice(&[0x60, *byte, 0x60, i as u8, 0x53]);
    }

    // CREATE child2
    factory_code.extend_from_slice(&[0x60, init_code.len() as u8, 0x60, 0x00]);
    factory_code.push(0x7f);
    factory_code.extend_from_slice(&create_value.to_big_endian());
    factory_code.push(0xf0); // CREATE - leaves child2 address on stack

    // Store child2 at memory offset 132
    factory_code.extend_from_slice(&[0x60, 132, 0x52]); // PUSH1 132, MSTORE

    // CALL child1 with 50 wei
    // Load child1 from memory offset 100
    factory_code.extend_from_slice(&[
        0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60,
        0x00, // retSize, retOffset, argsSize, argsOffset
    ]);
    factory_code.push(0x7f);
    factory_code.extend_from_slice(&call_value.to_big_endian());
    factory_code.extend_from_slice(&[0x60, 100, 0x51]); // PUSH1 100, MLOAD (child1 address)
    factory_code.push(0x5a); // GAS
    factory_code.push(0xf1); // CALL
    factory_code.push(0x50); // POP result

    // CALL child2 with 50 wei
    factory_code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]);
    factory_code.push(0x7f);
    factory_code.extend_from_slice(&call_value.to_big_endian());
    factory_code.extend_from_slice(&[0x60, 132, 0x51]); // PUSH1 132, MLOAD (child2 address)
    factory_code.push(0x5a); // GAS
    factory_code.push(0xf1); // CALL
    factory_code.push(0x50); // POP result
    factory_code.push(0x00); // STOP

    let report = TestBuilder::new()
        .account(sender, eoa(U256::from(DEFAULT_BALANCE)))
        .account(
            factory,
            contract_funded(U256::from(100000), Bytes::from(factory_code), 1),
        )
        .account(beneficiary, eoa(U256::zero()))
        .to(factory)
        .execute();

    assert!(report.is_success(), "Transaction should succeed");

    // Expected logs (8 total):
    // 1. Transfer(factory -> child1, 100) from CREATE
    // 2. Transfer(child1 -> beneficiary, 100) from SELFDESTRUCT
    // 3. Transfer(factory -> child2, 100) from CREATE
    // 4. Transfer(child2 -> beneficiary, 100) from SELFDESTRUCT
    // 5. Transfer(factory -> child1, 50) from CALL
    // 6. Transfer(factory -> child2, 50) from CALL
    // 7. Selfdestruct(lower_addr, 50) - closure log in lex order
    // 8. Selfdestruct(higher_addr, 50) - closure log in lex order
    assert_eq!(report.logs.len(), 8, "Should have 8 logs");

    // The last two logs should be Selfdestruct closure logs in lexicographical order
    let log7 = &report.logs[6];
    let log8 = &report.logs[7];

    assert_eq!(
        log7.topics[0], SELFDESTRUCT_EVENT_TOPIC,
        "7th log should be Selfdestruct"
    );
    assert_eq!(
        log8.topics[0], SELFDESTRUCT_EVENT_TOPIC,
        "8th log should be Selfdestruct"
    );

    // Extract addresses from the logs
    let addr7 = Address::from_slice(&log7.topics[1].as_bytes()[12..]);
    let addr8 = Address::from_slice(&log8.topics[1].as_bytes()[12..]);

    assert_eq!(
        addr7, lower_addr,
        "First closure log should be for lexicographically lower address"
    );
    assert_eq!(
        addr8, higher_addr,
        "Second closure log should be for lexicographically higher address"
    );
}
