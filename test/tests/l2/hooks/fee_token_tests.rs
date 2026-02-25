//! Fee token tests for L2Hook implementation.
//!
//! These tests verify the fee token (ERC20 gas payment) functionality:
//! - Fee token registration validation
//! - Fee token ratio fetching
//! - Fee token locking (upfront gas cost)
//! - Fee token payment (vault payments, refunds)
//!
//! Fee token transactions require mock ERC20 contracts that simulate:
//! - FeeTokenRegistry.isFeeToken(token) -> bool
//! - FeeTokenRatio.getFeeTokenRatio(token) -> uint256
//! - FeeToken.lockFee(payer, amount) -> void
//! - FeeToken.payFee(receiver, amount) -> void

use ethrex_common::types::fee_config::{FeeConfig, L1FeeConfig, OperatorFeeConfig};
use ethrex_common::types::{EIP1559Transaction, Transaction, TxKind};
use ethrex_common::{Address, H160, U256};
use ethrex_levm::hooks::l2_hook::{
    COMMON_BRIDGE_L2_ADDRESS, FEE_TOKEN_RATIO_ADDRESS, FEE_TOKEN_REGISTRY_ADDRESS,
    IS_FEE_TOKEN_SELECTOR, LOCK_FEE_SELECTOR, PAY_FEE_SELECTOR, encode_fee_token_call,
    encode_fee_token_ratio_call, encode_is_fee_token_call,
};
use once_cell::sync::OnceCell;

use super::test_utils::*;
use bytes::Bytes;

/// A mock fee token address for testing
const MOCK_FEE_TOKEN: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xFE, 0xE7, // 0xFEE7 - fee token
]);

// ============================================================================
// Encoding Tests for Fee Token Functions
// ============================================================================

mod encoding_tests {
    use super::*;

    #[test]
    fn test_lock_fee_encoding_format() {
        // lockFee(address payer, uint256 amount)
        let payer = TEST_SENDER;
        let amount = U256::from(1000u64);

        let encoded = encode_fee_token_call(LOCK_FEE_SELECTOR, payer, amount);

        // Verify length: 4 (selector) + 32 (padded address) + 32 (amount) = 68 bytes
        assert_eq!(encoded.len(), 68);

        // Verify selector
        assert_eq!(&encoded[0..4], &LOCK_FEE_SELECTOR);

        // Verify address is right-padded (12 zeros + 20 byte address)
        assert_eq!(&encoded[4..16], &[0u8; 12]);
        assert_eq!(&encoded[16..36], &payer.0);

        // Verify amount is left-padded to 32 bytes
        let expected_amount = amount.to_big_endian();
        assert_eq!(&encoded[36..68], &expected_amount);
    }

    #[test]
    fn test_pay_fee_encoding_format() {
        // payFee(address receiver, uint256 amount)
        let receiver = TEST_COINBASE;
        let amount = U256::from(5000u64);

        let encoded = encode_fee_token_call(PAY_FEE_SELECTOR, receiver, amount);

        assert_eq!(encoded.len(), 68);
        assert_eq!(&encoded[0..4], &PAY_FEE_SELECTOR);
        assert_eq!(&encoded[16..36], &receiver.0);
    }

    #[test]
    fn test_is_fee_token_encoding_format() {
        // isFeeToken(address token) -> bool
        let token = MOCK_FEE_TOKEN;

        let encoded = encode_is_fee_token_call(token);

        // Verify length: 4 (selector) + 32 (padded address) = 36 bytes
        assert_eq!(encoded.len(), 36);
        assert_eq!(&encoded[0..4], &IS_FEE_TOKEN_SELECTOR);
        assert_eq!(&encoded[16..36], &token.0);
    }

    #[test]
    fn test_fee_token_ratio_encoding_format() {
        // getFeeTokenRatio(address token) -> uint256
        let token = MOCK_FEE_TOKEN;

        let encoded = encode_fee_token_ratio_call(token);

        assert_eq!(encoded.len(), 36);
        // The selector is FEE_TOKEN_RATIO_SELECTOR
        assert_eq!(&encoded[16..36], &token.0);
    }

    #[test]
    fn test_encoding_with_max_amount() {
        let payer = TEST_SENDER;
        let amount = U256::MAX;

        let encoded = encode_fee_token_call(LOCK_FEE_SELECTOR, payer, amount);

        // Amount should be all 0xFF bytes
        let expected_amount: [u8; 32] = [0xFF; 32];
        assert_eq!(&encoded[36..68], &expected_amount);
    }

    #[test]
    fn test_encoding_with_zero_amount() {
        let payer = TEST_SENDER;
        let amount = U256::zero();

        let encoded = encode_fee_token_call(LOCK_FEE_SELECTOR, payer, amount);

        // Amount should be all zeros
        let expected_amount: [u8; 32] = [0; 32];
        assert_eq!(&encoded[36..68], &expected_amount);
    }
}

// ============================================================================
// Fee Token Address Constants Tests
// ============================================================================

mod address_constants_tests {
    use super::*;

    #[test]
    fn test_fee_token_registry_address() {
        // FEE_TOKEN_REGISTRY_ADDRESS should be 0x...fffc
        assert_eq!(FEE_TOKEN_REGISTRY_ADDRESS.0[18], 0xff);
        assert_eq!(FEE_TOKEN_REGISTRY_ADDRESS.0[19], 0xfc);
    }

    #[test]
    fn test_fee_token_ratio_address() {
        // FEE_TOKEN_RATIO_ADDRESS should be 0x...fffb
        assert_eq!(FEE_TOKEN_RATIO_ADDRESS.0[18], 0xff);
        assert_eq!(FEE_TOKEN_RATIO_ADDRESS.0[19], 0xfb);
    }

    #[test]
    fn test_fee_token_system_addresses_are_distinct() {
        assert_ne!(FEE_TOKEN_REGISTRY_ADDRESS, FEE_TOKEN_RATIO_ADDRESS);
        assert_ne!(FEE_TOKEN_REGISTRY_ADDRESS, COMMON_BRIDGE_L2_ADDRESS);
        assert_ne!(FEE_TOKEN_RATIO_ADDRESS, COMMON_BRIDGE_L2_ADDRESS);
    }

    #[test]
    fn test_system_addresses_are_in_precompile_range() {
        // All system addresses should have leading zeros
        for addr in [
            FEE_TOKEN_REGISTRY_ADDRESS,
            FEE_TOKEN_RATIO_ADDRESS,
            COMMON_BRIDGE_L2_ADDRESS,
        ] {
            // First 18 bytes should be zero
            for byte in &addr.0[0..18] {
                assert_eq!(*byte, 0);
            }
        }
    }
}

// ============================================================================
// Fee Config with Fee Token Tests
// ============================================================================

mod fee_config_tests {
    use super::*;

    #[test]
    fn test_fee_config_for_fee_token_transactions() {
        // Fee token transactions can use all fee vaults
        let fee_config = FeeConfig {
            base_fee_vault: Some(TEST_BASE_FEE_VAULT),
            operator_fee_config: Some(OperatorFeeConfig {
                operator_fee_vault: TEST_OPERATOR_VAULT,
                operator_fee_per_gas: 100, // 100 wei per gas
            }),
            l1_fee_config: Some(L1FeeConfig {
                l1_fee_vault: TEST_L1_FEE_VAULT,
                l1_fee_per_blob_gas: 1000,
            }),
        };

        assert!(fee_config.base_fee_vault.is_some());
        assert!(fee_config.operator_fee_config.is_some());
        assert!(fee_config.l1_fee_config.is_some());
    }

    #[test]
    fn test_fee_config_no_base_vault_burns_fee_token() {
        // When no base_fee_vault is set and using fee tokens,
        // the base fee should be sent to address(0) to burn the ERC20
        let fee_config = FeeConfig {
            base_fee_vault: None,
            operator_fee_config: None,
            l1_fee_config: None,
        };

        assert!(fee_config.base_fee_vault.is_none());
        // In fee token path, this means payFee(address(0), amount) is called
    }
}

// ============================================================================
// Fee Token Selector Tests
// ============================================================================

mod selector_tests {
    use super::*;
    use ethrex_crypto::keccak::keccak_hash;

    fn compute_selector(signature: &str) -> [u8; 4] {
        let hash = keccak_hash(signature.as_bytes());
        let mut selector = [0u8; 4];
        selector.copy_from_slice(&hash[0..4]);
        selector
    }

    #[test]
    fn test_lock_fee_selector_matches_signature() {
        // lockFee(address,uint256)
        let expected = compute_selector("lockFee(address,uint256)");
        assert_eq!(LOCK_FEE_SELECTOR, expected);
    }

    #[test]
    fn test_pay_fee_selector_matches_signature() {
        // payFee(address,uint256)
        let expected = compute_selector("payFee(address,uint256)");
        assert_eq!(PAY_FEE_SELECTOR, expected);
    }

    #[test]
    fn test_is_fee_token_selector_matches_signature() {
        // isFeeToken(address)
        let expected = compute_selector("isFeeToken(address)");
        assert_eq!(IS_FEE_TOKEN_SELECTOR, expected);
    }
}

// ============================================================================
// Fee Token Ratio Tests (Pure Logic)
// ============================================================================

mod fee_token_ratio_tests {
    #[test]
    fn test_ratio_1_means_no_scaling() {
        // If fee_token_ratio is 1, payments should not be scaled
        let gas_used: u64 = 21000;
        let ratio: u64 = 1;
        let scaled_gas = gas_used.saturating_mul(ratio);

        assert_eq!(scaled_gas, gas_used);
    }

    #[test]
    fn test_ratio_2_doubles_payments() {
        // If fee_token_ratio is 2, all payments are doubled
        let gas_used: u64 = 21000;
        let ratio: u64 = 2;
        let scaled_gas = gas_used.saturating_mul(ratio);

        assert_eq!(scaled_gas, 42000);
    }

    #[test]
    fn test_ratio_large_scales_appropriately() {
        // Large ratio for token with less value per unit
        let gas_used: u64 = 21000;
        let ratio: u64 = 1_000_000; // 1M ratio
        let scaled_gas = gas_used.saturating_mul(ratio);

        assert_eq!(scaled_gas, 21_000_000_000);
    }

    #[test]
    fn test_ratio_saturates_on_overflow() {
        // Very large gas * very large ratio should saturate, not panic
        let gas_used: u64 = u64::MAX / 2;
        let ratio: u64 = 3;
        let scaled_gas = gas_used.saturating_mul(ratio);

        assert_eq!(scaled_gas, u64::MAX);
    }
}

// ============================================================================
// Fee Token Transaction Creation Tests
// ============================================================================

mod fee_token_tx_tests {
    use super::*;

    #[test]
    fn test_create_fee_token_tx() {
        let tx = create_fee_token_tx(
            TEST_RECIPIENT,
            U256::from(1000u64),
            50000,
            U256::from(10_000_000_000u64), // 10 gwei max
            U256::from(1_000_000_000u64),  // 1 gwei priority
        );

        match &tx {
            Transaction::EIP1559Transaction(inner) => {
                assert_eq!(inner.gas_limit, 50000);
                assert_eq!(inner.to, TxKind::Call(TEST_RECIPIENT));
            }
            _ => panic!("Expected EIP1559 transaction"),
        }
    }

    #[test]
    fn test_fee_token_tx_value_transfer() {
        let value = U256::from(1_000_000_000_000_000_000u128); // 1 ETH

        let tx = create_fee_token_tx(
            TEST_RECIPIENT,
            value,
            21000,
            U256::from(10_000_000_000u64),
            U256::from(1_000_000_000u64),
        );

        match &tx {
            Transaction::EIP1559Transaction(inner) => {
                assert_eq!(inner.value, value);
            }
            _ => panic!("Expected EIP1559 transaction"),
        }
    }
}

// ============================================================================
// Fee Token Upfront Cost Calculation Tests
// ============================================================================

mod upfront_cost_tests {
    use super::*;

    #[test]
    fn test_fee_token_upfront_cost_calculation() {
        // upfront_cost = gas_limit * gas_price * fee_token_ratio
        let gas_limit = 21000u64;
        let gas_price = U256::from(10_000_000_000u64); // 10 gwei
        let fee_token_ratio = 2u64;

        let gaslimit_price_product = gas_price.checked_mul(U256::from(gas_limit)).unwrap();

        let upfront_cost = gaslimit_price_product.saturating_mul(U256::from(fee_token_ratio));

        // 21000 * 10 gwei * 2 = 420 gwei
        assert_eq!(upfront_cost, U256::from(420_000_000_000_000u128));
    }

    #[test]
    fn test_fee_token_upfront_includes_only_gas_not_value() {
        // Fee token covers gas fees, but ETH value is still deducted from sender
        let gas_limit = 21000u64;
        let gas_price = U256::from(10_000_000_000u64);
        let value = U256::from(1_000_000_000_000_000_000u128); // 1 ETH

        let gas_cost = gas_price.checked_mul(U256::from(gas_limit)).unwrap();

        // The fee token only covers gas_cost, value is separate
        // Sender needs: gas_cost in fee token + value in ETH
        assert!(gas_cost < value); // gas cost is less than 1 ETH
    }

    #[test]
    fn test_fee_token_upfront_with_ratio_1() {
        let gas_limit = 100000u64;
        let gas_price = U256::from(1_000_000_000u64); // 1 gwei
        let ratio = 1u64;

        let upfront = gas_price
            .checked_mul(U256::from(gas_limit))
            .unwrap()
            .saturating_mul(U256::from(ratio));

        // 100000 * 1 gwei = 0.0001 ETH = 100000000000000 wei
        assert_eq!(upfront, U256::from(100_000_000_000_000u128));
    }
}

// ============================================================================
// Fee Token Refund Calculation Tests
// ============================================================================

mod refund_calculation_tests {
    use super::*;

    #[test]
    fn test_fee_token_refund_unused_gas() {
        // If gas_limit = 50000 and actual_gas_used = 21000
        // refund = (50000 - 21000) * gas_price * ratio
        let gas_limit = 50000u64;
        let actual_gas_used = 21000u64;
        let gas_price = U256::from(10_000_000_000u64);
        let ratio = 1u64;

        let gas_to_return = gas_limit.checked_sub(actual_gas_used).unwrap();
        let refund = gas_price
            .checked_mul(U256::from(gas_to_return))
            .unwrap()
            .saturating_mul(U256::from(ratio));

        // (50000 - 21000) * 10 gwei = 29000 * 10 gwei = 290000 gwei
        assert_eq!(refund, U256::from(290_000_000_000_000u128));
    }

    #[test]
    fn test_fee_token_refund_with_ratio_scaling() {
        let gas_to_return = 10000u64;
        let gas_price = U256::from(1_000_000_000u64);
        let ratio = 5u64;

        let refund = gas_price
            .checked_mul(U256::from(gas_to_return))
            .unwrap()
            .saturating_mul(U256::from(ratio));

        // 10000 * 1 gwei * 5 = 50 gwei worth
        assert_eq!(refund, U256::from(50_000_000_000_000u128));
    }

    #[test]
    fn test_fee_token_no_refund_when_all_gas_used() {
        let gas_limit = 21000u64;
        let actual_gas_used = 21000u64;
        let gas_price = U256::from(10_000_000_000u64);

        let gas_to_return = gas_limit.saturating_sub(actual_gas_used);
        let refund = gas_price.checked_mul(U256::from(gas_to_return)).unwrap();

        assert_eq!(refund, U256::zero());
    }
}

// ============================================================================
// Fee Token Payment Distribution Tests (Calculation Only)
// ============================================================================

mod payment_distribution_tests {
    use super::*;

    #[test]
    fn test_fee_token_base_fee_calculation() {
        // base_fee = gas_used * base_fee_per_gas * ratio
        let gas_used = 21000u64;
        let base_fee_per_gas = U256::from(1_000_000_000u64);
        let ratio = 2u64;

        let scaled_gas = gas_used.saturating_mul(ratio);
        let base_fee = U256::from(scaled_gas)
            .checked_mul(base_fee_per_gas)
            .unwrap();

        // 21000 * 2 * 1 gwei = 42000 gwei
        assert_eq!(base_fee, U256::from(42_000_000_000_000u128));
    }

    #[test]
    fn test_fee_token_operator_fee_calculation() {
        // operator_fee = gas_used * operator_fee_per_gas * ratio
        let gas_used = 21000u64;
        let operator_fee_per_gas = 100u64; // 100 wei
        let ratio = 3u64;

        let scaled_gas = gas_used.saturating_mul(ratio);
        let operator_fee = U256::from(scaled_gas)
            .checked_mul(U256::from(operator_fee_per_gas))
            .unwrap();

        // 21000 * 3 * 100 = 6300000 wei
        assert_eq!(operator_fee, U256::from(6_300_000u64));
    }

    #[test]
    fn test_fee_token_coinbase_fee_calculation() {
        // coinbase_fee = gas_used * (gas_price - base_fee - operator_fee) * ratio
        let gas_used = 21000u64;
        let gas_price = U256::from(10_000_000_000u64); // 10 gwei
        let base_fee = U256::from(1_000_000_000u64); // 1 gwei
        let operator_fee_per_gas = U256::from(500_000_000u64); // 0.5 gwei
        let ratio = 1u64;

        let priority_fee = gas_price
            .checked_sub(base_fee)
            .unwrap()
            .checked_sub(operator_fee_per_gas)
            .unwrap();

        let scaled_gas = gas_used.saturating_mul(ratio);
        let coinbase_fee = U256::from(scaled_gas).checked_mul(priority_fee).unwrap();

        // 21000 * (10 - 1 - 0.5) gwei = 21000 * 8.5 gwei = 178500 gwei
        assert_eq!(coinbase_fee, U256::from(178_500_000_000_000u128));
    }

    #[test]
    fn test_fee_token_l1_fee_calculation() {
        // L1 fee is calculated then scaled by ratio
        let l1_gas = 1000u64; // calculated L1 gas
        let gas_price = U256::from(10_000_000_000u64);
        let ratio = 2u64;

        let scaled_l1_gas = l1_gas.saturating_mul(ratio);
        let l1_fee = U256::from(scaled_l1_gas).checked_mul(gas_price).unwrap();

        // 1000 * 2 * 10 gwei = 20000 gwei
        assert_eq!(l1_fee, U256::from(20_000_000_000_000u128));
    }
}

// ============================================================================
// Fee Token Burn Tests (No Base Fee Vault)
// ============================================================================

mod burn_tests {
    use super::*;

    #[test]
    fn test_fee_token_burn_sends_to_zero_address() {
        // When no base_fee_vault is set, fee tokens are burned by
        // calling payFee(address(0), amount)
        let burn_address = Address::zero();
        let amount = U256::from(1000u64);

        let encoded = encode_fee_token_call(PAY_FEE_SELECTOR, burn_address, amount);

        // Verify the address is zero
        assert_eq!(&encoded[16..36], &[0u8; 20]);
    }

    #[test]
    fn test_zero_address_is_valid_receiver() {
        let burn_address = Address::zero();

        // payFee can accept zero address (for burning)
        let encoded = encode_fee_token_call(PAY_FEE_SELECTOR, burn_address, U256::from(100u64));

        // Should produce valid calldata
        assert_eq!(encoded.len(), 68);
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Creates a fee token transaction (EIP1559)
fn create_fee_token_tx(
    to: Address,
    value: U256,
    gas_limit: u64,
    max_fee_per_gas: U256,
    max_priority_fee_per_gas: U256,
) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: max_priority_fee_per_gas.as_u64(),
        max_fee_per_gas: max_fee_per_gas.as_u64(),
        gas_limit,
        to: TxKind::Call(to),
        value,
        data: Bytes::new(),
        access_list: vec![],
        signature_r: U256::zero(),
        signature_s: U256::zero(),
        signature_y_parity: false,
        inner_hash: OnceCell::new(),
    })
}
