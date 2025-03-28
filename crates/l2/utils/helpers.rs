use ethrex_common::{
    types::{Receipt, Transaction, TxKind},
    H256,
};
use ethrex_l2_sdk::COMMON_BRIDGE_L2_ADDRESS;
use std::str::FromStr;

use super::error::UtilsError;

#[allow(clippy::expect_used)]
pub fn is_withdrawal_l2(tx: &Transaction, receipt: &Receipt) -> Result<bool, UtilsError> {
    // WithdrawalInitiated(address,address,uint256)
    let withdrawal_event_selector: H256 =
        H256::from_str("bb2689ff876f7ef453cf8865dde5ab10349d222e2e1383c5152fbdb083f02da2")
            .map_err(|e| UtilsError::WithdrawalSelectorError(e.to_string()))?;

    let is_withdrawal = match tx.to() {
        TxKind::Call(to) if to == COMMON_BRIDGE_L2_ADDRESS => receipt.logs.iter().any(|log| {
            log.topics
                .iter()
                .any(|topic| *topic == withdrawal_event_selector)
        }),
        _ => false,
    };
    Ok(is_withdrawal)
}

pub fn is_deposit_l2(tx: &Transaction) -> bool {
    matches!(tx, Transaction::PrivilegedL2Transaction(_tx))
}
