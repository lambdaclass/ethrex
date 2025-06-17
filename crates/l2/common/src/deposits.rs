use ethereum_types::{Address, H256, U256};
use ethrex_common::types::{PrivilegedL2Transaction, Transaction};
use keccak_hash::keccak;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DepositLog {
    pub address: Address,
    pub amount: U256,
    pub nonce: u64,
}

impl DepositLog {
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = Vec::new();
        encoded.extend(self.address.0);
        encoded.extend_from_slice(&self.amount.to_big_endian());
        encoded
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DepositError {
    #[error("Failed to decode deposit hash")]
    FailedToDecodeHash,
    #[error("Length does not fit in u16")]
    LengthTooLarge(#[from] std::num::TryFromIntError),
}

pub fn get_block_deposits(txs: &[Transaction]) -> Vec<PrivilegedL2Transaction> {
    txs.iter()
        .filter_map(|tx| match tx {
            Transaction::PrivilegedL2Transaction(tx) => Some(tx.clone()),
            _ => None,
        })
        .collect()
}

pub fn compute_deposit_logs_hash(deposit_log_hashes: Vec<H256>) -> Result<H256, DepositError> {
    if deposit_log_hashes.is_empty() {
        return Ok(H256::zero());
    }

    let deposit_hashes_len: u16 = deposit_log_hashes.len().try_into()?;

    Ok(H256::from_slice(
        [
            &deposit_hashes_len.to_be_bytes(),
            keccak(
                deposit_log_hashes
                    .iter()
                    .map(H256::as_bytes)
                    .collect::<Vec<&[u8]>>()
                    .concat(),
            )
            .as_bytes()
            .get(2..32)
            .ok_or(DepositError::FailedToDecodeHash)?,
        ]
        .concat()
        .as_slice(),
    ))
}
