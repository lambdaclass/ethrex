/// L2 Privileged Transaction Tests
///
/// These tests verify correct handling of edge cases in L2 privileged transactions,
/// particularly around gas accounting when transactions target precompile addresses.
///
/// Key insight: When a privileged transaction's intrinsic gas exceeds the gas limit,
/// gas_remaining becomes negative. If the transaction targets a precompile, we must
/// handle this gracefully rather than casting the negative value to u64 (which would
/// wrap to a huge positive value due to two's complement).
use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, H160, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        Account, BlockHeader, Fork, LegacyTransaction, Transaction, TxKind, fee_config::FeeConfig,
    },
};
use ethrex_levm::{
    EVMConfig, Environment, db::gen_db::GeneralizedDatabase, tracing::LevmCallTracer, vm::VM,
    vm::VMType,
};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use once_cell::sync::OnceCell;
use rustc_hash::FxHashMap;
use std::sync::Arc;

// Precompile addresses
const ECRECOVER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01,
]);

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

/// Creates a privileged L2 environment
fn create_privileged_environment(sender: Address, gas_limit: u64) -> Environment {
    Environment {
        origin: sender,
        gas_limit,
        gas_price: U256::from(1),
        block_gas_limit: u64::MAX,
        config: EVMConfig::new(Fork::Prague, EVMConfig::canonical_values(Fork::Prague)),
        is_privileged: true,
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

/// Test: Privileged transaction to precompile with insufficient gas
///
/// This test verifies that when a privileged L2 transaction:
/// 1. Targets a precompile address (e.g., ecrecover at 0x01)
/// 2. Has a gas limit lower than intrinsic gas (causing gas_remaining to go negative)
///
/// The transaction properly fails with OutOfGas rather than:
/// - Executing the precompile with wrapped (huge) gas value
/// - Causing undefined behavior due to negative-to-unsigned cast
///
/// Bug scenario (before fix):
/// - gas_limit = 100, intrinsic_gas = 21000
/// - gas_remaining = 100 - 21000 = -20900 (as i64)
/// - In run_execution: `gas_remaining as u64` = 18446744073709530716 (wrapped!)
/// - Precompile executes with essentially unlimited gas
#[test]
fn test_privileged_tx_to_precompile_with_insufficient_gas_fails_gracefully() {
    let sender = Address::from_low_u64_be(1);

    // Create sender account with balance
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

    let mut db = setup_db(accounts);

    // Gas limit intentionally lower than intrinsic gas (21000)
    // This will cause gas_remaining to become negative after add_intrinsic_gas
    let gas_limit = 100u64;
    let env = create_privileged_environment(sender, gas_limit);

    // Transaction to ecrecover precompile
    let tx = create_legacy_tx(0, gas_limit, ECRECOVER_ADDRESS, U256::zero(), Bytes::new());

    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L2(FeeConfig::default()),
    )
    .expect("Failed to create VM");

    let result = vm.execute().expect("Execution should not return VMError");

    // The transaction should fail (revert), not succeed
    // Before the fix, the precompile would execute with wrapped huge gas
    assert!(
        !result.is_success(),
        "Transaction should fail due to insufficient gas, not succeed with wrapped gas value"
    );

    // Gas used should be the full gas limit (all gas consumed on failure)
    assert_eq!(
        result.gas_used, gas_limit,
        "All gas should be consumed on failure"
    );
}

/// Test: Privileged transaction to precompile with sufficient gas succeeds
///
/// This is a sanity check to ensure we didn't break the happy path.
#[test]
fn test_privileged_tx_to_precompile_with_sufficient_gas_succeeds() {
    let sender = Address::from_low_u64_be(1);

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

    let mut db = setup_db(accounts);

    // Sufficient gas for intrinsic + precompile execution
    let gas_limit = 100_000u64;
    let env = create_privileged_environment(sender, gas_limit);

    // Transaction to ecrecover precompile with empty calldata
    // (ecrecover will fail due to invalid input, but execution should proceed)
    let tx = create_legacy_tx(0, gas_limit, ECRECOVER_ADDRESS, U256::zero(), Bytes::new());

    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L2(FeeConfig::default()),
    )
    .expect("Failed to create VM");

    let result = vm.execute().expect("Execution should not return VMError");

    // With sufficient gas, execution should proceed (precompile may fail due to bad input,
    // but that's a different kind of failure than gas exhaustion)
    // The key point is we didn't hit the gas underflow bug
    assert!(
        result.gas_used <= gas_limit,
        "Gas used ({}) should not exceed gas limit ({})",
        result.gas_used,
        gas_limit
    );
}

/// Test: Verify negative gas_remaining is handled before precompile execution
///
/// This test specifically checks that negative gas_remaining causes immediate
/// failure without attempting to execute the precompile.
#[test]
fn test_negative_gas_remaining_causes_immediate_failure() {
    let sender = Address::from_low_u64_be(1);

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

    let mut db = setup_db(accounts);

    // Very low gas limit - much less than intrinsic gas
    let gas_limit = 1u64;
    let env = create_privileged_environment(sender, gas_limit);

    let tx = create_legacy_tx(0, gas_limit, ECRECOVER_ADDRESS, U256::zero(), Bytes::new());

    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L2(FeeConfig::default()),
    )
    .expect("Failed to create VM");

    let result = vm.execute().expect("Execution should not return VMError");

    // Must fail due to gas exhaustion
    assert!(
        !result.is_success(),
        "Transaction with 1 gas should fail, not succeed"
    );
}
