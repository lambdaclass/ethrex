//! Deposit (privileged transaction) handler.
//!
//! Privileged transactions are L1->L2 deposits routed through the
//! CommonBridgeL2 contract. The on-chain TX has:
//!   - `to`:    0x...ffff  (bridge contract)
//!   - `input`: selector (4 B) + abi.encode(address recipient)
//!   - `value`: deposit amount
//!
//! The EVM executes the bridge contract which internally credits the
//! recipient, but the guest program has no EVM -- so we parse the
//! calldata to find the actual recipient and credit them directly.

use ethrex_common::Address;
use ethrex_common::types::{Transaction, TxKind};

use super::constants::COMMON_BRIDGE_L2_ADDRESS;
use crate::common::app_execution::AppCircuitError;
use crate::common::app_state::AppState;

/// Handle a privileged (deposit) transaction.
pub fn handle_privileged_tx(state: &mut AppState, tx: &Transaction) -> Result<(), AppCircuitError> {
    let value = tx.value();
    if value.is_zero() {
        return Ok(());
    }

    let TxKind::Call(to) = tx.to() else {
        return Ok(());
    };

    let data = tx.data();
    if to == COMMON_BRIDGE_L2_ADDRESS && data.len() >= 36 {
        // Decode the actual recipient from calldata:
        //   data[0..4]   = function selector
        //   data[4..36]  = abi.encode(address) = 12 zero-padding bytes + 20 byte address
        let recipient = Address::from_slice(&data[16..36]);
        state.credit_balance(recipient, value)?;
    } else {
        // Fallback: credit `to` directly (non-bridge privileged TX).
        state.credit_balance(to, value)?;
    }

    Ok(())
}
