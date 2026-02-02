/// EIP-7702 Delegation Gas Tests
///
/// These tests verify the correct gas accounting for EIP-7702 delegated accounts.
///
/// Key insight: The delegation resolution gas cost (cold/warm access to delegated address)
/// should ONLY be charged during CALL opcodes, NOT during the initial transaction setup.
///
/// EIP-7702 specifies delegation resolution for "opcodes which get code" (CALL, CALLCODE,
/// DELEGATECALL, STATICCALL, EXTCODESIZE, EXTCODECOPY, EXTCODEHASH). The initial transaction
/// is not an opcode, so it shouldn't charge EIP-2929 access costs for delegation resolution.
///
/// The delegated address IS added to accessed_addresses (warming it for subsequent calls),
/// but without charging the cold access cost at transaction setup time.
use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, U256,
    constants::EMPTY_TRIE_HASH,
    types::{Account, BlockHeader, Code, Fork, LegacyTransaction, Transaction, TxKind},
};
use ethrex_levm::{
    EVMConfig, Environment,
    constants::SET_CODE_DELEGATION_BYTES,
    db::gen_db::GeneralizedDatabase,
    gas_cost::COLD_ADDRESS_ACCESS_COST,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use once_cell::sync::OnceCell;
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// Creates EIP-7702 delegation bytecode: 0xef0100 || address
fn create_delegation_code(target: Address) -> Bytes {
    let mut code = SET_CODE_DELEGATION_BYTES.to_vec();
    code.extend_from_slice(target.as_bytes());
    Bytes::from(code)
}

/// Simple bytecode that just returns (STOP opcode)
fn simple_return_code() -> Bytes {
    Bytes::from(vec![0x00]) // STOP
}

/// Creates an in-memory database with given accounts
fn setup_db(accounts: FxHashMap<Address, Account>) -> GeneralizedDatabase {
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());
    GeneralizedDatabase::new_with_account_state(Arc::new(store), accounts)
}

/// Creates a basic environment for Prague fork (EIP-7702 enabled)
fn create_environment(sender: Address, gas_limit: u64) -> Environment {
    Environment {
        origin: sender,
        gas_limit,
        gas_price: U256::from(1),
        block_gas_limit: u64::MAX,
        config: EVMConfig::new(Fork::Prague, EVMConfig::canonical_values(Fork::Prague)),
        ..Default::default()
    }
}

/// Helper to create a LegacyTransaction
fn create_legacy_tx(nonce: u64, gas: u64, to: Address, value: U256, data: Bytes) -> Transaction {
    Transaction::LegacyTransaction(LegacyTransaction {
        nonce,
        gas_price: U256::from(1),
        gas,
        to: TxKind::Call(to),
        value,
        data,
        v: U256::zero(),
        r: U256::zero(),
        s: U256::zero(),
        inner_hash: OnceCell::new(),
    })
}

/// Test: Initial transaction to delegated account does NOT charge cold access for delegation target
///
/// Setup:
/// - Account A (delegated): bytecode = 0xef0100 || B's address
/// - Account B (target): bytecode = STOP (0x00)
/// - Sender with enough balance
///
/// When a transaction is sent TO account A:
/// - The delegation is resolved and B's code is executed
/// - B is added to accessed_addresses (for warming)
/// - But the cold access cost (2600 gas) for B is NOT charged
///
/// This test verifies the gas consumed matches expected (without cold access cost).
#[test]
fn test_initial_tx_to_delegated_account_no_cold_access_charge() {
    let sender = Address::from_low_u64_be(1);
    let delegated_account = Address::from_low_u64_be(2); // Account A
    let target_account = Address::from_low_u64_be(3); // Account B

    // Create accounts
    let mut accounts = FxHashMap::default();

    // Sender with balance
    accounts.insert(
        sender,
        Account {
            info: ethrex_common::types::AccountInfo {
                balance: U256::from(10_000_000),
                nonce: 0,
                ..Default::default()
            },
            ..Default::default()
        },
    );

    // Delegated account (A) - points to target (B)
    let delegation_code = create_delegation_code(target_account);
    accounts.insert(
        delegated_account,
        Account {
            code: Code::from_bytecode(delegation_code),
            ..Default::default()
        },
    );

    // Target account (B) - simple code
    accounts.insert(
        target_account,
        Account {
            code: Code::from_bytecode(simple_return_code()),
            ..Default::default()
        },
    );

    let mut db = setup_db(accounts);
    let gas_limit = 100_000u64;
    let env = create_environment(sender, gas_limit);

    let tx = create_legacy_tx(0, gas_limit, delegated_account, U256::zero(), Bytes::new());

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
        .expect("Failed to create VM");

    let result = vm.execute().expect("Execution failed");

    // The gas used should NOT include COLD_ADDRESS_ACCESS_COST for the target account.
    // Base transaction cost is 21000, plus minimal execution cost.
    // If cold access was incorrectly charged, we'd see an extra 2600 gas.
    let gas_used = result.gas_used;

    // Verify gas is reasonable (less than base + cold access cost buffer)
    // If the bug was present, gas_used would be >= 21000 + 2600 = 23600
    // With correct behavior, it should be around 21000 (just intrinsic gas)
    assert!(
        gas_used < 21000 + COLD_ADDRESS_ACCESS_COST,
        "Gas used ({}) suggests cold access was incorrectly charged. \
         Expected less than {} (21000 intrinsic + {} cold access)",
        gas_used,
        21000 + COLD_ADDRESS_ACCESS_COST,
        COLD_ADDRESS_ACCESS_COST
    );

    // Verify the transaction succeeded (delegation resolution worked)
    assert!(
        result.is_success(),
        "Transaction should succeed with delegated code execution"
    );
}

/// Test: Verify delegated address is added to accessed_addresses after resolution
///
/// After a transaction to a delegated account, the delegation target should be
/// in accessed_addresses (warming it for subsequent operations within the same tx).
#[test]
fn test_delegated_address_is_warmed_after_resolution() {
    let sender = Address::from_low_u64_be(1);
    let delegated_account = Address::from_low_u64_be(2);
    let target_account = Address::from_low_u64_be(3);

    let mut accounts = FxHashMap::default();

    accounts.insert(
        sender,
        Account {
            info: ethrex_common::types::AccountInfo {
                balance: U256::from(10_000_000),
                nonce: 0,
                ..Default::default()
            },
            ..Default::default()
        },
    );

    // Delegated account points to target
    accounts.insert(
        delegated_account,
        Account {
            code: Code::from_bytecode(create_delegation_code(target_account)),
            ..Default::default()
        },
    );

    // Target account with STOP
    accounts.insert(
        target_account,
        Account {
            code: Code::from_bytecode(simple_return_code()),
            ..Default::default()
        },
    );

    let mut db = setup_db(accounts);
    let gas_limit = 100_000u64;
    let env = create_environment(sender, gas_limit);

    let tx = create_legacy_tx(0, gas_limit, delegated_account, U256::zero(), Bytes::new());

    let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
        .expect("Failed to create VM");

    let result = vm.execute().expect("Execution failed");
    assert!(result.is_success());

    // After execution, target_account should be in accessed_addresses
    // This verifies the delegation resolution added it even without charging gas
    let is_accessed = vm.substate.is_address_accessed(&target_account);
    assert!(
        is_accessed,
        "Target account should be in accessed_addresses after delegation resolution"
    );
}

/// Test: Verify the delegation code format is correctly detected
#[test]
fn test_delegation_code_format() {
    let target = Address::from_low_u64_be(0xDEADBEEF);
    let code = create_delegation_code(target);

    // Should be exactly 23 bytes (3 prefix + 20 address)
    assert_eq!(code.len(), 23, "Delegation code should be 23 bytes");

    // Should start with 0xef0100
    assert_eq!(
        &code[0..3],
        &SET_CODE_DELEGATION_BYTES,
        "Should have EIP-7702 prefix"
    );

    // Should contain the target address
    let extracted_addr = Address::from_slice(&code[3..23]);
    assert_eq!(extracted_addr, target, "Should contain target address");
}
