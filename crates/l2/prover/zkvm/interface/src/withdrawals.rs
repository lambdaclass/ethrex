// TODO: We should move this to some kind of "common" library for the L2, but the zkvm programs
// can't depend on ethrex-l2 because of incompatible dependencies.

use std::str::FromStr;

use ethrex_common::{
    types::{Receipt, Transaction, TxKind},
    Address, H160, H256,
};

use keccak_hash::keccak;
use thiserror::Error;

pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
]);

#[derive(Debug, Error)]
pub enum Error {
    #[error("Withdrawal transaction was invalid")]
    InvalidWithdrawalTransaction,
    #[error("Failed to merkelize withdrawals")]
    FailedToMerkelize,
    #[error("Failed to create withdrawal selector")]
    WithdrawalSelector,
    #[error("Failed to get withdrawal hash")]
    WithdrawalHash,
}

pub fn get_block_withdrawals(
    txs: &[Transaction],
    receipts: &[Receipt],
) -> Result<Vec<H256>, Error> {
    txs.iter()
        .zip(receipts.iter())
        .filter(|(tx, receipt)| is_withdrawal_l2(tx, receipt))
        .map(|(withdrawal, _)| get_withdrawal_hash(withdrawal).ok_or(Error::WithdrawalHash))
        .collect::<Result<Vec<_>, _>>()
}

fn is_withdrawal_l2(tx: &Transaction, receipt: &Receipt) -> bool {
    // WithdrawalInitiated(address,address,uint256)
    let withdrawal_event_selector: H256 =
        H256::from_str("bb2689ff876f7ef453cf8865dde5ab10349d222e2e1383c5152fbdb083f02da2").unwrap();

    match tx.to() {
        TxKind::Call(to) if to == COMMON_BRIDGE_L2_ADDRESS => receipt.logs.iter().any(|log| {
            log.topics
                .iter()
                .any(|topic| *topic == withdrawal_event_selector)
        }),
        _ => false,
    }
}

pub fn get_withdrawals_merkle_root(withdrawals_hashes: Vec<H256>) -> Result<H256, Error> {
    if !withdrawals_hashes.is_empty() {
        merkelize(withdrawals_hashes)
    } else {
        Ok(H256::zero())
    }
}

pub fn get_withdrawal_hash(tx: &Transaction) -> Option<H256> {
    let to_bytes: [u8; 20] = match tx.data().get(16..36)?.try_into() {
        Ok(value) => value,
        Err(_) => return None,
    };
    let to = Address::from(to_bytes);

    let value = tx.value().to_big_endian();

    Some(keccak_hash::keccak(
        [to.as_bytes(), &value, tx.compute_hash().as_bytes()].concat(),
    ))
}

pub fn merkelize(data: Vec<H256>) -> Result<H256, Error> {
    let mut data = data;
    let mut first = true;
    while data.len() > 1 || first {
        first = false;
        data = data
            .chunks(2)
            .flat_map(|chunk| -> Result<H256, Error> {
                let left = chunk.first().ok_or(Error::FailedToMerkelize)?;
                let right = *chunk.get(1).unwrap_or(left);
                Ok(keccak([left.as_bytes(), right.as_bytes()].concat())
                    .as_fixed_bytes()
                    .into())
            })
            .collect();
    }
    data.first().copied().ok_or(Error::FailedToMerkelize)
}
