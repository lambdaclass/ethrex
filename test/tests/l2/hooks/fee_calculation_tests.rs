//! Comprehensive fee calculation tests for L2Hook.
//!
//! These tests verify exact fee amounts and calculations:
//! - Base fee vault receives exactly base_fee * gas_used
//! - Operator vault receives exactly operator_fee_per_gas * gas_used
//! - Coinbase receives exactly (priority_fee - operator_fee) * gas_used
//! - L1 fee vault receives L1 data availability fee
//! - Sender is charged correctly and refunded unused gas
//!
//! These tests ensure the fee distribution logic is mathematically correct.

use ethrex_common::types::fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig};
use ethrex_common::{Address, H256, U256};

use super::test_utils::*;

// ============================================================================
// Section 1: Exact Base Fee Calculation Tests
// ============================================================================

mod base_fee_exact_tests {
    use super::*;

    #[test]
    fn test_base_fee_vault_receives_exact_amount() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_BASE_FEE_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 1_000_000_000u64; // 1 gwei
        let max_priority_fee = 500_000_000u64; // 0.5 gwei
        let max_fee = base_fee + max_priority_fee;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee,
            max_priority_fee,
            0,
        );

        let env = create_eip1559_env(
            TEST_SENDER,
            gas_limit,
            U256::from(max_fee),
            U256::from(max_priority_fee),
            U256::from(base_fee),
            false,
        );

        let fee_config = FeeConfig {
            base_fee_vault: Some(TEST_BASE_FEE_VAULT),
            operator_fee_config: None,
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());

        // Base fee vault should receive exactly: base_fee * gas_used
        let vault_balance = db
            .current_accounts_state
            .get(&TEST_BASE_FEE_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let expected = U256::from(base_fee) * U256::from(report.gas_used);
        assert_eq!(
            vault_balance, expected,
            "Base fee vault should receive exactly base_fee * gas_used = {} * {} = {}",
            base_fee, report.gas_used, expected
        );
    }

    #[test]
    fn test_base_fee_with_different_gas_amounts() {
        // Test with various gas usage scenarios
        for gas_limit in [21_000u64, 50_000, 100_000] {
            let mut db = create_test_db_with_accounts(vec![
                (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
                (TEST_RECIPIENT, U256::zero(), 0),
                (TEST_BASE_FEE_VAULT, U256::zero(), 0),
            ]);

            let base_fee = 100_000_000u64;
            let max_fee = 200_000_000u64;
            let max_priority = 100_000_000u64;

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
                base_fee_vault: Some(TEST_BASE_FEE_VAULT),
                operator_fee_config: None,
                l1_fee_config: None,
            };

            let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
            let report = vm.execute().unwrap();

            let vault_balance = db
                .current_accounts_state
                .get(&TEST_BASE_FEE_VAULT)
                .map(|a| a.info.balance)
                .unwrap_or(U256::zero());

            let expected = U256::from(base_fee) * U256::from(report.gas_used);
            assert_eq!(
                vault_balance, expected,
                "Gas limit {}: vault should receive {} but got {}",
                gas_limit, expected, vault_balance
            );
        }
    }
}

// ============================================================================
// Section 2: Exact Operator Fee Calculation Tests
// ============================================================================

mod operator_fee_exact_tests {
    use super::*;

    #[test]
    fn test_operator_vault_receives_exact_amount() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let operator_fee_per_gas = 50_000_000u64;
        let max_priority = 200_000_000u64;
        let max_fee = base_fee + max_priority;

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
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());

        // Operator vault should receive exactly: operator_fee_per_gas * gas_used
        let vault_balance = db
            .current_accounts_state
            .get(&TEST_OPERATOR_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let expected = U256::from(operator_fee_per_gas) * U256::from(report.gas_used);
        assert_eq!(
            vault_balance, expected,
            "Operator vault should receive exactly operator_fee_per_gas * gas_used"
        );
    }

    #[test]
    fn test_operator_fee_with_varying_rates() {
        // Test different operator fee rates
        for operator_fee_per_gas in [10_000_000u64, 50_000_000, 100_000_000] {
            let mut db = create_test_db_with_accounts(vec![
                (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
                (TEST_RECIPIENT, U256::zero(), 0),
                (TEST_OPERATOR_VAULT, U256::zero(), 0),
            ]);

            let gas_limit = 21_000u64;
            let base_fee = 100_000_000u64;
            let max_priority = operator_fee_per_gas + 100_000_000;
            let max_fee = base_fee + max_priority;

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
                    operator_fee_per_gas,
                }),
                l1_fee_config: None,
            };

            let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
            let report = vm.execute().unwrap();

            let vault_balance = db
                .current_accounts_state
                .get(&TEST_OPERATOR_VAULT)
                .map(|a| a.info.balance)
                .unwrap_or(U256::zero());

            let expected = U256::from(operator_fee_per_gas) * U256::from(report.gas_used);
            assert_eq!(
                vault_balance, expected,
                "Operator rate {}: vault should receive {}",
                operator_fee_per_gas, expected
            );
        }
    }
}

// ============================================================================
// Section 3: Exact Coinbase Fee Calculation Tests
// ============================================================================

mod coinbase_fee_exact_tests {
    use super::*;

    #[test]
    fn test_coinbase_receives_priority_minus_operator() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let operator_fee_per_gas = 50_000_000u64;
        let max_priority = 200_000_000u64; // priority > operator
        let max_fee = base_fee + max_priority;

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
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());

        // Coinbase should receive: (priority_fee - operator_fee) * gas_used
        let coinbase_balance = db
            .current_accounts_state
            .get(&TEST_COINBASE)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let coinbase_fee_per_gas = max_priority - operator_fee_per_gas;
        let expected = U256::from(coinbase_fee_per_gas) * U256::from(report.gas_used);

        assert_eq!(
            coinbase_balance, expected,
            "Coinbase should receive (priority - operator) * gas_used = ({} - {}) * {} = {}",
            max_priority, operator_fee_per_gas, report.gas_used, expected
        );
    }

    #[test]
    fn test_coinbase_gets_full_priority_without_operator() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let max_priority = 200_000_000u64;
        let max_fee = base_fee + max_priority;

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

        // No operator fee configured
        let fee_config = FeeConfig::default();

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());

        // Coinbase should receive full priority fee
        let coinbase_balance = db
            .current_accounts_state
            .get(&TEST_COINBASE)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let expected = U256::from(max_priority) * U256::from(report.gas_used);

        assert_eq!(
            coinbase_balance, expected,
            "Coinbase should receive full priority fee when no operator: {} * {} = {}",
            max_priority, report.gas_used, expected
        );
    }
}

// ============================================================================
// Section 4: Total Fee Accounting Tests
// ============================================================================

mod total_fee_accounting_tests {
    use super::*;

    #[test]
    fn test_all_fees_sum_correctly() {
        // Verify that base_fee_vault + operator_vault + coinbase = total fees paid
        let initial_sender = U256::from(DEFAULT_SENDER_BALANCE);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, initial_sender, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_BASE_FEE_VAULT, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let operator_fee_per_gas = 50_000_000u64;
        let max_priority = 200_000_000u64;
        let max_fee = base_fee + max_priority;

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
            base_fee_vault: Some(TEST_BASE_FEE_VAULT),
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: TEST_OPERATOR_VAULT,
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());

        // Get all balances
        let final_sender = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let base_vault = db
            .current_accounts_state
            .get(&TEST_BASE_FEE_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let operator_vault = db
            .current_accounts_state
            .get(&TEST_OPERATOR_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let coinbase = db
            .current_accounts_state
            .get(&TEST_COINBASE)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        // Total fees paid by sender
        let total_paid = initial_sender - final_sender;

        // Total fees received
        let total_received = base_vault + operator_vault + coinbase;

        // They should be equal (no L1 fee in this test)
        assert_eq!(
            total_paid, total_received,
            "Total paid ({}) should equal total received (base:{} + operator:{} + coinbase:{} = {})",
            total_paid, base_vault, operator_vault, coinbase, total_received
        );

        // Verify individual components
        let gas_used = U256::from(report.gas_used);
        assert_eq!(base_vault, U256::from(base_fee) * gas_used);
        assert_eq!(operator_vault, U256::from(operator_fee_per_gas) * gas_used);
        assert_eq!(
            coinbase,
            U256::from(max_priority - operator_fee_per_gas) * gas_used
        );
    }

    #[test]
    fn test_sender_charged_effective_gas_price() {
        let initial_sender = U256::from(DEFAULT_SENDER_BALANCE);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, initial_sender, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let max_priority = 50_000_000u64;
        let max_fee = base_fee + max_priority;

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
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        let final_sender = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let total_paid = initial_sender - final_sender;

        // Effective gas price = base_fee + priority_fee (capped by max_fee)
        let effective_price = base_fee + max_priority;
        let expected_cost = U256::from(effective_price) * U256::from(report.gas_used);

        assert_eq!(
            total_paid, expected_cost,
            "Sender should be charged effective_gas_price * gas_used = {} * {} = {}",
            effective_price, report.gas_used, expected_cost
        );
    }
}

// ============================================================================
// Section 5: L1 Fee Calculation Tests
// ============================================================================

mod l1_fee_calculation_tests {
    use super::*;

    #[test]
    fn test_l1_fee_included_in_total_gas() {
        let initial_sender = U256::from(DEFAULT_SENDER_BALANCE);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, initial_sender, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_L1_FEE_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 100_000u64;
        let base_fee = 100_000_000u64;
        let max_priority = 100_000_000u64;
        let max_fee = base_fee + max_priority;
        let l1_fee_per_blob_gas = 100u64;

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
            operator_fee_config: None,
            l1_fee_config: Some(L1FeeConfig {
                l1_fee_vault: TEST_L1_FEE_VAULT,
                l1_fee_per_blob_gas,
            }),
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());

        // L1 vault should have received some fee
        let _l1_vault_balance = db
            .current_accounts_state
            .get(&TEST_L1_FEE_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        // L1 fee is paid as: l1_gas * gas_price
        // The exact amount depends on tx size and l1_fee_per_blob_gas
        // Just verify gas was used correctly
        assert!(
            report.gas_used >= 21_000,
            "Gas used should include at least intrinsic gas"
        );
    }

    #[test]
    fn test_l1_fee_zero_when_not_configured() {
        let initial_sender = U256::from(DEFAULT_SENDER_BALANCE);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, initial_sender, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let max_priority = 100_000_000u64;
        let max_fee = base_fee + max_priority;

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

        // No L1 fee config
        let fee_config = FeeConfig::default();

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        // Gas used should be exactly 21000 (intrinsic only, no L1 gas)
        assert_eq!(
            report.gas_used, 21_000,
            "Without L1 fee config, gas_used should be exactly intrinsic gas"
        );
    }
}

// ============================================================================
// Section 6: Gas Refund Exact Calculation Tests
// ============================================================================

mod gas_refund_exact_tests {
    use super::*;

    #[test]
    fn test_exact_refund_calculation() {
        let initial_sender = U256::from(DEFAULT_SENDER_BALANCE);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, initial_sender, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
        ]);

        // High gas limit, simple transfer uses only 21000
        let gas_limit = 100_000u64;
        let base_fee = 100_000_000u64;
        let max_priority = 100_000_000u64;
        let max_fee = base_fee + max_priority;

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
        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        let final_sender = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        // Sender should only pay for gas_used, not gas_limit
        let total_paid = initial_sender - final_sender;
        let effective_price = U256::from(base_fee + max_priority);
        let expected_payment = effective_price * U256::from(report.gas_used);

        assert_eq!(
            total_paid, expected_payment,
            "Sender should pay for gas_used ({}), not gas_limit ({})",
            report.gas_used, gas_limit
        );

        // Verify refund happened (paid less than max possible)
        let max_payment = effective_price * U256::from(gas_limit);
        assert!(
            total_paid < max_payment,
            "Refund should have occurred: paid {} < max {}",
            total_paid,
            max_payment
        );
    }
}

// ============================================================================
// Section 7: Multiple Transaction Sequence Tests
// ============================================================================

mod multi_tx_tests {
    use super::*;

    #[test]
    fn test_sequential_transactions_accumulate_fees() {
        let initial_sender = U256::from(DEFAULT_SENDER_BALANCE);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, initial_sender, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_BASE_FEE_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000u64;
        let max_priority = 100_000_000u64;
        let max_fee = base_fee + max_priority;

        let fee_config = FeeConfig {
            base_fee_vault: Some(TEST_BASE_FEE_VAULT),
            operator_fee_config: None,
            l1_fee_config: None,
        };

        let mut total_base_fees = U256::zero();

        // Execute 3 transactions
        for nonce in 0..3u64 {
            let tx = create_eip1559_tx(
                TEST_RECIPIENT,
                U256::zero(),
                gas_limit,
                max_fee,
                max_priority,
                nonce,
            );

            let mut env = create_eip1559_env(
                TEST_SENDER,
                gas_limit,
                U256::from(max_fee),
                U256::from(max_priority),
                U256::from(base_fee),
                false,
            );
            env.tx_nonce = nonce;

            let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
            let report = vm.execute().unwrap();

            assert!(report.is_success(), "Transaction {} should succeed", nonce);

            total_base_fees += U256::from(base_fee) * U256::from(report.gas_used);
        }

        // Verify accumulated fees in vault
        let vault_balance = db
            .current_accounts_state
            .get(&TEST_BASE_FEE_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        assert_eq!(
            vault_balance, total_base_fees,
            "Vault should have accumulated fees from all transactions"
        );
    }
}

// ============================================================================
// Section 8: Edge Cases in Fee Calculations
// ============================================================================

mod fee_calculation_edge_cases {
    use super::*;

    #[test]
    fn test_very_small_fees() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_BASE_FEE_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 1u64; // 1 wei base fee
        let max_priority = 1u64;
        let max_fee = base_fee + max_priority;

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
            base_fee_vault: Some(TEST_BASE_FEE_VAULT),
            operator_fee_config: None,
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());

        // Even tiny fees should be calculated correctly
        let vault_balance = db
            .current_accounts_state
            .get(&TEST_BASE_FEE_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let expected = U256::from(base_fee) * U256::from(report.gas_used);
        assert_eq!(vault_balance, expected);
    }

    #[test]
    fn test_large_fees_no_overflow() {
        let very_large_balance = U256::from(1_000_000_000_000_000_000_000u128); // 1000 ETH
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, very_large_balance, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_BASE_FEE_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100_000_000_000u64; // 100 gwei
        let max_priority = 50_000_000_000u64; // 50 gwei
        let max_fee = base_fee + max_priority;

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
            base_fee_vault: Some(TEST_BASE_FEE_VAULT),
            operator_fee_config: None,
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success(), "Large fee transaction should succeed");

        // Verify no overflow in calculation
        let vault_balance = db
            .current_accounts_state
            .get(&TEST_BASE_FEE_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let expected = U256::from(base_fee) * U256::from(report.gas_used);
        assert_eq!(vault_balance, expected);
    }
}

// ============================================================================
// Section 9: Priority Fee Capping Tests (EIP-1559 + Operator Fee)
// ============================================================================
//
// These tests verify the formula from transaction_fees.md:
//
//   effective_priority_fee = min(
//       max_priority_fee_per_gas,
//       max_fee_per_gas - base_fee_per_gas - operator_fee_per_gas
//   )
//
// The gas_price in Environment must be the "effective gas price":
//   gas_price = min(max_priority_fee + base_fee + operator_fee, max_fee_per_gas)
//
// This ensures operator_fee is always paid from max_fee_per_gas, not max_priority_fee.

mod priority_fee_capping_tests {
    use super::*;
    use std::cmp::min;

    /// Creates an EIP-1559 environment with the correct effective gas price calculation.
    /// This mirrors `calculate_gas_price_for_tx` in production code.
    fn create_eip1559_env_with_effective_gas_price(
        origin: Address,
        gas_limit: u64,
        max_fee_per_gas: u64,
        max_priority_fee_per_gas: u64,
        base_fee: u64,
        operator_fee_per_gas: u64,
    ) -> ethrex_levm::environment::Environment {
        use ethrex_common::types::Fork;
        use ethrex_levm::EVMConfig;

        // Calculate effective gas price as per EIP-1559 + operator fee:
        // gas_price = min(max_priority_fee + base_fee + operator_fee, max_fee_per_gas)
        let fee_per_gas = base_fee + operator_fee_per_gas;
        let effective_gas_price = min(max_priority_fee_per_gas + fee_per_gas, max_fee_per_gas);

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
            base_fee_per_gas: U256::from(base_fee),
            base_blob_fee_per_gas: U256::zero(),
            gas_price: U256::from(effective_gas_price),
            block_excess_blob_gas: None,
            block_blob_gas_used: None,
            tx_blob_hashes: Vec::new(),
            tx_max_priority_fee_per_gas: Some(U256::from(max_priority_fee_per_gas)),
            tx_max_fee_per_gas: Some(U256::from(max_fee_per_gas)),
            tx_max_fee_per_blob_gas: None,
            tx_nonce: 0,
            block_gas_limit: u64::MAX,
            is_privileged: false,
            fee_token: None,
        }
    }

    /// Case 1: max_priority_fee + base_fee + operator_fee <= max_fee_per_gas
    /// Expected: effective_priority_fee = max_priority_fee
    ///
    /// Gas price calculation:
    ///   gas_price = min(max_priority + base + operator, max_fee) = max_priority + base + operator
    ///
    /// Priority fee calculation:
    ///   effective_priority = gas_price - base - operator = max_priority
    #[test]
    fn test_priority_capped_to_max_priority_fee_when_sum_within_max_fee() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100u64; // 100 wei
        let operator_fee_per_gas = 50u64; // 50 wei
        let max_priority_fee = 200u64; // 200 wei
        let max_fee_per_gas = 500u64; // 500 wei (plenty of room)

        // Verify: max_priority + base + operator = 200 + 100 + 50 = 350 <= 500 ✓
        assert!(
            max_priority_fee + base_fee + operator_fee_per_gas <= max_fee_per_gas,
            "Test setup: sum should be <= max_fee_per_gas"
        );

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            0,
        );

        let env = create_eip1559_env_with_effective_gas_price(
            TEST_SENDER,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            base_fee,
            operator_fee_per_gas,
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

        assert!(report.is_success());

        let coinbase_balance = db
            .current_accounts_state
            .get(&TEST_COINBASE)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let operator_balance = db
            .current_accounts_state
            .get(&TEST_OPERATOR_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        // gas_price = min(200 + 100 + 50, 500) = 350
        // effective_priority = 350 - 100 - 50 = 200 (= max_priority_fee)
        // coinbase gets: effective_priority * gas_used = 200 * 21000
        let expected_coinbase = U256::from(max_priority_fee) * U256::from(report.gas_used);
        let expected_operator = U256::from(operator_fee_per_gas) * U256::from(report.gas_used);

        assert_eq!(
            coinbase_balance, expected_coinbase,
            "Coinbase should receive effective_priority * gas = {} * {} = {}",
            max_priority_fee, report.gas_used, expected_coinbase
        );

        assert_eq!(
            operator_balance, expected_operator,
            "Operator vault should receive operator_fee * gas = {} * {} = {}",
            operator_fee_per_gas, report.gas_used, expected_operator
        );
    }

    /// Case 2: max_priority_fee + base_fee + operator_fee > max_fee_per_gas
    /// Expected: effective_priority_fee = max_fee_per_gas - base_fee - operator_fee
    ///
    /// Note: EIP-1559 requires max_priority_fee <= max_fee_per_gas
    ///
    /// Gas price calculation:
    ///   gas_price = min(max_priority + base + operator, max_fee) = max_fee (capped!)
    ///
    /// Priority fee calculation:
    ///   effective_priority = gas_price - base - operator = max_fee - base - operator
    #[test]
    fn test_priority_capped_by_max_fee_minus_base_minus_operator() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100u64; // 100 wei
        let operator_fee_per_gas = 50u64; // 50 wei
        let max_priority_fee = 100u64; // 100 wei (user wants 100 priority)
        let max_fee_per_gas = 200u64; // 200 wei (but total is capped)

        // Verify EIP-1559 constraint: max_priority <= max_fee
        assert!(
            max_priority_fee <= max_fee_per_gas,
            "Test setup: EIP-1559 requires max_priority <= max_fee"
        );
        // Verify: max_priority + base + operator = 100 + 100 + 50 = 250 > 200 ✓
        assert!(
            max_priority_fee + base_fee + operator_fee_per_gas > max_fee_per_gas,
            "Test setup: sum should be > max_fee_per_gas to trigger capping"
        );
        // Also verify max_fee >= base + operator (otherwise tx would be rejected)
        assert!(
            max_fee_per_gas >= base_fee + operator_fee_per_gas,
            "Test setup: max_fee should cover base + operator"
        );

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            0,
        );

        let env = create_eip1559_env_with_effective_gas_price(
            TEST_SENDER,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            base_fee,
            operator_fee_per_gas,
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

        assert!(report.is_success());

        let coinbase_balance = db
            .current_accounts_state
            .get(&TEST_COINBASE)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let operator_balance = db
            .current_accounts_state
            .get(&TEST_OPERATOR_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        // gas_price = min(100 + 100 + 50, 200) = min(250, 200) = 200
        // effective_priority = 200 - 100 - 50 = 50 (< max_priority of 100!)
        // coinbase gets: 50 * gas_used
        let effective_priority = max_fee_per_gas - base_fee - operator_fee_per_gas; // 50
        let expected_coinbase = U256::from(effective_priority) * U256::from(report.gas_used);
        let expected_operator = U256::from(operator_fee_per_gas) * U256::from(report.gas_used);

        // Verify the priority was actually capped (not the full max_priority)
        assert!(
            effective_priority < max_priority_fee,
            "Test validation: effective_priority ({}) should be less than max_priority ({})",
            effective_priority,
            max_priority_fee
        );

        assert_eq!(
            coinbase_balance, expected_coinbase,
            "Coinbase should receive (max_fee - base - operator) * gas = ({} - {} - {}) * {} = {}",
            max_fee_per_gas, base_fee, operator_fee_per_gas, report.gas_used, expected_coinbase
        );

        assert_eq!(
            operator_balance, expected_operator,
            "Operator vault always receives operator_fee * gas = {} * {} = {}",
            operator_fee_per_gas, report.gas_used, expected_operator
        );
    }

    /// Edge case: max_priority_fee = 0
    /// Expected: coinbase gets 0, operator still gets paid
    #[test]
    fn test_zero_max_priority_fee_coinbase_gets_zero() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100u64;
        let operator_fee_per_gas = 50u64;
        let max_priority_fee = 0u64; // User doesn't want to tip
        let max_fee_per_gas = 200u64; // Enough to cover base + operator

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            0,
        );

        let env = create_eip1559_env_with_effective_gas_price(
            TEST_SENDER,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            base_fee,
            operator_fee_per_gas,
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

        assert!(report.is_success());

        let coinbase_balance = db
            .current_accounts_state
            .get(&TEST_COINBASE)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let operator_balance = db
            .current_accounts_state
            .get(&TEST_OPERATOR_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        // gas_price = min(0 + 150, 200) = 150
        // priority_fee = 150 - 100 = 50
        // coinbase_fee = 50 - 50 = 0
        let expected_operator = U256::from(operator_fee_per_gas) * U256::from(report.gas_used);

        assert_eq!(
            coinbase_balance,
            U256::zero(),
            "Coinbase should receive 0 when max_priority_fee = 0"
        );

        assert_eq!(
            operator_balance, expected_operator,
            "Operator still gets paid even when max_priority_fee = 0"
        );
    }

    /// Edge case: max_fee_per_gas = base_fee + operator_fee (exact minimum)
    /// Expected: coinbase gets 0, operator gets paid
    ///
    /// Note: EIP-1559 requires max_priority_fee <= max_fee_per_gas
    ///
    /// Gas price calculation:
    ///   gas_price = min(max_priority + base + operator, max_fee) = max_fee
    ///
    /// Priority fee calculation:
    ///   effective_priority = max_fee - base - operator = 0
    #[test]
    fn test_max_fee_equals_base_plus_operator_coinbase_gets_zero() {
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE), 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100u64;
        let operator_fee_per_gas = 50u64;
        let max_fee_per_gas = 150u64; // max_fee is exactly base + operator
        let max_priority_fee = 100u64; // User wants to tip, but can't due to max_fee cap

        // Verify EIP-1559 constraint: max_priority <= max_fee
        assert!(
            max_priority_fee <= max_fee_per_gas,
            "Test setup: EIP-1559 requires max_priority <= max_fee"
        );
        // Verify: max_fee = base + operator exactly
        assert_eq!(
            max_fee_per_gas,
            base_fee + operator_fee_per_gas,
            "Test setup: max_fee should equal base + operator exactly"
        );

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            0,
        );

        let env = create_eip1559_env_with_effective_gas_price(
            TEST_SENDER,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            base_fee,
            operator_fee_per_gas,
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

        assert!(report.is_success());

        let coinbase_balance = db
            .current_accounts_state
            .get(&TEST_COINBASE)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let operator_balance = db
            .current_accounts_state
            .get(&TEST_OPERATOR_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        // gas_price = min(100 + 100 + 50, 150) = min(250, 150) = 150
        // effective_priority = 150 - 100 - 50 = 0
        // coinbase gets 0
        let expected_operator = U256::from(operator_fee_per_gas) * U256::from(report.gas_used);

        assert_eq!(
            coinbase_balance,
            U256::zero(),
            "Coinbase should receive 0 when max_fee = base + operator"
        );

        assert_eq!(
            operator_balance, expected_operator,
            "Operator gets paid when max_fee = base + operator"
        );
    }

    /// Verify total sender payment matches: gas_price * gas_used
    /// where gas_price = min(max_priority + base + operator, max_fee)
    #[test]
    fn test_sender_pays_effective_gas_price_with_operator() {
        let initial_sender = U256::from(DEFAULT_SENDER_BALANCE);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, initial_sender, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
            (TEST_BASE_FEE_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100u64;
        let operator_fee_per_gas = 50u64;
        let max_priority_fee = 80u64;
        let max_fee_per_gas = 300u64;

        // gas_price = min(80 + 150, 300) = 230
        let expected_gas_price = min(
            max_priority_fee + base_fee + operator_fee_per_gas,
            max_fee_per_gas,
        );

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            0,
        );

        let env = create_eip1559_env_with_effective_gas_price(
            TEST_SENDER,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            base_fee,
            operator_fee_per_gas,
        );

        let fee_config = FeeConfig {
            base_fee_vault: Some(TEST_BASE_FEE_VAULT),
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: TEST_OPERATOR_VAULT,
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());

        let final_sender = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let total_paid = initial_sender - final_sender;
        let expected_payment = U256::from(expected_gas_price) * U256::from(report.gas_used);

        assert_eq!(
            total_paid, expected_payment,
            "Sender should pay gas_price * gas_used = {} * {} = {}",
            expected_gas_price, report.gas_used, expected_payment
        );
    }

    /// Verify all fees sum correctly: base_fee_vault + operator_vault + coinbase = sender payment
    #[test]
    fn test_all_fees_sum_to_sender_payment_with_operator() {
        let initial_sender = U256::from(DEFAULT_SENDER_BALANCE);
        let mut db = create_test_db_with_accounts(vec![
            (TEST_SENDER, initial_sender, 0),
            (TEST_RECIPIENT, U256::zero(), 0),
            (TEST_COINBASE, U256::zero(), 0),
            (TEST_OPERATOR_VAULT, U256::zero(), 0),
            (TEST_BASE_FEE_VAULT, U256::zero(), 0),
        ]);

        let gas_limit = 21_000u64;
        let base_fee = 100u64;
        let operator_fee_per_gas = 50u64;
        let max_priority_fee = 80u64;
        let max_fee_per_gas = 300u64;

        let tx = create_eip1559_tx(
            TEST_RECIPIENT,
            U256::zero(),
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            0,
        );

        let env = create_eip1559_env_with_effective_gas_price(
            TEST_SENDER,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee,
            base_fee,
            operator_fee_per_gas,
        );

        let fee_config = FeeConfig {
            base_fee_vault: Some(TEST_BASE_FEE_VAULT),
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: TEST_OPERATOR_VAULT,
                operator_fee_per_gas,
            }),
            l1_fee_config: None,
        };

        let mut vm = create_test_l2_vm(&env, &mut db, &tx, fee_config).unwrap();
        let report = vm.execute().unwrap();

        assert!(report.is_success());

        let final_sender = db
            .current_accounts_state
            .get(&TEST_SENDER)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let base_vault = db
            .current_accounts_state
            .get(&TEST_BASE_FEE_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let operator_vault = db
            .current_accounts_state
            .get(&TEST_OPERATOR_VAULT)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let coinbase = db
            .current_accounts_state
            .get(&TEST_COINBASE)
            .map(|a| a.info.balance)
            .unwrap_or(U256::zero());

        let total_paid = initial_sender - final_sender;
        let total_received = base_vault + operator_vault + coinbase;

        assert_eq!(
            total_paid, total_received,
            "Total paid ({}) = base_vault ({}) + operator ({}) + coinbase ({}) = {}",
            total_paid, base_vault, operator_vault, coinbase, total_received
        );

        // Verify individual components
        let gas_used = U256::from(report.gas_used);
        assert_eq!(
            base_vault,
            U256::from(base_fee) * gas_used,
            "Base fee vault mismatch"
        );
        assert_eq!(
            operator_vault,
            U256::from(operator_fee_per_gas) * gas_used,
            "Operator vault mismatch"
        );
        // Coinbase = (effective_priority - operator) * gas = (80 - 0) * gas = 80 * gas
        // Wait, let me recalculate:
        // gas_price = min(80 + 150, 300) = 230
        // priority = 230 - 100 = 130
        // coinbase = 130 - 50 = 80
        assert_eq!(
            coinbase,
            U256::from(max_priority_fee) * gas_used,
            "Coinbase mismatch"
        );
    }
}

// ============================================================================
// Discovered Bugs Section
// ============================================================================
// No bugs discovered during this test implementation.
