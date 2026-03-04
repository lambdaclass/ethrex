//! Simple ETH transfer handler.
//!
//! Handles transactions with no calldata (pure ETH value transfers).

use ethrex_common::Address;

use super::constants::ETH_TRANSFER_GAS;
use crate::common::app_execution::AppCircuitError;
use crate::common::app_state::AppState;

/// Handle a simple ETH transfer (no calldata).
///
/// Returns the fixed gas cost for this operation.
pub fn handle_eth_transfer(
    state: &mut AppState,
    sender: Address,
    to: Address,
    value: ethrex_common::U256,
) -> Result<u64, AppCircuitError> {
    state.transfer_eth(sender, to, value)?;
    Ok(ETH_TRANSFER_GAS)
}
