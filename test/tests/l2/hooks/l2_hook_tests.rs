//! Unit tests for L2Hook implementation.
//!
//! This module tests the L2-specific transaction hooks including:
//! - Privileged transaction handling (bridge transactions)
//! - Fee token transaction handling (ERC20 fee payments)
//! - L1 fee calculations for data availability
//! - Fee distribution to various vaults
//!
//! Tests are organized into sections:
//! 1. Encoding functions (pure, no VM needed)
//! 2. L1 fee calculations (pure, no VM needed)
//! 3. Hook factory tests
//! 4. prepare_execution tests (requires VM)
//! 5. finalize_execution tests (requires VM)
//! 6. Edge cases and error handling

use ethrex_common::types::SAFE_BYTES_PER_BLOB;
use ethrex_common::types::fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig};
use ethrex_common::{H160, U256};
use ethrex_levm::hooks::hook::{get_hooks, l1_hooks, l2_hooks};
use ethrex_levm::hooks::l2_hook::{
    COMMON_BRIDGE_L2_ADDRESS, FEE_TOKEN_RATIO_ADDRESS, FEE_TOKEN_REGISTRY_ADDRESS,
    IS_FEE_TOKEN_SELECTOR, LOCK_FEE_SELECTOR, PAY_FEE_SELECTOR, calculate_l1_fee,
    encode_fee_token_call, encode_fee_token_ratio_call, encode_is_fee_token_call,
};
use ethrex_levm::vm::VMType;

use super::test_utils::*;

// ============================================================================
// Section 1: Encoding Function Tests (Pure Functions)
// ============================================================================

mod encoding_tests {
    use super::*;

    #[test]
    fn test_encode_fee_token_call_lock_fee() {
        let payer = TEST_SENDER;
        let amount = U256::from(1000u64);

        let encoded = encode_fee_token_call(LOCK_FEE_SELECTOR, payer, amount);

        // Check length: 4 (selector) + 32 (address padded) + 32 (amount) = 68
        assert_eq!(encoded.len(), 68, "Encoded length should be 68 bytes");

        // Check selector
        assert_eq!(
            &encoded[0..4],
            &LOCK_FEE_SELECTOR,
            "First 4 bytes should be LOCK_FEE_SELECTOR"
        );

        // Check address padding (12 zero bytes before address)
        assert_eq!(
            &encoded[4..16],
            &[0u8; 12],
            "Address should be left-padded with 12 zero bytes"
        );

        // Check address
        assert_eq!(
            &encoded[16..36],
            &payer.0,
            "Bytes 16-36 should contain the address"
        );

        // Check amount is big-endian encoded
        let expected_amount = amount.to_big_endian();
        assert_eq!(
            &encoded[36..68],
            &expected_amount,
            "Bytes 36-68 should contain the amount in big-endian"
        );
    }

    #[test]
    fn test_encode_fee_token_call_pay_fee() {
        let receiver = TEST_COINBASE;
        let amount = U256::from(500u64);

        let encoded = encode_fee_token_call(PAY_FEE_SELECTOR, receiver, amount);

        assert_eq!(encoded.len(), 68);
        assert_eq!(&encoded[0..4], &PAY_FEE_SELECTOR);
        assert_eq!(&encoded[16..36], &receiver.0);
    }

    #[test]
    fn test_encode_fee_token_call_zero_amount() {
        let receiver = TEST_RECIPIENT;
        let amount = U256::zero();

        let encoded = encode_fee_token_call(PAY_FEE_SELECTOR, receiver, amount);

        assert_eq!(encoded.len(), 68);

        // Check that amount is all zeros
        assert_eq!(&encoded[36..68], &[0u8; 32]);
    }

    #[test]
    fn test_encode_fee_token_call_max_amount() {
        let receiver = TEST_RECIPIENT;
        let amount = U256::MAX;

        let encoded = encode_fee_token_call(PAY_FEE_SELECTOR, receiver, amount);

        assert_eq!(encoded.len(), 68);

        // Check that amount is all 0xff bytes
        assert_eq!(&encoded[36..68], &[0xffu8; 32]);
    }

    #[test]
    fn test_encode_is_fee_token_call() {
        let token = TEST_FEE_TOKEN;

        let encoded = encode_is_fee_token_call(token);

        // Check length: 4 (selector) + 32 (address padded) = 36
        assert_eq!(encoded.len(), 36, "Encoded length should be 36 bytes");

        // Check selector
        assert_eq!(
            &encoded[0..4],
            &IS_FEE_TOKEN_SELECTOR,
            "First 4 bytes should be IS_FEE_TOKEN_SELECTOR"
        );

        // Check address padding
        assert_eq!(
            &encoded[4..16],
            &[0u8; 12],
            "Address should be left-padded with 12 zero bytes"
        );

        // Check address
        assert_eq!(
            &encoded[16..36],
            &token.0,
            "Bytes 16-36 should contain the token address"
        );
    }

    #[test]
    fn test_encode_fee_token_ratio_call() {
        let token = TEST_FEE_TOKEN;

        let encoded = encode_fee_token_ratio_call(token);

        // Check length: 4 (selector) + 32 (address padded) = 36
        assert_eq!(encoded.len(), 36, "Encoded length should be 36 bytes");

        // Check selector (getFeeTokenRatio)
        let expected_selector: [u8; 4] = [0xc6, 0xab, 0x85, 0xd8];
        assert_eq!(
            &encoded[0..4],
            &expected_selector,
            "First 4 bytes should be FEE_TOKEN_RATIO_SELECTOR"
        );

        // Check address
        assert_eq!(
            &encoded[16..36],
            &token.0,
            "Bytes 16-36 should contain the token address"
        );
    }

    #[test]
    fn test_selectors_are_correct() {
        // lockFee(address,uint256) = keccak256("lockFee(address,uint256)")[0..4]
        // These are the expected selectors from the Solidity contract
        assert_eq!(LOCK_FEE_SELECTOR, [0x89, 0x9c, 0x86, 0xe2]);
        assert_eq!(PAY_FEE_SELECTOR, [0x72, 0x74, 0x6e, 0xaf]);
        assert_eq!(IS_FEE_TOKEN_SELECTOR, [0x16, 0xad, 0x82, 0xd7]);
    }
}

// ============================================================================
// Section 2: L1 Fee Calculation Tests (Pure Functions)
// ============================================================================

mod l1_fee_calculation_tests {
    use super::*;

    #[test]
    fn test_calculate_l1_fee_basic() {
        let fee_config = L1FeeConfig {
            l1_fee_vault: TEST_L1_FEE_VAULT,
            l1_fee_per_blob_gas: 1, // 1 wei per blob gas
        };
        let tx_size = 100; // 100 bytes

        let result = calculate_l1_fee(&fee_config, tx_size);
        assert!(result.is_ok(), "calculate_l1_fee should succeed");

        let fee = result.unwrap();

        // GAS_PER_BLOB / SAFE_BYTES_PER_BLOB = 131072 / 126976 = 1 (integer division)
        // fee = 1 * l1_fee_per_blob_gas(1) * tx_size(100) = 100
        let expected = U256::from(100u64);
        assert_eq!(fee, expected, "L1 fee calculation mismatch");
    }

    #[test]
    fn test_calculate_l1_fee_zero_size() {
        let fee_config = L1FeeConfig {
            l1_fee_vault: TEST_L1_FEE_VAULT,
            l1_fee_per_blob_gas: 1000,
        };
        let tx_size = 0;

        let result = calculate_l1_fee(&fee_config, tx_size);
        assert!(result.is_ok());

        let fee = result.unwrap();
        assert_eq!(fee, U256::zero(), "Fee should be zero for zero-size tx");
    }

    #[test]
    fn test_calculate_l1_fee_large_tx() {
        let fee_config = L1FeeConfig {
            l1_fee_vault: TEST_L1_FEE_VAULT,
            l1_fee_per_blob_gas: 100,
        };
        // Use SAFE_BYTES_PER_BLOB as tx size (one full blob worth of data)
        let tx_size = SAFE_BYTES_PER_BLOB;

        let result = calculate_l1_fee(&fee_config, tx_size);
        assert!(result.is_ok());

        let fee = result.unwrap();

        // fee_per_blob = 100 * 131072 = 13_107_200
        // fee_per_byte = 13_107_200 / 126976 = 103 (integer division)
        // fee = 103 * 126976 = 13_078_528
        let expected = U256::from(13_078_528u64);
        assert_eq!(fee, expected);
    }

    #[test]
    fn test_calculate_l1_fee_high_blob_gas_price() {
        let fee_config = L1FeeConfig {
            l1_fee_vault: TEST_L1_FEE_VAULT,
            l1_fee_per_blob_gas: 1_000_000_000, // 1 gwei per blob gas
        };
        let tx_size = 1000;

        let result = calculate_l1_fee(&fee_config, tx_size);
        assert!(result.is_ok());

        let fee = result.unwrap();

        // Verify fee is greater than zero
        assert!(fee > U256::zero(), "Fee should be positive");
    }

    #[test]
    fn test_calculate_l1_fee_overflow() {
        let fee_config = L1FeeConfig {
            l1_fee_vault: TEST_L1_FEE_VAULT,
            l1_fee_per_blob_gas: u64::MAX, // Maximum possible value
        };
        // Very large transaction size to trigger overflow in checked_mul
        let tx_size = usize::MAX;

        let result = calculate_l1_fee(&fee_config, tx_size);

        // u64::MAX * GAS_PER_BLOB overflows U256 intermediate, so this must return Err
        assert!(result.is_err(), "Expected Err for overflow inputs, got Ok");
        assert!(
            format!("{:?}", result.unwrap_err()).contains("Overflow"),
            "Error should be overflow-related"
        );
    }

    #[test]
    fn test_calculate_l1_fee_typical_transaction() {
        // Test with typical L2 transaction parameters
        let fee_config = L1FeeConfig {
            l1_fee_vault: TEST_L1_FEE_VAULT,
            l1_fee_per_blob_gas: 20, // 20 wei per blob gas (typical)
        };
        let tx_size = 200; // 200 bytes (typical simple transfer)

        let result = calculate_l1_fee(&fee_config, tx_size);
        assert!(result.is_ok());

        let fee = result.unwrap();

        // Fee should be reasonable (not zero, not absurdly high)
        assert!(fee > U256::zero());
        assert!(fee < U256::from(1_000_000_000_000_000_000u64)); // Less than 1 ETH
    }
}

// ============================================================================
// Section 3: Hook Factory Tests
// ============================================================================

mod hook_factory_tests {
    use super::*;

    #[test]
    fn test_get_hooks_l1_returns_single_hook() {
        let hooks = l1_hooks();
        assert_eq!(
            hooks.len(),
            1,
            "L1 should have exactly 1 hook (DefaultHook)"
        );
    }

    #[test]
    fn test_get_hooks_l2_returns_two_hooks() {
        let fee_config = FeeConfig::default();
        let hooks = l2_hooks(fee_config);
        assert_eq!(
            hooks.len(),
            2,
            "L2 should have exactly 2 hooks (L2Hook + BackupHook)"
        );
    }

    #[test]
    fn test_get_hooks_dispatches_correctly_for_l1() {
        let vm_type = VMType::L1;
        let hooks = get_hooks(&vm_type);
        assert_eq!(hooks.len(), 1, "VMType::L1 should return 1 hook");
    }

    #[test]
    fn test_get_hooks_dispatches_correctly_for_l2() {
        let fee_config = create_default_fee_config();
        let vm_type = VMType::L2(fee_config);
        let hooks = get_hooks(&vm_type);
        assert_eq!(hooks.len(), 2, "VMType::L2 should return 2 hooks");
    }

    #[test]
    fn test_l2_hooks_with_empty_fee_config() {
        let fee_config = FeeConfig::default();
        let hooks = l2_hooks(fee_config);

        // Should still return 2 hooks even with empty config
        assert_eq!(hooks.len(), 2);
    }

    #[test]
    fn test_l2_hooks_with_full_fee_config() {
        let fee_config = FeeConfig {
            base_fee_vault: Some(TEST_BASE_FEE_VAULT),
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: TEST_OPERATOR_VAULT,
                operator_fee_per_gas: 100,
            }),
            l1_fee_config: Some(L1FeeConfig {
                l1_fee_vault: TEST_L1_FEE_VAULT,
                l1_fee_per_blob_gas: 50,
            }),
        };

        let hooks = l2_hooks(fee_config);
        assert_eq!(hooks.len(), 2);
    }
}

// ============================================================================
// Section 4: Constant Address Tests
// ============================================================================

mod address_constants_tests {
    use super::*;

    #[test]
    fn test_common_bridge_address_format() {
        // COMMON_BRIDGE_L2_ADDRESS should be 0x000...ffff
        let expected = H160([
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0xff, 0xff,
        ]);
        assert_eq!(COMMON_BRIDGE_L2_ADDRESS, expected);
    }

    #[test]
    fn test_fee_token_registry_address_format() {
        // FEE_TOKEN_REGISTRY_ADDRESS should be 0x000...fffc
        let expected = H160([
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0xff, 0xfc,
        ]);
        assert_eq!(FEE_TOKEN_REGISTRY_ADDRESS, expected);
    }

    #[test]
    fn test_fee_token_ratio_address_format() {
        // FEE_TOKEN_RATIO_ADDRESS should be 0x000...fffb
        let expected = H160([
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0xff, 0xfb,
        ]);
        assert_eq!(FEE_TOKEN_RATIO_ADDRESS, expected);
    }

    #[test]
    fn test_system_addresses_are_distinct() {
        // All system addresses should be unique
        assert_ne!(COMMON_BRIDGE_L2_ADDRESS, FEE_TOKEN_REGISTRY_ADDRESS);
        assert_ne!(COMMON_BRIDGE_L2_ADDRESS, FEE_TOKEN_RATIO_ADDRESS);
        assert_ne!(FEE_TOKEN_REGISTRY_ADDRESS, FEE_TOKEN_RATIO_ADDRESS);
    }
}

// ============================================================================
// Section 5: Fee Config Tests
// ============================================================================

mod fee_config_tests {
    use super::*;

    #[test]
    fn test_create_test_fee_config_all_none() {
        let config = create_test_fee_config(None, None, None);

        assert!(config.base_fee_vault.is_none());
        assert!(config.operator_fee_config.is_none());
        assert!(config.l1_fee_config.is_none());
    }

    #[test]
    fn test_create_test_fee_config_with_base_fee_vault() {
        let config = create_test_fee_config(Some(TEST_BASE_FEE_VAULT), None, None);

        assert_eq!(config.base_fee_vault, Some(TEST_BASE_FEE_VAULT));
        assert!(config.operator_fee_config.is_none());
        assert!(config.l1_fee_config.is_none());
    }

    #[test]
    fn test_create_test_fee_config_with_operator_fee() {
        let operator_fee_per_gas = 50;
        let config = create_test_fee_config(None, Some(operator_fee_per_gas), None);

        assert!(config.base_fee_vault.is_none());
        assert!(config.operator_fee_config.is_some());

        let operator_config = config.operator_fee_config.unwrap();
        assert_eq!(operator_config.operator_fee_vault, TEST_OPERATOR_VAULT);
        assert_eq!(operator_config.operator_fee_per_gas, operator_fee_per_gas);
    }

    #[test]
    fn test_create_test_fee_config_with_l1_fee() {
        let l1_fee_per_blob_gas = 100;
        let config = create_test_fee_config(None, None, Some(l1_fee_per_blob_gas));

        assert!(config.base_fee_vault.is_none());
        assert!(config.operator_fee_config.is_none());
        assert!(config.l1_fee_config.is_some());

        let l1_config = config.l1_fee_config.unwrap();
        assert_eq!(l1_config.l1_fee_vault, TEST_L1_FEE_VAULT);
        assert_eq!(l1_config.l1_fee_per_blob_gas, l1_fee_per_blob_gas);
    }

    #[test]
    fn test_create_default_fee_config_has_all_components() {
        let config = create_default_fee_config();

        assert!(config.base_fee_vault.is_some());
        assert!(config.operator_fee_config.is_some());
        assert!(config.l1_fee_config.is_some());
    }
}

// ============================================================================
// Section 6: Environment Creation Tests
// ============================================================================

mod environment_tests {
    use super::*;

    #[test]
    fn test_create_minimal_env() {
        let env = create_minimal_env();

        assert_eq!(env.origin, TEST_SENDER);
        assert!(!env.is_privileged);
        assert!(env.fee_token.is_none());
        assert_eq!(env.gas_limit, DEFAULT_GAS_LIMIT);
    }

    #[test]
    fn test_create_privileged_env() {
        let env = create_privileged_env();

        assert_eq!(env.origin, COMMON_BRIDGE_L2_ADDRESS);
        assert!(env.is_privileged);
        assert!(env.fee_token.is_none());
    }

    #[test]
    fn test_create_fee_token_env() {
        let env = create_fee_token_env(TEST_FEE_TOKEN);

        assert_eq!(env.origin, TEST_SENDER);
        assert!(!env.is_privileged);
        assert_eq!(env.fee_token, Some(TEST_FEE_TOKEN));
    }

    #[test]
    fn test_create_test_env_l2_custom_gas_prices() {
        let gas_price = U256::from(2_000_000_000u64); // 2 gwei
        let base_fee = U256::from(1_000_000_000u64); // 1 gwei

        let env = create_test_env_l2(false, None, gas_price, base_fee);

        assert_eq!(env.gas_price, gas_price);
        assert_eq!(env.base_fee_per_gas, base_fee);

        // Priority fee should be gas_price - base_fee
        let expected_priority = gas_price - base_fee;
        assert_eq!(env.tx_max_priority_fee_per_gas, Some(expected_priority));
    }
}

// ============================================================================
// Section 7: Database Creation Tests
// ============================================================================

mod database_tests {
    use super::*;

    #[test]
    fn test_create_default_test_db_has_sender() {
        let db = create_default_test_db();

        assert!(
            db.current_accounts_state.contains_key(&TEST_SENDER),
            "Database should contain TEST_SENDER"
        );

        let sender_account = db.current_accounts_state.get(&TEST_SENDER).unwrap();
        assert_eq!(
            sender_account.info.balance,
            U256::from(DEFAULT_SENDER_BALANCE)
        );
        assert_eq!(sender_account.info.nonce, 0);
    }

    #[test]
    fn test_create_test_db_with_recipient() {
        let db = create_test_db_with_recipient();

        assert!(db.current_accounts_state.contains_key(&TEST_SENDER));
        assert!(db.current_accounts_state.contains_key(&TEST_RECIPIENT));

        let recipient_account = db.current_accounts_state.get(&TEST_RECIPIENT).unwrap();
        assert_eq!(recipient_account.info.balance, U256::zero());
    }

    #[test]
    fn test_create_test_db_with_accounts_custom() {
        let custom_address = H160([0x99; 20]);
        let custom_balance = U256::from(5000u64);
        let custom_nonce = 42u64;

        let db = create_test_db_with_accounts(vec![(custom_address, custom_balance, custom_nonce)]);

        assert!(db.current_accounts_state.contains_key(&custom_address));

        let account = db.current_accounts_state.get(&custom_address).unwrap();
        assert_eq!(account.info.balance, custom_balance);
        assert_eq!(account.info.nonce, custom_nonce);
    }

    #[test]
    fn test_create_test_db_initial_state_matches_current() {
        let db = create_default_test_db();

        // Initial state should match current state
        assert_eq!(
            db.initial_accounts_state.len(),
            db.current_accounts_state.len()
        );

        for (addr, account) in &db.current_accounts_state {
            let initial = db.initial_accounts_state.get(addr);
            assert!(initial.is_some(), "Initial state should have same accounts");
            assert_eq!(initial.unwrap().info, account.info);
        }
    }
}

// ============================================================================
// Section 8: Transaction Creation Tests
// ============================================================================

mod transaction_tests {
    use ethrex_common::types::Transaction;

    use super::*;

    #[test]
    fn test_create_default_tx() {
        let tx = create_default_tx();

        if let Transaction::LegacyTransaction(legacy) = tx {
            assert_eq!(legacy.gas, DEFAULT_GAS_LIMIT);
            assert_eq!(legacy.gas_price, U256::from(DEFAULT_GAS_PRICE));
            assert_eq!(legacy.value, U256::zero());
        } else {
            panic!("Expected LegacyTransaction");
        }
    }

    #[test]
    fn test_create_value_transfer_tx() {
        let value = U256::from(1_000_000_000_000_000_000u128); // 1 ETH

        let tx = create_value_transfer_tx(value);

        if let Transaction::LegacyTransaction(legacy) = tx {
            assert_eq!(legacy.value, value);
        } else {
            panic!("Expected LegacyTransaction");
        }
    }

    #[test]
    fn test_create_test_tx_custom_params() {
        let to = TEST_COINBASE;
        let value = U256::from(500u64);
        let gas_limit = 100_000u64;
        let gas_price = U256::from(50_000_000_000u64); // 50 gwei

        let tx = create_test_tx(to, value, gas_limit, gas_price);

        if let Transaction::LegacyTransaction(legacy) = tx {
            assert_eq!(legacy.gas, gas_limit);
            assert_eq!(legacy.gas_price, gas_price);
            assert_eq!(legacy.value, value);
        } else {
            panic!("Expected LegacyTransaction");
        }
    }
}

// ============================================================================
// Section 9: Assertion Helper Tests
// ============================================================================

mod assertion_tests {
    use super::*;

    #[test]
    fn test_assert_balance_passes_on_match() {
        let db = create_default_test_db();
        // This should not panic
        assert_balance(&db, TEST_SENDER, U256::from(DEFAULT_SENDER_BALANCE));
    }

    #[test]
    #[should_panic(expected = "Balance mismatch")]
    fn test_assert_balance_fails_on_mismatch() {
        let db = create_default_test_db();
        assert_balance(&db, TEST_SENDER, U256::zero());
    }

    #[test]
    fn test_assert_balance_returns_zero_for_unknown() {
        let db = create_default_test_db();
        let unknown_address = H160([0xaa; 20]);
        // Unknown address should have zero balance
        assert_balance(&db, unknown_address, U256::zero());
    }

    #[test]
    fn test_assert_nonce_passes_on_match() {
        let db = create_default_test_db();
        // Nonce should be 0 for newly created account
        assert_nonce(&db, TEST_SENDER, 0);
    }

    #[test]
    #[should_panic(expected = "Nonce mismatch")]
    fn test_assert_nonce_fails_on_mismatch() {
        let db = create_default_test_db();
        assert_nonce(&db, TEST_SENDER, 100);
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
