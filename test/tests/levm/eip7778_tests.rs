//! EIP-7778: Block Gas Limit Accounting Without Refunds
//!
//! Tests for the separation of block-level gas accounting (pre-refund)
//! from user-level gas accounting (post-refund).
//!
//! Note: gas_spent is tracked in ExecutionReport (VM layer) only,
//! NOT in Receipt structs. Receipts use cumulative_gas_used which is
//! post-refund for receipt accounting.

use bytes::Bytes;
use ethrex_common::types::{Receipt, TxType};
use ethrex_levm::errors::{ExecutionReport, TxResult};

/// Test that Receipt RLP encoding/decoding works correctly
#[test]
fn test_receipt_encoding() {
    let receipt = Receipt::new(TxType::EIP1559, true, 21000, vec![]);

    // Encode and decode
    let encoded = receipt.encode_inner_with_bloom();
    let decoded = ethrex_common::types::ReceiptWithBloom::decode_inner(&encoded).unwrap();

    assert!(decoded.succeeded);
    assert_eq!(decoded.cumulative_gas_used, 21000);
}

#[test]
fn test_execution_report_has_both_gas_fields() {
    // Test that ExecutionReport structure contains both gas_used and gas_spent
    // gas_used: pre-refund (for block-level accounting per EIP-7778)
    // gas_spent: post-refund (what user actually pays)
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
    // Test that receipts encode/decode correctly for both legacy and typed transactions

    let receipt_legacy = Receipt::new(TxType::Legacy, true, 21000, vec![]);
    let receipt_eip1559 = Receipt::new(TxType::EIP1559, true, 25000, vec![]);

    // Both should encode/decode correctly
    let encoded_legacy = receipt_legacy.encode_inner_with_bloom();
    let encoded_eip1559 = receipt_eip1559.encode_inner_with_bloom();

    let decoded_legacy =
        ethrex_common::types::ReceiptWithBloom::decode_inner(&encoded_legacy).unwrap();
    let decoded_eip1559 =
        ethrex_common::types::ReceiptWithBloom::decode_inner(&encoded_eip1559).unwrap();

    // Fields should be correctly decoded
    assert_eq!(decoded_legacy.tx_type, TxType::Legacy);
    assert_eq!(decoded_legacy.cumulative_gas_used, 21000);
    assert_eq!(decoded_eip1559.tx_type, TxType::EIP1559);
    assert_eq!(decoded_eip1559.cumulative_gas_used, 25000);
}
