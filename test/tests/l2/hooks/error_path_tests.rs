//! Error path tests for L2Hook implementation.
//!
//! These tests verify error handling and edge cases:
//! - Privileged transaction forced reverts
//! - Validation failures
//! - Overflow/underflow handling
//! - Gas limit violations
//!
//! Testing error paths is critical for security.

use ethrex_common::types::fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig};
use ethrex_common::types::{PrivilegedL2Transaction, Transaction, TxKind};
use ethrex_common::{Address, H160, U256};
use ethrex_levm::hooks::l2_hook::COMMON_BRIDGE_L2_ADDRESS;
use once_cell::sync::OnceCell;

use super::test_utils::*;
use bytes::Bytes;

// ============================================================================
// Helper Functions
// ============================================================================

fn create_privileged_tx(from: Address, to: Address, value: U256, gas_limit: u64) -> Transaction {
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
        from,
        inner_hash: OnceCell::new(),
    })
}

/// Non-bridge privileged address for testing
const NON_BRIDGE_PRIVILEGED: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

// ============================================================================
// Section 1: Privileged Transaction Forced Revert Tests
// ============================================================================

mod privileged_forced_revert_tests {
    use super::*;

    #[test]
    fn test_non_bridge_privileged_insufficient_balance_forces_revert() {
        // Non-bridge privileged tx with insufficient balance should be forced to revert
        // via INVALID opcode, but tx is still included (not rejected)
        let small_balance = U256::from(100u64); // Very small balance
        let mut db = create_test_db_with_accounts(vec![
            (NON_BRIDGE_PRIVILEGED, small_balance, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let large_value = U256::from(1_000_000_000_000_000_000u128); // 1 ETH
        let gas_limit = 21_000u64;

        let tx = create_privileged_tx(
            NON_BRIDGE_PRIVILEGED,
            TEST_RECIPIENT,
            large_value,
            gas_limit,
        );

        let env = create_eip1559_env(
            NON_BRIDGE_PRIVILEGED,
            gas_limit,
            U256::from(DEFAULT_GAS_PRICE),
            U256::zero(),
            U256::from(DEFAULT_BASE_FEE),
            true,
        );

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        // Transaction should NOT succeed (forced revert)
        assert!(
            !report.is_success(),
            "Privileged tx with insufficient balance should revert"
        );

        // But execution should still complete (tx is included)
        // Recipient should NOT receive any value
        let recipient_balance = db
            .current_accounts_state
            .get(&TEST_RECIPIENT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(
            recipient_balance,
            U256::zero(),
            "Recipient should not receive value when privileged tx is forced to revert"
        );

        // Sender balance should be unchanged (value transfer was zeroed)
        let sender_balance = db
            .current_accounts_state
            .get(&NON_BRIDGE_PRIVILEGED)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(
            sender_balance, small_balance,
            "Sender balance should be unchanged when privileged tx reverts"
        );
    }

    #[test]
    fn test_bridge_with_zero_balance_can_still_mint() {
        // Bridge with zero balance should still be able to "mint" ETH
        let mut db = create_test_db_with_accounts(vec![
            (COMMON_BRIDGE_L2_ADDRESS, U256::zero(), 0), // Zero balance!
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let mint_value = U256::from(10_000_000_000_000_000_000u128); // 10 ETH
        let gas_limit = 21_000u64;

        let tx = create_privileged_tx(
            COMMON_BRIDGE_L2_ADDRESS,
            TEST_RECIPIENT,
            mint_value,
            gas_limit,
        );

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

        // Should succeed even with zero bridge balance
        assert!(report.is_success(), "Bridge minting should succeed");

        // Recipient should receive minted value
        let recipient_balance = db
            .current_accounts_state
            .get(&TEST_RECIPIENT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(
            recipient_balance, mint_value,
            "Recipient should receive minted ETH"
        );
    }

    #[test]
    fn test_privileged_tx_with_very_low_gas_reverts() {
        // Privileged tx with gas too low for intrinsic should revert
        let mut db = create_test_db_with_accounts(vec![
            (NON_BRIDGE_PRIVILEGED, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        // Gas limit below intrinsic (21000 for simple transfer)
        let gas_limit = 1000u64;

        let tx = create_privileged_tx(
            NON_BRIDGE_PRIVILEGED,
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
        );

        let env = create_eip1559_env(
            NON_BRIDGE_PRIVILEGED,
            gas_limit,
            U256::from(DEFAULT_GAS_PRICE),
            U256::zero(),
            U256::from(DEFAULT_BASE_FEE),
            true,
        );

        let fee_config = FeeConfig::default();
        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);

        // VM creation might fail or execution will fail
        match vm_result {
            Err(_) => {
                // Expected - intrinsic gas check failed
            }
            Ok(mut vm) => {
                let report = vm.execute().unwrap();
                // If we got here, tx should have reverted
                assert!(
                    !report.is_success(),
                    "Privileged tx with insufficient gas should revert"
                );
            }
        }
    }
}

// ============================================================================
// Section 2: Validation Failure Tests
// ============================================================================

mod validation_failure_tests {
    use super::*;

    #[test]
    fn test_max_fee_exactly_one_below_required_fails() {
        // Test boundary: max_fee = base + operator - 1 should fail
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let operator_fee = 50_000_000u64;
        // Exactly 1 wei below required
        let max_fee = base_fee + operator_fee - 1;
        let max_priority = operator_fee - 1;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee,
            max_priority,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee),
            U256::from(max_priority),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig {
            base_fee_vault: None,
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: TEST_OPERATOR_VAULT,
                operator_fee_per_gas: operator_fee,
            }),
            l1_fee_config: None,
        };

        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);

        match vm_result {
            Err(_) => {
                // Expected: validation failed
            }
            Ok(mut vm) => {
                let exec_result = vm.execute();
                assert!(
                    exec_result.is_err(),
                    "Should fail when max_fee is 1 below required"
                );
            }
        }
    }

    #[test]
    fn test_zero_max_fee_fails_validation() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let max_fee = 0u64; // Zero!
        let max_priority = 0u64;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee,
            max_priority,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee),
            U256::from(max_priority),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig::default();
        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);

        // Should fail - max_fee < base_fee
        match vm_result {
            Err(_) => {
                // Expected
            }
            Ok(mut vm) => {
                let exec_result = vm.execute();
                assert!(
                    exec_result.is_err() || !exec_result.unwrap().is_success(),
                    "Zero max_fee should fail validation"
                );
            }
        }
    }

    #[test]
    fn test_max_fee_below_base_fee_fails() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let max_fee = base_fee - 1; // Below base fee
        let max_priority = 0u64;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee,
            max_priority,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee),
            U256::from(max_priority),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig::default();
        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);

        match vm_result {
            Err(_) => {
                // Expected: max_fee < base_fee
            }
            Ok(mut vm) => {
                let exec_result = vm.execute();
                assert!(
                    exec_result.is_err() || !exec_result.unwrap().is_success(),
                    "max_fee < base_fee should fail"
                );
            }
        }
    }
}

// ============================================================================
// Section 3: Balance Edge Cases
// ============================================================================

mod balance_edge_cases {
    use super::*;

    #[test]
    fn test_exact_balance_for_gas_succeeds() {
        // Sender has exactly enough for gas, no value transfer
        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let max_priority = 100_000_000u64;
        let max_fee = base_fee + max_priority;
        let effective_price = base_fee + max_priority;
        let exact_balance = U256::from(gas_limit) * U256::from(effective_price);

        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, exact_balance, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(), // No value
            gas_limit,
            max_fee,
            max_priority,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee),
            U256::from(max_priority),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(
            report.is_success(),
            "Should succeed with exactly enough balance for gas"
        );

        // Sender should have zero balance left
        let final_balance = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(
            final_balance,
            U256::zero(),
            "Sender should have zero balance after using exact amount"
        );
    }

    #[test]
    fn test_one_wei_short_fails() {
        // Sender has 1 wei less than needed
        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let max_priority = 100_000_000u64;
        let max_fee = base_fee + max_priority;
        let effective_price = base_fee + max_priority;
        let needed = U256::from(gas_limit) * U256::from(effective_price);
        let balance = needed - U256::one(); // 1 wei short

        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, balance, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee,
            max_priority,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee),
            U256::from(max_priority),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig::default();
        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);

        match vm_result {
            Err(_) => {
                // Expected: insufficient funds
            }
            Ok(mut vm) => {
                let exec_result = vm.execute();
                assert!(
                    exec_result.is_err() || !exec_result.unwrap().is_success(),
                    "Should fail when 1 wei short"
                );
            }
        }
    }

    #[test]
    fn test_balance_covers_gas_but_not_value() {
        // Sender has enough for gas but not for value transfer
        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let max_priority = 100_000_000u64;
        let max_fee = base_fee + max_priority;
        let effective_price = base_fee + max_priority;
        let gas_cost = U256::from(gas_limit) * U256::from(effective_price);
        let balance = gas_cost + U256::from(1000u64); // Enough for gas + 1000 wei

        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, balance, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let value = U256::from(1_000_000u64); // More than 1000 wei extra

        let tx = create_eip1559_tx(TEST_RECIPIENT, value, gas_limit, max_fee, max_priority, 0);

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee),
            U256::from(max_priority),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig::default();
        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);

        match vm_result {
            Err(_) => {
                // Expected: insufficient funds for value
            }
            Ok(mut vm) => {
                let exec_result = vm.execute();
                assert!(
                    exec_result.is_err() || !exec_result.unwrap().is_success(),
                    "Should fail when balance covers gas but not value"
                );
            }
        }
    }
}

// ============================================================================
// Section 4: Gas Limit Edge Cases
// ============================================================================

mod gas_limit_edge_cases {
    use super::*;

    #[test]
    fn test_gas_limit_exactly_21000() {
        // Minimum gas for simple transfer
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            1_000_000_000,
            100_000_000,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(1_000_000_000u64),
            U256::from(100_000_000u64),
            U256::from(100_000_000u64),
            false,
        );

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());
        assert_eq!(report.gas_used, 21_000, "Should use exactly 21000 gas");
    }

    #[test]
    fn test_gas_limit_20999_fails() {
        // 1 below minimum
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let gas_limit = 20_999u64;
        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            1_000_000_000,
            100_000_000,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(1_000_000_000u64),
            U256::from(100_000_000u64),
            U256::from(100_000_000u64),
            false,
        );

        let fee_config = FeeConfig::default();
        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);

        match vm_result {
            Err(_) => {
                // Expected: intrinsic gas too low
            }
            Ok(mut vm) => {
                let exec_result = vm.execute();
                assert!(
                    exec_result.is_err() || !exec_result.unwrap().is_success(),
                    "Gas limit below 21000 should fail"
                );
            }
        }
    }

    #[test]
    fn test_very_high_gas_limit() {
        // Very high gas limit should work
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let gas_limit = 30_000_000u64; // 30M gas
        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            1_000_000_000,
            100_000_000,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(1_000_000_000u64),
            U256::from(100_000_000u64),
            U256::from(100_000_000u64),
            false,
        );

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());
        // Only 21000 should be used for simple transfer
        assert_eq!(report.gas_used, 21_000);
    }
}

// ============================================================================
// Section 5: Nonce Edge Cases
// ============================================================================

mod nonce_edge_cases {
    use super::*;

    #[test]
    fn test_nonce_zero_first_tx_succeeds() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0), // nonce 0
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            21_000,
            1_000_000_000,
            100_000_000,
            0, // nonce 0
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
        let report = vm.execute().unwrap();

        assert!(report.is_success(), "First tx with nonce 0 should succeed");
    }

    #[test]
    fn test_nonce_increments_after_tx() {
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

        let new_nonce = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.nonce)
            .unwrap_or(0);

        assert_eq!(new_nonce, 1, "Nonce should increment to 1 after first tx");
    }

    #[test]
    fn test_high_nonce_account_works() {
        let high_nonce = 1000u64;
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), high_nonce),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            21_000,
            1_000_000_000,
            100_000_000,
            high_nonce,
        );

        let mut env = create_eip1559_env(
            TEST_SENDER,
            21_000,
            U256::from(1_000_000_000u64),
            U256::from(100_000_000u64),
            U256::from(100_000_000u64),
            false,
        );
        env.tx_nonce = high_nonce;

        let fee_config = FeeConfig::default();
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success(), "High nonce should work");

        let final_nonce = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.nonce)
            .unwrap_or(0);

        assert_eq!(final_nonce, high_nonce + 1, "Nonce should increment");
    }
}

// ============================================================================
// Section 6: Value Transfer Edge Cases
// ============================================================================

mod value_transfer_edge_cases {
    use super::*;

    #[test]
    fn test_transfer_max_u64_value() {
        let large_balance = U256::from(u128::MAX);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, large_balance, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let value = U256::from(u64::MAX);

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
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success(), "Large value transfer should succeed");

        let recipient_balance = db
            .current_accounts_state
            .get(&TEST_RECIPIENT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(recipient_balance, value);
    }

    #[test]
    fn test_transfer_1_wei() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let value = U256::one(); // 1 wei

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
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success(), "1 wei transfer should succeed");

        let recipient_balance = db
            .current_accounts_state
            .get(&TEST_RECIPIENT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(recipient_balance, value, "Recipient should receive 1 wei");
    }
}

// ============================================================================
// Section 7: L1 Fee Gas Limit Validation Tests
// ============================================================================

mod l1_fee_gas_limit_validation {
    use super::*;

    #[test]
    fn test_insufficient_gas_for_l1_fee_rejected() {
        // A transaction with gas_limit = intrinsic_gas (21000) and L1 fee config
        // should be rejected at prepare_execution, not reverted at finalize.
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_L1_FEE_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64; // Exactly intrinsic gas — no room for L1 fee
        let max_fee_per_gas = 1_000_000_000u64;
        let base_fee = 100_000_000u64;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_fee_per_gas - base_fee,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_fee_per_gas - base_fee),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig {
            base_fee_vault: None,
            operator_fee_config: None,
            l1_fee_config: Some(L1FeeConfig {
                l1_fee_vault: TEST_L1_FEE_VAULT,
                l1_fee_per_blob_gas: 1000, // Non-trivial L1 fee
            }),
        };

        // Transaction must fail during prepare_execution (called by execute)
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let exec_result = vm.execute();
        assert!(
            exec_result.is_err(),
            "Transaction with gas_limit = intrinsic_gas should be rejected when L1 fee > 0"
        );
    }

    #[test]
    fn test_no_l1_fee_config_skips_validation() {
        // Without L1 fee config, gas_limit = intrinsic_gas should work fine
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let max_fee_per_gas = 1_000_000_000u64;
        let base_fee = 100_000_000u64;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_fee_per_gas - base_fee,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee_per_gas),
            U256::from(max_fee_per_gas - base_fee),
            U256::from(base_fee),
            false,
        );

        // No L1 fee config
        let fee_config = FeeConfig::default();

        let vm_result = create_test_l2_vm(&env, &mut db, &tx, fee_config);
        assert!(
            vm_result.is_ok(),
            "Without L1 fee config, gas_limit = intrinsic_gas should be accepted"
        );

        let mut vm = vm_result.unwrap();
        let report = vm.execute().unwrap();
        assert!(
            report.is_success(),
            "Transaction should succeed without L1 fee config"
        );
    }
}

// ============================================================================
// Discovered Bugs Section
// ============================================================================
// No bugs discovered during this test implementation.
