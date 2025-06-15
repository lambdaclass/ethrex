use std::sync::LazyLock;

use ethereum_types::{Address, H256};
use ethrex_common::{
    H160, U256,
    types::{Receipt, Transaction, TxKind},
};
use keccak_hash::keccak;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
]);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WithdrawalLog {
    pub address: Address,
    pub amount: U256,
    pub tx_hash: H256,
}

impl WithdrawalLog {
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = Vec::new();
        encoded.extend(self.address.0);
        encoded.extend_from_slice(&self.amount.to_big_endian());
        encoded.extend(&self.tx_hash.0);
        encoded
    }
}

#[derive(Debug, Error)]
pub enum WithdrawalError {
    #[error("Withdrawal transaction was invalid")]
    InvalidWithdrawalTransaction,
    #[error("Failed to merkelize withdrawals")]
    FailedToMerkelize,
    #[error("Failed to create withdrawal selector")]
    WithdrawalSelector,
    #[error("Failed to get withdrawal hash")]
    WithdrawalHash,
}

pub fn get_block_withdrawal_hashes(
    txs: &[Transaction],
    receipts: &[Receipt],
) -> Result<Vec<H256>, WithdrawalError> {
    txs.iter()
        .zip(receipts.iter())
        .filter(|(tx, receipt)| is_withdrawal_l2(tx, receipt))
        .map(|(withdrawal, _)| get_withdrawal_hash(withdrawal))
        .collect::<Result<Vec<_>, _>>()
}

pub fn get_block_withdrawals(txs: &[Transaction], receipts: &[Receipt]) -> Vec<Transaction> {
    txs.iter()
        .zip(receipts.iter())
        .filter_map(|(tx, receipt)| {
            if is_withdrawal_l2(tx, receipt) {
                Some(tx.clone())
            } else {
                None
            }
        })
        .collect()
}

fn is_withdrawal_l2(tx: &Transaction, receipt: &Receipt) -> bool {
    static WITHDRAWAL_EVENT_SELECTOR: LazyLock<H256> =
        LazyLock::new(|| keccak("WithdrawalInitiated(address,address,uint256)".as_bytes()));

    match tx.to() {
        TxKind::Call(to) if to == COMMON_BRIDGE_L2_ADDRESS => receipt
            .logs
            .iter()
            .any(|log| log.topics.contains(&WITHDRAWAL_EVENT_SELECTOR)),
        _ => false,
    }
}

pub fn get_withdrawal_hash(tx: &Transaction) -> Result<H256, WithdrawalError> {
    let to_bytes: [u8; 20] = match tx.data().get(16..36).map(TryInto::try_into) {
        Some(Ok(value)) => value,
        _ => return Err(WithdrawalError::WithdrawalHash),
    };
    let to = Address::from(to_bytes);

    let value = tx.value().to_big_endian();

    Ok(keccak_hash::keccak(
        [
            to.as_bytes(),
            &value,
            tx.compute_hash()
                .map_err(|_| WithdrawalError::WithdrawalHash)?
                .as_bytes(),
        ]
        .concat(),
    ))
}

pub fn compute_withdrawals_merkle_root(
    withdrawals_hashes: &[H256],
) -> Result<H256, WithdrawalError> {
    if !withdrawals_hashes.is_empty() {
        merkelize(withdrawals_hashes)
    } else {
        Ok(H256::zero())
    }
}

pub fn merkelize(data: &[H256]) -> Result<H256, WithdrawalError> {
    let mut data = data.to_vec();
    let mut first = true;
    while data.len() > 1 || first {
        first = false;
        data = data
            .chunks(2)
            .flat_map(|chunk| -> Result<H256, WithdrawalError> {
                let left = chunk.first().ok_or(WithdrawalError::FailedToMerkelize)?;
                let right = *chunk.get(1).unwrap_or(left);
                Ok(keccak([left.as_bytes(), right.as_bytes()].concat())
                    .as_fixed_bytes()
                    .into())
            })
            .collect();
    }
    data.first()
        .copied()
        .ok_or(WithdrawalError::FailedToMerkelize)
}
