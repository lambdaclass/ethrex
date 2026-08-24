//! EIP-8037 two-dimensional block inclusion gate.
//!
//! The gate is flat: a tx's worst-case contribution to each dimension is measured
//! against that dimension's remaining budget with no credit for what the tx will
//! actually spend. Specifically, no intrinsic gas is subtracted and no allowance is
//! made for the state gas the tx charges at its top frame. A client that credited
//! either would accept transactions the spec rejects.
//!
//! Only the execution dimension is capped at `TX_MAX_GAS_LIMIT`; the state dimension
//! measures the full `tx.gas`.

use ethrex_common::{
    Address, U256,
    constants::TX_MAX_GAS_LIMIT_AMSTERDAM,
    types::{EIP1559Transaction, Transaction, TxKind},
};
use ethrex_vm::check_2d_gas_allowance;

/// A tx carrying no calldata, no access list and no value, so its only relevant
/// property is `gas_limit`.
fn tx_with_gas(gas_limit: u64) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit,
        to: TxKind::Call(Address::from_low_u64_be(0xBEEF)),
        value: U256::zero(),
        ..Default::default()
    })
}

const BLOCK_GAS_LIMIT: u64 = 30_000_000;

/// Execution gas already consumed, chosen so the remaining budget
/// ([`SUB_CAP_AVAILABLE`]) sits below `TX_MAX_GAS_LIMIT`. Above the cap the
/// execution contribution is clamped and the clamp, not the flat gate, would decide
/// these cases.
const SUB_CAP_USED: u64 = 25_000_000;
const SUB_CAP_AVAILABLE: u64 = BLOCK_GAS_LIMIT - SUB_CAP_USED;

#[test]
fn test_execution_dim_exact_fit_is_accepted() {
    // tx.gas exactly equals the remaining execution budget: the control case, valid
    // under both a flat gate and one that credits the intrinsic or state charge.
    check_2d_gas_allowance(
        &tx_with_gas(SUB_CAP_AVAILABLE),
        SUB_CAP_USED,
        0,
        BLOCK_GAS_LIMIT,
    )
    .expect("a tx whose gas exactly fills the remaining budget must be accepted");
}

#[test]
fn test_execution_dim_one_above_is_rejected() {
    // One gas above the remaining budget. A client subtracting the intrinsic cost
    // would have slack here and wrongly accept.
    let err = check_2d_gas_allowance(
        &tx_with_gas(SUB_CAP_AVAILABLE + 1),
        SUB_CAP_USED,
        0,
        BLOCK_GAS_LIMIT,
    )
    .expect_err("one gas over the remaining execution budget must be rejected");
    assert!(
        err.to_string().contains("regular dim"),
        "expected the execution dimension to reject, got: {err}"
    );
}

#[test]
fn test_execution_dim_no_credit_for_top_frame_state_charge() {
    // The gate gives no credit for state gas the tx will charge at its top frame,
    // so a tx over by any amount is rejected even when the state budget is untouched.
    // 11_000 is CREATE_ACCESS, the state charge a creation pays at its top frame.
    for over_by in [1, 1_000, 11_000] {
        assert!(
            check_2d_gas_allowance(
                &tx_with_gas(SUB_CAP_AVAILABLE + over_by),
                SUB_CAP_USED,
                0,
                BLOCK_GAS_LIMIT
            )
            .is_err(),
            "over by {over_by} must be rejected regardless of the untouched state budget"
        );
    }
}

#[test]
fn test_execution_dim_contribution_is_capped_at_tx_max_gas_limit() {
    // Only the execution dimension is capped: a tx asking for more than
    // TX_MAX_GAS_LIMIT contributes just the cap, so it fits a budget smaller than
    // its own gas limit.
    let block_gas_limit = TX_MAX_GAS_LIMIT_AMSTERDAM * 2;
    let tx_gas = TX_MAX_GAS_LIMIT_AMSTERDAM + 5_000_000;
    // Leave exactly the cap free in the execution dimension.
    let used = block_gas_limit - TX_MAX_GAS_LIMIT_AMSTERDAM;
    check_2d_gas_allowance(&tx_with_gas(tx_gas), used, 0, block_gas_limit)
        .expect("the execution contribution is capped at TX_MAX_GAS_LIMIT, so this fits");

    // One gas less available and the capped contribution no longer fits.
    check_2d_gas_allowance(&tx_with_gas(tx_gas), used + 1, 0, block_gas_limit)
        .expect_err("the capped contribution must still be measured against the budget");
}

#[test]
fn test_state_dim_uses_full_tx_gas_uncapped() {
    // The state dimension is not capped, so the same over-cap tx is rejected on the
    // state side once the state budget drops below the full tx.gas.
    let block_gas_limit = TX_MAX_GAS_LIMIT_AMSTERDAM * 2;
    let tx_gas = TX_MAX_GAS_LIMIT_AMSTERDAM + 5_000_000;
    let state_used = block_gas_limit - tx_gas + 1;
    let err = check_2d_gas_allowance(&tx_with_gas(tx_gas), 0, state_used, block_gas_limit)
        .expect_err("the state dimension measures the full tx.gas, uncapped");
    assert!(
        err.to_string().contains("state dim"),
        "expected the state dimension to reject, got: {err}"
    );
}

#[test]
fn test_state_dim_exact_fit_is_accepted() {
    let tx_gas = 5_000_000;
    let state_used = BLOCK_GAS_LIMIT - tx_gas;
    check_2d_gas_allowance(&tx_with_gas(tx_gas), 0, state_used, BLOCK_GAS_LIMIT)
        .expect("a tx whose gas exactly fills the remaining state budget must be accepted");
}

#[test]
fn test_dimensions_are_tracked_independently() {
    // A tx can fit the execution budget while overflowing the state budget, which is
    // the whole point of tracking two dimensions against one block gas limit.
    let tx_gas = 10_000_000;
    check_2d_gas_allowance(&tx_with_gas(tx_gas), 20_000_000, 0, BLOCK_GAS_LIMIT)
        .expect("execution budget exactly fits");
    check_2d_gas_allowance(&tx_with_gas(tx_gas), 0, 20_000_000, BLOCK_GAS_LIMIT)
        .expect("state budget exactly fits");
    assert!(
        check_2d_gas_allowance(&tx_with_gas(tx_gas), 20_000_001, 0, BLOCK_GAS_LIMIT).is_err(),
        "execution dimension must reject on its own"
    );
    assert!(
        check_2d_gas_allowance(&tx_with_gas(tx_gas), 0, 20_000_001, BLOCK_GAS_LIMIT).is_err(),
        "state dimension must reject on its own"
    );
}

#[test]
fn test_exhausted_budget_rejects_any_nonzero_tx() {
    // Saturating arithmetic must not turn an already-overfull block into free space.
    assert!(
        check_2d_gas_allowance(
            &tx_with_gas(21_000),
            BLOCK_GAS_LIMIT + 1,
            0,
            BLOCK_GAS_LIMIT
        )
        .is_err(),
        "an over-limit execution total leaves no budget"
    );
    assert!(
        check_2d_gas_allowance(
            &tx_with_gas(21_000),
            0,
            BLOCK_GAS_LIMIT + 1,
            BLOCK_GAS_LIMIT
        )
        .is_err(),
        "an over-limit state total leaves no budget"
    );
}
