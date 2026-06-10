//! Tests for `validate_l1_transaction_types`: L1 blocks must not contain
//! L2-only transaction types (`FeeToken` 0x7d, `Privileged` 0x7e).

use ethrex_common::InvalidBlockError;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, EIP1559Transaction, FeeTokenTransaction,
    PrivilegedL2Transaction, Transaction,
};
use ethrex_common::validate_l1_transaction_types;

fn block_with(transactions: Vec<Transaction>) -> Block {
    Block::new(
        BlockHeader::default(),
        BlockBody {
            transactions,
            ..Default::default()
        },
    )
}

#[test]
fn accepts_blocks_without_l2_only_transactions() {
    let block = block_with(vec![Transaction::EIP1559Transaction(
        EIP1559Transaction::default(),
    )]);
    assert!(validate_l1_transaction_types(&block).is_ok());

    let empty = block_with(vec![]);
    assert!(validate_l1_transaction_types(&empty).is_ok());
}

#[test]
fn rejects_privileged_transactions() {
    let block = block_with(vec![
        Transaction::EIP1559Transaction(EIP1559Transaction::default()),
        Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction::default()),
    ]);
    assert!(matches!(
        validate_l1_transaction_types(&block),
        Err(InvalidBlockError::UnsupportedTransactionType(0x7e))
    ));
}

#[test]
fn rejects_fee_token_transactions() {
    let block = block_with(vec![Transaction::FeeTokenTransaction(
        FeeTokenTransaction::default(),
    )]);
    assert!(matches!(
        validate_l1_transaction_types(&block),
        Err(InvalidBlockError::UnsupportedTransactionType(0x7d))
    ));
}
