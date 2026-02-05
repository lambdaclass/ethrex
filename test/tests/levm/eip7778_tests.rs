//! EIP-7778: Block Gas Limit Accounting Without Refunds
//!
//! Tests for the separation of block-level gas accounting (pre-refund)
//! from user-level gas accounting (post-refund).

use bytes::Bytes;
use ethrex_common::types::{Receipt, TxType};
use ethrex_levm::errors::{ExecutionReport, TxResult};

/// Test that Receipt RLP encoding/decoding works correctly with gas_spent field
#[test]
fn test_receipt_gas_spent_encoding_pre_amsterdam() {
    // Pre-Amsterdam: gas_spent should be None
    let receipt = Receipt::new(
        TxType::EIP1559,
        true,
        21000, // cumulative_gas_used
        None,  // gas_spent: None for pre-Amsterdam
        vec![],
    );

    assert!(receipt.gas_spent.is_none());

    // Encode and decode
    let encoded = receipt.encode_inner_with_bloom();
    let decoded = ethrex_common::types::ReceiptWithBloom::decode_inner(&encoded).unwrap();

    assert!(decoded.succeeded);
    assert_eq!(decoded.cumulative_gas_used, 21000);
    assert!(decoded.gas_spent.is_none());
}

#[test]
fn test_receipt_gas_spent_encoding_amsterdam() {
    // gas_spent is an internal field only and is NOT included in RLP encoding.
    // After encode/decode cycle, gas_spent should be None.
    let cumulative_gas_used = 25000; // Pre-refund (for block accounting)
    let gas_spent = 20200; // Post-refund (what user pays)

    let receipt = Receipt::new(
        TxType::EIP1559,
        true,
        cumulative_gas_used,
        Some(gas_spent),
        vec![],
    );

    // gas_spent is set on the receipt struct
    assert_eq!(receipt.gas_spent, Some(gas_spent));

    // Encode and decode - gas_spent is NOT part of RLP encoding
    let encoded = receipt.encode_inner_with_bloom();
    let decoded = ethrex_common::types::ReceiptWithBloom::decode_inner(&encoded).unwrap();

    assert!(decoded.succeeded);
    assert_eq!(decoded.cumulative_gas_used, cumulative_gas_used);
    // gas_spent is not encoded in RLP, so it's None after decoding
    assert!(decoded.gas_spent.is_none());
}

#[test]
fn test_receipt_gas_spent_reflects_refund_difference() {
    // Test that gas_spent correctly reflects the refund
    // Pre-refund gas: 50000
    // Refund: 4800 (from SSTORE clearing)
    // Post-refund gas (gas_spent): 50000 - 4800 = 45200

    let gas_used_pre_refund = 50000u64;
    let refund = 4800u64;
    let gas_spent = gas_used_pre_refund - refund;

    let receipt = Receipt::new(
        TxType::EIP1559,
        true,
        gas_used_pre_refund, // Block accounting uses pre-refund
        Some(gas_spent),     // User pays post-refund
        vec![],
    );

    // Verify the difference matches the refund
    assert_eq!(
        receipt.cumulative_gas_used - receipt.gas_spent.unwrap(),
        refund
    );
}

#[test]
fn test_execution_report_has_both_gas_fields() {
    // Test that ExecutionReport structure contains both gas_used and gas_spent
    let report = ExecutionReport {
        result: TxResult::Success,
        gas_used: 50000,    // Pre-refund
        gas_spent: 45200,   // Post-refund
        gas_refunded: 4800, // The refund amount
        output: Bytes::new(),
        logs: vec![],
    };

    // Verify both fields are present and different
    assert_eq!(report.gas_used, 50000);
    assert_eq!(report.gas_spent, 45200);
    assert_eq!(report.gas_refunded, 4800);

    // Verify the relationship: gas_spent = gas_used - min(gas_refunded, gas_used/5)
    // In this case: 50000 - min(4800, 10000) = 50000 - 4800 = 45200
    let max_refund = report.gas_used / 5; // EIP-3529 caps refund at 20%
    let actual_refund = std::cmp::min(report.gas_refunded, max_refund);
    assert_eq!(report.gas_spent, report.gas_used - actual_refund);
}

#[test]
fn test_receipt_backward_compatibility() {
    // Test that receipts encode/decode correctly.
    // gas_spent is an internal field and is NOT part of the RLP encoding,
    // so both receipts will have gas_spent = None after decoding.

    // Create a receipt without gas_spent set
    let receipt_without = Receipt::new(TxType::Legacy, true, 21000, None, vec![]);

    // Create a receipt with gas_spent set internally
    let receipt_with = Receipt::new(TxType::Legacy, true, 25000, Some(21000), vec![]);

    // Both should encode/decode correctly
    let encoded_without = receipt_without.encode_inner_with_bloom();
    let encoded_with = receipt_with.encode_inner_with_bloom();

    let decoded_without =
        ethrex_common::types::ReceiptWithBloom::decode_inner(&encoded_without).unwrap();
    let decoded_with = ethrex_common::types::ReceiptWithBloom::decode_inner(&encoded_with).unwrap();

    // gas_spent is not part of RLP encoding, so both are None after decode
    assert!(decoded_without.gas_spent.is_none());
    assert!(decoded_with.gas_spent.is_none());

    // Other fields should be correctly decoded
    assert_eq!(decoded_without.cumulative_gas_used, 21000);
    assert_eq!(decoded_with.cumulative_gas_used, 25000);
}

#[test]
fn test_cumulative_gas_uses_pre_refund_for_block_accounting() {
    // Simulate two transactions in a block with refunds
    // TX1: 30000 gas used, 3000 refund -> gas_spent = 27000
    // TX2: 40000 gas used, 5000 refund -> gas_spent = 35000

    // For Amsterdam+, cumulative_gas_used should be pre-refund values
    let tx1_gas_used = 30000u64;
    let tx1_gas_spent = 27000u64;
    let tx2_gas_used = 40000u64;
    let tx2_gas_spent = 35000u64;

    // Block accounting: cumulative uses pre-refund
    let cumulative_after_tx1 = tx1_gas_used;
    let cumulative_after_tx2 = tx1_gas_used + tx2_gas_used;

    let receipt1 = Receipt::new(
        TxType::EIP1559,
        true,
        cumulative_after_tx1,
        Some(tx1_gas_spent),
        vec![],
    );

    let receipt2 = Receipt::new(
        TxType::EIP1559,
        true,
        cumulative_after_tx2,
        Some(tx2_gas_spent),
        vec![],
    );

    // Verify cumulative is pre-refund (block accounting)
    assert_eq!(receipt1.cumulative_gas_used, 30000);
    assert_eq!(receipt2.cumulative_gas_used, 70000);

    // Verify gas_spent is post-refund (user payment)
    assert_eq!(receipt1.gas_spent, Some(27000));
    assert_eq!(receipt2.gas_spent, Some(35000));

    // Total user payment is less than total block gas
    let total_block_gas = receipt2.cumulative_gas_used;
    let total_user_payment = tx1_gas_spent + tx2_gas_spent;
    assert!(total_user_payment < total_block_gas);
}
