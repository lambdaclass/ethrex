use ethrex_common::{
    types::{Receipt, Transaction, TxKind},
    H256,
};
use ethrex_l2_sdk::COMMON_BRIDGE_L2_ADDRESS;

// this selector corresponds to this function signature:
// WithdrawalInitiated(address,address,uint256)
const WITHDRAWAL_EVENT_SELECTOR: H256 = H256([
    0xbb, 0x26, 0x89, 0xff, 0x87, 0x6f, 0x7e, 0xf4, 0x53, 0xcf, 0x88, 0x65, 0xdd, 0xe5, 0xab, 0x10,
    0x34, 0x9d, 0x22, 0x2e, 0x2e, 0x13, 0x83, 0xc5, 0x15, 0x2f, 0xbd, 0xb0, 0x83, 0xf0, 0x2d, 0xa2,
]);

pub fn is_withdrawal_l2(tx: &Transaction, receipt: &Receipt) -> bool {
    if let TxKind::Call(to) = tx.to() {
        if to == COMMON_BRIDGE_L2_ADDRESS {
            receipt
                .logs
                .iter()
                .any(|log| log.topics.contains(&WITHDRAWAL_EVENT_SELECTOR))
        } else {
            false
        }
    } else {
        false
    }
}

pub fn is_deposit_l2(tx: &Transaction) -> bool {
    matches!(tx, Transaction::PrivilegedL2Transaction(_tx))
}
