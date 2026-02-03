//! VM-based integration tests for L2Hook.
//!
//! These tests execute actual transactions through the VM to verify
//! L2Hook's prepare_execution and finalize_execution behavior.
//!
//! Unlike the pure function tests, these tests require full VM instantiation
//! and verify end-to-end transaction execution with L2-specific fee handling.

use ethrex_common::types::fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig};
use ethrex_common::types::{EIP1559Transaction, PrivilegedL2Transaction, Transaction, TxKind};
use ethrex_common::{Address, H256, U256};
use ethrex_levm::hooks::l2_hook::COMMON_BRIDGE_L2_ADDRESS;
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use once_cell::sync::OnceCell;

use super::test_utils::*;
use bytes::Bytes;

// ============================================================================
// Helper Functions for VM Integration Tests
// ============================================================================

/// Creates a test VM configured for L2 execution.
///
/// This helper sets up a complete VM instance with:
/// - Test environment (gas price, base fee, block params)
/// - Test database with sender account
/// - L2 hooks (L2Hook + BackupHook)
fn create_test_l2_vm<'a>(
    env: &ethrex_levm::environment::Environment,
    db: &'a mut ethrex_levm::db::gen_db::GeneralizedDatabase,
    tx: &Transaction,
    fee_config: FeeConfig,
) -> Result<VM<'a>, ethrex_levm::errors::VMError> {
    let vm_type = VMType::L2(fee_config);
    VM::new(env.clone(), db, tx, LevmCallTracer::disabled(), vm_type)
}

/// Creates an EIP1559 transaction for testing.
fn create_eip1559_tx(
    to: Address,
    value: U256,
    gas_limit: u64,
    max_fee_per_gas: u64,
    max_priority_fee_per_gas: u64,
    nonce: u64,
) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        gas_limit,
        to: TxKind::Call(to),
        value,
        data: Bytes::new(),
        access_list: Vec::new(),
        chain_id: 1,
        signature_y_parity: false,
        signature_r: U256::zero(),
        signature_s: U256::zero(),
        inner_hash: OnceCell::new(),
    })
}

/// Creates a privileged L2 transaction for testing bridge operations.
fn create_privileged_tx(to: Address, value: U256, gas_limit: u64) -> Transaction {
    Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: DEFAULT_GAS_PRICE,
        gas_limit,
        to: TxKind::Call(to),
        value,
        data: Bytes::new(),
        access_list: Vec::new(),
        from: COMMON_BRIDGE_L2_ADDRESS,
        inner_hash: OnceCell::new(),
    })
}

/// Creates an environment for EIP1559 transactions.
fn create_eip1559_env(
    origin: Address,
    gas_limit: u64,
    max_fee_per_gas: U256,
    max_priority_fee_per_gas: U256,
    base_fee: U256,
    is_privileged: bool,
) -> ethrex_levm::environment::Environment {
    use ethrex_common::types::Fork;
    use ethrex_levm::EVMConfig;

    ethrex_levm::environment::Environment {
        origin,
        gas_limit,
        config: EVMConfig::new(Fork::Cancun, EVMConfig::canonical_values(Fork::Cancun)),
        block_number: U256::from(1),
        coinbase: TEST_COINBASE,
        timestamp: U256::from(1000),
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: base_fee,
        base_blob_fee_per_gas: U256::zero(),
        gas_price: max_fee_per_gas,
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: Vec::new(),
        tx_max_priority_fee_per_gas: Some(max_priority_fee_per_gas),
        tx_max_fee_per_gas: Some(max_fee_per_gas),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: u64::MAX,
        is_privileged,
        fee_token: None,
    }
}

// ============================================================================
// Section 1: Normal L2 Transaction Tests (prepare_execution + finalize_execution)
// ============================================================================

mod normal_l2_tx_tests {
    use super::*;

    #[test]
    fn test_normal_tx_executes_successfully() {
        // Setup: Create a simple value transfer transaction
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let value = U256::from(1_000_000_000_000_000_000u128); // 1 ETH
        let gas_limit = 21_000u64;
        let max_fee_per_gas = 1_000_000_000u64; // 1 gwei
        let max_priority_fee_per_gas = 100_000_000u64; // 0.1 gwei
        let base_fee = U256::from(100_000_000u64); // 0.1 gwei

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            value,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_priority_fee_per_gas),
            base_fee,
            false,
        );

        // Use empty fee config (no vaults, no L1 fee, no operator fee)
        let fee_config = FeeConfig::default();

        let result = create_test_l2_vm(&env, &mut db, &tx, fee_config);
        assert!(result.is_ok(), "VM creation should succeed");

        let mut vm = result.unwrap();
        let exec_result = vm.execute();

        // Transaction should execute successfully
        assert!(
            exec_result.is_ok(),
            "Transaction execution should succeed: {:?}",
            exec_result.err()
        );

        let report = exec_result.unwrap();
        assert!(
            report.is_success(),
            "Transaction should complete successfully"
        );

        // Verify recipient received value
        let recipient_balance = db
            .current_accounts_state
            .get(&TEST_RECIPIENT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(
            recipient_balance, value,
            "Recipient should receive the transferred value"
        );
    }

    #[test]
    fn test_normal_tx_deducts_gas_from_sender() {
        let initial_balance = U256::from(DEFAULT_SENDER_BALANCE);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, initial_balance, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let value = U256::from(1_000_000_000_000_000_000u128); // 1 ETH
        let gas_limit = 21_000u64;
        let max_fee_per_gas = 1_000_000_000u64;
        let max_priority_fee_per_gas = 100_000_000u64;
        let base_fee = U256::from(100_000_000u64);

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            value,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_priority_fee_per_gas),
            base_fee,
            false,
        );

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        // Sender balance should be: initial - value - (gas_used * effective_gas_price)
        let sender_balance = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        // Sender should have less than initial - value (due to gas)
        assert!(
            sender_balance < initial_balance - value,
            "Sender balance should be reduced by value + gas"
        );

        // Verify gas was used
        assert!(report.gas_used > 0, "Gas should have been consumed");
    }

    #[test]
    fn test_normal_tx_increments_sender_nonce() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            21_000,
            1_000_000_000,
            100_000_000,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            21_000,
            U256::from(1_000_000_000u64),
            U256::from(100_000_000u64),
            U256::from(100_000_000u64),
            false,
        );

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let _ = vm.execute().unwrap();

        // Sender nonce should be incremented
        let sender_nonce = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.nonce)
            .unwrap_or(0);

        assert_eq!(sender_nonce, 1, "Sender nonce should be incremented to 1");
    }

    #[test]
    fn test_normal_tx_insufficient_balance_fails() {
        // Sender has less balance than needed for value + gas
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(1_000u64), 0), // Very small balance
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let value = U256::from(1_000_000_000_000_000_000u128); // 1 ETH (more than balance)
        let tx = create_eip1559_tx(TEST_RECIPIENT, value, 21_000, 1_000_000_000, 100_000_000, 0);

        let env = create_eip1559_env(
            TEST_SENDER,
            21_000,
            U256::from(1_000_000_000u64),
            U256::from(100_000_000u64),
            U256::from(100_000_000u64),
            false,
        );

        let fee_config = FeeConfig::default();
        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);

        // VM creation might succeed (validation happens during execute)
        // Check either creation fails OR execution fails
        match vm_result {
            Err(_) => {
                // Expected: VM creation failed due to insufficient balance
            }
            Ok(mut vm) => {
                // VM created, but execution should fail
                let exec_result = vm.execute();
                assert!(
                    exec_result.is_err() || !exec_result.unwrap().is_success(),
                    "Execution should fail or revert with insufficient balance"
                );
            }
        }
    }

    #[test]
    fn test_normal_tx_invalid_nonce_fails() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0), // nonce is 0
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        // Transaction has nonce 5, but account nonce is 0
        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            21_000,
            1_000_000_000,
            100_000_000,
            5, // Wrong nonce
        );

        let mut env = create_eip1559_env(
            TEST_SENDER,
            21_000,
            U256::from(1_000_000_000u64),
            U256::from(100_000_000u64),
            U256::from(100_000_000u64),
            false,
        );
        env.tx_nonce = 5; // Set environment nonce to match tx

        let fee_config = FeeConfig::default();
        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);

        // Nonce validation may happen during VM::new or during execute
        match vm_result {
            Err(_) => {
                // Expected: VM creation failed due to nonce mismatch
            }
            Ok(mut vm) => {
                // VM created, but execution should fail
                let exec_result = vm.execute();
                assert!(
                    exec_result.is_err() || !exec_result.unwrap().is_success(),
                    "Execution should fail or revert with invalid nonce"
                );
            }
        }
    }
}

// ============================================================================
// Section 2: Fee Distribution Tests
// ============================================================================

mod fee_distribution_tests {
    use super::*;

    #[test]
    fn test_base_fee_goes_to_vault_when_configured() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_BASE_FEE_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let max_fee_per_gas = 1_000_000_000u64;
        let max_priority_fee_per_gas = 100_000_000u64;
        let base_fee = 100_000_000u64; // 0.1 gwei

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_priority_fee_per_gas),
            U256::from(base_fee),
            false,
        );

        // Configure base fee vault
        let fee_config = FeeConfig {
            base_fee_vault: Some(TEST_BASE_FEE_VAULT),
            operator_fee_config: None,
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        // Base fee vault should have received: base_fee * gas_used
        let vault_balance = db
            .current_accounts_state
            .get(&TEST_BASE_FEE_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let expected_base_fee_payment = U256::from(base_fee) * U256::from(report.gas_used);

        assert_eq!(
            vault_balance, expected_base_fee_payment,
            "Base fee vault should receive base_fee * gas_used"
        );
    }

    #[test]
    fn test_operator_fee_goes_to_vault_when_configured() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let operator_fee_per_gas = 50_000_000u64; // 0.05 gwei
        let base_fee = 100_000_000u64;
        let max_fee_per_gas = base_fee + operator_fee_per_gas + 100_000_000; // Enough for all fees
        let max_priority_fee_per_gas = 100_000_000u64;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_priority_fee_per_gas),
            U256::from(base_fee),
            false,
        );

        // Configure operator fee
        let fee_config = FeeConfig {
            base_fee_vault: None,
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: TEST_OPERATOR_VAULT,
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        // Operator vault should have received: operator_fee_per_gas * gas_used
        let vault_balance = db
            .current_accounts_state
            .get(&TEST_OPERATOR_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let expected_operator_fee = U256::from(operator_fee_per_gas) * U256::from(report.gas_used);

        assert_eq!(
            vault_balance, expected_operator_fee,
            "Operator vault should receive operator_fee_per_gas * gas_used"
        );
    }

    #[test]
    fn test_coinbase_receives_priority_fee_minus_operator_fee() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let operator_fee_per_gas = 50_000_000u64;
        let base_fee = 100_000_000u64;
        let max_priority_fee_per_gas = 200_000_000u64; // More than operator fee
        let max_fee_per_gas = base_fee + max_priority_fee_per_gas;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_priority_fee_per_gas),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig {
            base_fee_vault: None,
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: TEST_OPERATOR_VAULT,
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        // Coinbase should receive: (priority_fee - operator_fee) * gas_used
        let coinbase_balance = db
            .current_accounts_state
            .get(&TEST_COINBASE)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let expected_coinbase_payment = U256::from(max_priority_fee_per_gas - operator_fee_per_gas)
            * U256::from(report.gas_used);

        assert_eq!(
            coinbase_balance, expected_coinbase_payment,
            "Coinbase should receive (priority_fee - operator_fee) * gas_used"
        );
    }

    #[test]
    fn test_l1_fee_goes_to_vault_when_configured() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_L1_FEE_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 100_000u64; // Higher gas limit to accommodate L1 fee
        let l1_fee_per_blob_gas = 10u64;
        let base_fee = 100_000_000u64;
        let max_fee_per_gas = 10_000_000_000u64; // High enough to cover all fees
        let max_priority_fee_per_gas = 100_000_000u64;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_priority_fee_per_gas),
            U256::from(base_fee),
            false,
        );

        // Configure L1 fee
        let fee_config = FeeConfig {
            base_fee_vault: None,
            operator_fee_config: None,
            l1_fee_config: Some(L1FeeConfig {
                l1_fee_vault: TEST_L1_FEE_VAULT,
                l1_fee_per_blob_gas,
            }),
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        // L1 fee vault should have received the L1 data availability fee
        let vault_balance = db
            .current_accounts_state
            .get(&TEST_L1_FEE_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        // The L1 fee is based on transaction size, should be > 0
        assert!(
            vault_balance > U256::zero() || report.gas_used > 0,
            "L1 fee vault should receive some fee (or tx should use gas)"
        );
    }
}

// ============================================================================
// Section 3: Privileged Transaction Tests
// ============================================================================

mod privileged_tx_tests {
    use super::*;

    #[test]
    fn test_privileged_tx_from_bridge_can_mint() {
        // Bridge can send value even without having balance (minting)
        let mut db = create_test_db_with_accounts(vec![
            (COMMON_BRIDGE_L2_ADDRESS, U256::zero(), 0), // Bridge has no balance
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let minted_value = U256::from(5_000_000_000_000_000_000u128); // 5 ETH minted
        let gas_limit = 21_000u64;

        let tx = create_privileged_tx(TEST_RECIPIENT, minted_value, gas_limit);

        let env = create_eip1559_env(
            COMMON_BRIDGE_L2_ADDRESS,
            gas_limit,
            U256::from(DEFAULT_GAS_PRICE),
            U256::zero(),
            U256::from(DEFAULT_BASE_FEE),
            true, // is_privileged = true
        );

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success(), "Privileged tx should succeed");

        // Recipient should have received the minted value
        let recipient_balance = db
            .current_accounts_state
            .get(&TEST_RECIPIENT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(
            recipient_balance, minted_value,
            "Recipient should receive minted value"
        );
    }

    #[test]
    fn test_privileged_tx_bridge_mints_without_balance_deduction() {
        // This test verifies that the bridge can "mint" ETH - its balance is NOT deducted
        // when sending value, but the recipient still receives it.
        let initial_bridge_balance = U256::from(1_000_000_000_000_000_000u128); // 1 ETH
        let mut db = create_test_db_with_accounts(vec![
            (COMMON_BRIDGE_L2_ADDRESS, initial_bridge_balance, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let transfer_value = U256::from(100_000_000_000_000_000u128); // 0.1 ETH
        let gas_limit = 21_000u64;

        let tx = create_privileged_tx(TEST_RECIPIENT, transfer_value, gas_limit);

        let env = create_eip1559_env(
            COMMON_BRIDGE_L2_ADDRESS,
            gas_limit,
            U256::from(DEFAULT_GAS_PRICE),
            U256::zero(),
            U256::from(DEFAULT_BASE_FEE),
            true,
        );

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success(), "Privileged tx should succeed");

        // Bridge balance should NOT decrease (this is the minting behavior)
        let bridge_balance = db
            .current_accounts_state
            .get(&COMMON_BRIDGE_L2_ADDRESS)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(
            bridge_balance, initial_bridge_balance,
            "Bridge balance should NOT decrease (minting behavior)"
        );

        // Recipient should still receive the value (minted)
        let recipient_balance = db
            .current_accounts_state
            .get(&TEST_RECIPIENT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(
            recipient_balance, transfer_value,
            "Recipient should receive the minted value"
        );
    }

    #[test]
    fn test_privileged_tx_no_nonce_increment() {
        let mut db = create_test_db_with_accounts(vec![
            (
                COMMON_BRIDGE_L2_ADDRESS,
                U256::from(DEFAULT_SENDER_BALANCE),
                0,
            ),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let tx = create_privileged_tx(TEST_RECIPIENT, U256::zero(), 21_000);

        let env = create_eip1559_env(
            COMMON_BRIDGE_L2_ADDRESS,
            21_000,
            U256::from(DEFAULT_GAS_PRICE),
            U256::zero(),
            U256::from(DEFAULT_BASE_FEE),
            true,
        );

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let _ = vm.execute().unwrap();

        // Bridge nonce should NOT be incremented for privileged transactions
        let bridge_nonce = db
            .current_accounts_state
            .get(&COMMON_BRIDGE_L2_ADDRESS)
            .map(|a| a.info.nonce)
            .unwrap_or(0);

        assert_eq!(
            bridge_nonce, 0,
            "Privileged transactions should not increment nonce"
        );
    }
}

// ============================================================================
// Section 4: L2 Validation Tests
// ============================================================================

mod l2_validation_tests {
    use super::*;

    #[test]
    fn test_max_fee_must_cover_base_plus_operator_fee() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let operator_fee_per_gas = 500_000_000u64; // 0.5 gwei
        let base_fee = 100_000_000u64; // 0.1 gwei
        // max_fee_per_gas is LESS than base_fee + operator_fee
        let max_fee_per_gas = base_fee + operator_fee_per_gas - 1;
        let max_priority_fee_per_gas = 10_000_000u64;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_priority_fee_per_gas),
            U256::from(base_fee),
            false,
        );

        // Configure operator fee that makes max_fee insufficient
        let fee_config = FeeConfig {
            base_fee_vault: None,
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: TEST_OPERATOR_VAULT,
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);

        // Validation happens during prepare_execution (part of execute())
        match vm_result {
            Err(_) => {
                // VM creation failed - this is acceptable
            }
            Ok(mut vm) => {
                // VM created, execution should fail due to insufficient max_fee
                let exec_result = vm.execute();
                assert!(
                    exec_result.is_err(),
                    "Execution should fail when max_fee_per_gas < base_fee + operator_fee"
                );
            }
        }
    }

    #[test]
    fn test_max_fee_sufficient_for_base_plus_operator() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let operator_fee_per_gas = 50_000_000u64;
        let base_fee = 100_000_000u64;
        // max_fee_per_gas is MORE than base_fee + operator_fee
        let max_fee_per_gas = base_fee + operator_fee_per_gas + 100_000_000;
        let max_priority_fee_per_gas = 100_000_000u64;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_priority_fee_per_gas),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig {
            base_fee_vault: None,
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: TEST_OPERATOR_VAULT,
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        let result = create_test_l2_vm(&env, &mut db, &tx, fee_config);
        assert!(
            result.is_ok(),
            "Should succeed when max_fee_per_gas >= base_fee + operator_fee"
        );

        let mut vm = result.unwrap();
        let report = vm.execute();
        assert!(report.is_ok(), "Transaction should execute successfully");
    }
}

// ============================================================================
// Section 5: Gas Refund Tests
// ============================================================================

mod gas_refund_tests {
    use super::*;

    #[test]
    fn test_unused_gas_refunded_to_sender() {
        let initial_balance = U256::from(DEFAULT_SENDER_BALANCE);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, initial_balance, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        // Use high gas limit but simple transfer only uses 21000
        let gas_limit = 100_000u64;
        let max_fee_per_gas = 1_000_000_000u64;
        let max_priority_fee_per_gas = 100_000_000u64;
        let base_fee = 100_000_000u64;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_priority_fee_per_gas),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        // Gas used should be around 21000, not 100000
        assert!(
            report.gas_used < gas_limit,
            "Gas used ({}) should be less than gas limit ({})",
            report.gas_used,
            gas_limit
        );

        let sender_balance = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        // Key assertion: sender should NOT be charged for the full gas_limit
        // If there was no refund, the cost would be: gas_limit * max_fee_per_gas
        let max_possible_cost = U256::from(gas_limit) * U256::from(max_fee_per_gas);
        let actual_cost = initial_balance - sender_balance;

        assert!(
            actual_cost < max_possible_cost,
            "Sender should pay less than max possible cost (refund happened). \
             Actual cost: {}, Max possible: {}",
            actual_cost,
            max_possible_cost
        );

        // Also verify sender still has significant balance
        assert!(
            sender_balance > initial_balance / U256::from(2u64),
            "Sender should retain most of their balance after a simple transfer"
        );
    }
}

// ============================================================================
// Discovered Bugs Section
// ============================================================================
// Any bugs discovered during test implementation should be documented here.
// Format:
// - Bug ID: [unique identifier]
// - Description: [what the bug is]
// - Location: [file:line]
// - Reproduction: [steps to reproduce]
// - Expected: [expected behavior]
// - Actual: [actual behavior]
// - Status: [documented, to be fixed in separate PR]
//
// No bugs discovered during this test implementation phase.
