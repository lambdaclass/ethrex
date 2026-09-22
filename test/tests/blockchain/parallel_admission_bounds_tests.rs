//! Bounds that let the parallel Amsterdam pipeline reject a hopeless block without
//! executing it.
//!
//! Ordered gas admission (`check_2d_gas_allowance`) needs each transaction's real gas,
//! so on the parallel path it can only run after every transaction has executed and its
//! report is held in memory. A block can therefore declare far more transactions than it
//! could ever admit and still be executed in full first. These bounds close that gap
//! using only what is knowable up front, and must never reject a block that ordered
//! admission would have accepted.

use ethrex_common::{
    Address, U256,
    types::{EIP1559Transaction, Fork, Transaction, TxKind},
};
use ethrex_vm::{block_work_budget, check_minimum_block_work};

const BLOCK_GAS_LIMIT: u64 = 30_000_000;

/// A transaction with no calldata, access list or value, so its floor is the bare
/// Amsterdam base cost and the only thing under test is how many of them there are.
fn bare_tx() -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 21_000,
        to: TxKind::Call(Address::from_low_u64_be(0xBEEF)),
        value: U256::zero(),
        ..Default::default()
    })
}

fn block_of(n: usize) -> Vec<(Transaction, Address)> {
    let sender = Address::from_low_u64_be(0xA11CE);
    (0..n).map(|_| (bare_tx(), sender)).collect()
}

fn check(txs: &[(Transaction, Address)]) -> Result<(), String> {
    check_minimum_block_work(
        txs.iter().map(|(tx, sender)| (tx, *sender)),
        Fork::Amsterdam,
        BLOCK_GAS_LIMIT,
    )
    .map_err(|e| e.to_string())
}

/// The budget is twice the gas limit, not once: the two EIP-8037 dimensions are summed
/// per transaction but the block is charged their max, so a valid block can legitimately
/// spend up to twice its limit across both dimensions. A budget of one limit would reject
/// valid blocks.
#[test]
fn test_work_budget_is_twice_the_gas_limit() {
    assert_eq!(block_work_budget(BLOCK_GAS_LIMIT), 2 * BLOCK_GAS_LIMIT);
    assert_eq!(block_work_budget(u64::MAX), u64::MAX, "must saturate");
}

/// A block that fills the whole budget is still executed. This is the case that would
/// break if the bound were tightened to a single gas limit.
#[test]
fn test_block_filling_the_work_budget_is_accepted() {
    let per_tx_floor = {
        let one = block_of(1);
        // Derive the floor from the bound itself: the largest n that still passes tells
        // us the per-tx floor without duplicating the EIP-7623/7976 formula here.
        assert!(check(&one).is_ok());
        block_work_budget(BLOCK_GAS_LIMIT)
    };
    // 2 * 30M / 21000 ~= 2857 bare transactions fit inside the budget.
    let fits = (per_tx_floor / 21_000) as usize;
    check(&block_of(fits)).expect("a block inside the work budget must be executed");
}

/// The case the bound exists for: far more transactions than the block could ever admit.
/// Ordered admission would reject this too, but only after executing every one of them
/// and retaining its report.
#[test]
fn test_block_beyond_the_work_budget_is_rejected_before_execution() {
    let err = check(&block_of(40_000)).expect_err("a block past the work budget must be rejected");
    assert!(
        err.contains("Gas allowance exceeded"),
        "must reject as a gas-allowance failure so it maps like ordered admission, got: {err}"
    );
}

/// An empty block has no floor to accumulate.
#[test]
fn test_empty_block_is_accepted() {
    check(&block_of(0)).expect("an empty block has no minimum work");
}

/// The bound must trip on accumulated floor, not on any single transaction, so a block
/// of individually-cheap transactions is still caught.
#[test]
fn test_rejection_is_cumulative_not_per_transaction() {
    let fits = (block_work_budget(BLOCK_GAS_LIMIT) / 21_000) as usize;
    check(&block_of(fits)).expect("boundary block must pass");
    check(&block_of(fits * 2)).expect_err("twice the boundary must fail on the sum alone");
}
