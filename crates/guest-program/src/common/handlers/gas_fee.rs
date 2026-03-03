//! Gas fee distribution handler.
//!
//! Replicates the L2 hook's gas fee distribution logic so that the guest
//! program's state transition matches the EVM exactly:
//!   1. Debit `effective_gas_price * gas_used` from sender
//!   2. Credit `priority_fee * gas_used` to coinbase
//!   3. Credit `base_fee * gas_used` to base_fee_vault (if configured)
//!   4. Credit `operator_fee * gas_used` to operator_fee_vault (if configured)

use ethrex_common::types::l2::fee_config::FeeConfig;
use ethrex_common::types::{BlockHeader, Transaction};
use ethrex_common::{Address, U256};

use crate::common::app_execution::AppCircuitError;
use crate::common::app_state::AppState;

/// Apply gas fee distribution matching the L2 EVM execution.
///
/// This replaces the previous `apply_gas_deduction` which only charged
/// `base_fee * gas_used` from the sender. The new implementation distributes
/// fees to coinbase and configured vaults exactly as the L2 hook does.
pub fn apply_gas_fee_distribution(
    state: &mut AppState,
    sender: Address,
    tx: &Transaction,
    gas_used: u64,
    block_header: &BlockHeader,
    fee_config: &FeeConfig,
) -> Result<(), AppCircuitError> {
    let base_fee = block_header.base_fee_per_gas.unwrap_or(0);
    let effective_price = tx
        .effective_gas_price(Some(base_fee))
        .unwrap_or(U256::from(base_fee));
    let gas = U256::from(gas_used);

    // 1. sender -= effective_price * gas_used
    let total_fee = effective_price.saturating_mul(gas);
    if !total_fee.is_zero() {
        state.debit_balance(sender, total_fee)?;
    }

    // 2. Compute operator_fee_per_gas (if configured)
    let op_fee_per_gas = fee_config
        .operator_fee_config
        .map(|c| U256::from(c.operator_fee_per_gas))
        .unwrap_or_default();

    // 3. priority_fee = effective_price - base_fee - operator_fee_per_gas
    let priority_fee = effective_price
        .saturating_sub(U256::from(base_fee))
        .saturating_sub(op_fee_per_gas);

    // 4. coinbase += priority_fee * gas_used
    let coinbase_credit = gas.saturating_mul(priority_fee);
    if !coinbase_credit.is_zero() {
        state.credit_balance(block_header.coinbase, coinbase_credit)?;
    }

    // 5. base_fee_vault += base_fee * gas_used (if configured)
    if let Some(vault) = fee_config.base_fee_vault {
        let credit = gas.saturating_mul(U256::from(base_fee));
        if !credit.is_zero() {
            state.credit_balance(vault, credit)?;
        }
    }

    // 6. operator_vault += operator_fee * gas_used (if configured)
    if let Some(op) = fee_config.operator_fee_config {
        let credit = gas.saturating_mul(U256::from(op.operator_fee_per_gas));
        if !credit.is_zero() {
            state.credit_balance(op.operator_fee_vault, credit)?;
        }
    }

    Ok(())
}
