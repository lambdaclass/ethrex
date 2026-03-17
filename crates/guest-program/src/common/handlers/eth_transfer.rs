//! Simple ETH transfer handler.
//!
//! Handles transactions with no calldata (pure ETH value transfers).

use ethrex_common::Address;

use crate::common::app_execution::AppCircuitError;
use crate::common::app_state::AppState;

/// Handle a simple ETH transfer (no calldata).
///
/// Gas is NOT returned here — the caller uses the block header's gas_used.
pub fn handle_eth_transfer(
    state: &mut AppState,
    sender: Address,
    to: Address,
    value: ethrex_common::U256,
) -> Result<(), AppCircuitError> {
    state.transfer_eth(sender, to, value)?;
    Ok(())
}
