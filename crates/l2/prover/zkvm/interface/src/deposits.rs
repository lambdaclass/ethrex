// This module was based on the L1 committer.
// TODO: We should move this to some kind of "common" library for the L2, but the zkvm programs
// can't depend on ethrex-l2 because of incompatible dependencies.

use ethrex_common::{
    types::{PrivilegedL2Transaction, Transaction},
    Address, U256,
};
use keccak_hash::{keccak, H256};

#[derive(Debug, thiserror::Error)]
pub enum DepositError {
    #[error("Failed to decode deposit hash")]
    FailedToDecodeHash,
    #[error("Length does not fit in u16")]
    LengthTooLarge(#[from] std::num::TryFromIntError),
}

#[derive(Clone)]
pub struct DepositLog {
    pub address: Address,
    pub amount: U256,
    pub nonce: u64,
}

pub fn get_block_deposits(txs: &[Transaction]) -> Vec<PrivilegedL2Transaction> {
    txs.iter()
        .filter_map(|tx| match tx {
            Transaction::PrivilegedL2Transaction(tx) => Some(tx.clone()),
            _ => None,
        })
        .collect()
}

pub fn get_deposit_hash(deposit_hashes: Vec<H256>) -> Result<H256, DepositError> {
    if !deposit_hashes.is_empty() {
        let deposit_hashes_len: u16 = deposit_hashes
            .len()
            .try_into()
            .map_err(DepositError::from)?;
        Ok(H256::from_slice(
            [
                &deposit_hashes_len.to_be_bytes(),
                keccak(
                    deposit_hashes
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
    } else {
        Ok(H256::zero())
    }
}
