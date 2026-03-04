//! System contract call handler.
//!
//! Handles calls to known system contracts (L1Messenger, FeeTokenRegistry, etc.).

use ethrex_common::Address;
use ethrex_common::types::Transaction;

use super::constants::{
    COMMON_BRIDGE_L2_ADDRESS, FEE_TOKEN_RATIO_ADDRESS, FEE_TOKEN_REGISTRY_ADDRESS,
    L2_TO_L1_MESSENGER_ADDRESS, SYSTEM_CALL_GAS,
};
use crate::common::app_execution::AppCircuitError;
use crate::common::app_state::AppState;

/// Handle a system contract call (L1Messenger, FeeTokenRegistry, etc.).
///
/// Returns the fixed gas cost for this operation.
pub fn handle_system_call(
    _state: &mut AppState,
    _tx: &Transaction,
    _sender: Address,
    _target: Address,
) -> Result<u64, AppCircuitError> {
    // TODO: Implement system contract logic per contract.
    // For now, just charge a fixed gas cost.
    Ok(SYSTEM_CALL_GAS)
}

/// Check if an address is a known system contract.
pub fn is_system_contract(address: Address) -> bool {
    address == COMMON_BRIDGE_L2_ADDRESS
        || address == L2_TO_L1_MESSENGER_ADDRESS
        || address == FEE_TOKEN_REGISTRY_ADDRESS
        || address == FEE_TOKEN_RATIO_ADDRESS
}
