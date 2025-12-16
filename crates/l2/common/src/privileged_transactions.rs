use ethereum_types::{Address, H256, U256};
use ethrex_common::types::{PrivilegedL2Transaction, Transaction};
use ethrex_common::utils::keccak;
use serde::{Deserialize, Serialize};

/// Max privileged tx to allow per batch
pub const PRIVILEGED_TX_BUDGET: u64 = 300;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrivilegedTransactionLog {
    pub address: Address,
    pub amount: U256,
    pub nonce: u64,
}

impl PrivilegedTransactionLog {
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = Vec::new();
        encoded.extend(self.address.0);
        encoded.extend_from_slice(&self.amount.to_big_endian());
        encoded
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrivilegedTransactionError {
    #[error("Failed to decode transaction hash")]
    FailedToDecodeHash,
    #[error("Length does not fit in u16")]
    LengthTooLarge(#[from] std::num::TryFromIntError),
}

pub fn get_block_l1_in_messages(
    txs: &[Transaction],
    chain_id: u64,
) -> Vec<PrivilegedL2Transaction> {
    txs.iter()
        .filter_map(|tx| {
            if let Transaction::PrivilegedL2Transaction(tx) = tx {
                // The chain id of the L1-in message should match the chain id of the current chain
                if tx.chain_id == chain_id {
                    return Some(tx.clone());
                }
                None
            } else {
                None
            }
        })
        .collect()
}

pub fn get_block_l2_in_messages(
    txs: &[Transaction],
    chain_id: u64,
) -> Vec<PrivilegedL2Transaction> {
    txs.iter()
        .filter_map(|tx| {
            if let Transaction::PrivilegedL2Transaction(tx) = tx {
                // The chain id of the L2-in message is the chain id of the source chain
                if tx.chain_id != chain_id {
                    return Some(tx.clone());
                }
                None
            } else {
                None
            }
        })
        .collect()
}

pub fn compute_privileged_transactions_hash(
    privileged_transaction_hashes: Vec<H256>,
) -> Result<H256, PrivilegedTransactionError> {
    if privileged_transaction_hashes.is_empty() {
        return Ok(H256::zero());
    }

    let privileged_transaction_hashes_len: u16 = privileged_transaction_hashes.len().try_into()?;

    Ok(H256::from_slice(
        [
            &privileged_transaction_hashes_len.to_be_bytes(),
            keccak(
                privileged_transaction_hashes
                    .iter()
                    .map(H256::as_bytes)
                    .collect::<Vec<&[u8]>>()
                    .concat(),
            )
            .as_bytes()
            .get(2..32)
            .ok_or(PrivilegedTransactionError::FailedToDecodeHash)?,
        ]
        .concat()
        .as_slice(),
    ))
}
