//! The guest L1 program must reject L2-only transaction types (`FeeToken`
//! 0x7d, `Privileged` 0x7e), matching `Blockchain::validate_l1_transaction_types`
//! on full nodes. A privileged transaction takes its sender from an unsigned,
//! caller-chosen `from`, so accepting one in a stateless proof would let a
//! block forge a sender that every L1 full node rejects.

use std::sync::Arc;

use ethrex_common::InvalidBlockError;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, EIP1559Transaction, FeeTokenTransaction,
    PrivilegedL2Transaction, Transaction,
};
use ethrex_common::validate_l1_transaction_types;
use ethrex_crypto::NativeCrypto;
use ethrex_guest_program::common::ExecutionError;
use ethrex_guest_program::l1::execute_l1_blocks;

fn block_with(transactions: Vec<Transaction>) -> Block {
    Block::new(
        BlockHeader::default(),
        BlockBody {
            transactions,
            ..Default::default()
        },
    )
}

fn assert_rejects_with_type(transactions: Vec<Transaction>, expected_type: u8) {
    let block = block_with(transactions);
    let result = execute_l1_blocks(
        &[block],
        ExecutionWitness::default(),
        Arc::new(NativeCrypto),
    );
    match result.map(|_| ()) {
        Err(ExecutionError::BlockValidation(InvalidBlockError::UnsupportedTransactionType(
            tx_type,
        ))) if tx_type == expected_type => {}
        other => panic!("expected UnsupportedTransactionType({expected_type:#x}), got {other:?}"),
    }
}

#[test]
fn rejects_privileged_transactions() {
    assert_rejects_with_type(
        vec![
            Transaction::EIP1559Transaction(EIP1559Transaction::default()),
            Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction::default()),
        ],
        0x7e,
    );
}

#[test]
fn rejects_fee_token_transactions() {
    assert_rejects_with_type(
        vec![Transaction::FeeTokenTransaction(
            FeeTokenTransaction::default(),
        )],
        0x7d,
    );
}

#[test]
fn accepts_l1_transaction_types() {
    let block = block_with(vec![Transaction::EIP1559Transaction(
        EIP1559Transaction::default(),
    )]);
    assert!(validate_l1_transaction_types(&block).is_ok());

    let empty = block_with(vec![]);
    assert!(validate_l1_transaction_types(&empty).is_ok());
}
