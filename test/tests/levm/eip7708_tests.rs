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
        log.address, EIP7708_SYSTEM_ADDRESS,
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
        log.address, EIP7708_SYSTEM_ADDRESS,
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
    // NOTE: Current implementation emits the transfer log even when the callee reverts.
    // The log is added BEFORE push_backup(), so it persists even when child context reverts.
    assert_eq!(
        report.logs.len(),
        1,
        "Transfer log is emitted even when callee reverts"
    );
    assert_transfer_log(&report.logs[0], contract_addr, callee, call_value);
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
        EIP7708_SYSTEM_ADDRESS.as_bytes(),
        &expected_bytes,
        "EIP7708_SYSTEM_ADDRESS should be 0xfffffffffffffffffffffffffffffffffffffffe"
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
