use std::sync::LazyLock;

use ethereum_types::{Address, H256};
use ethrex_common::{H160, types::Receipt};
use keccak_hash::keccak;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const L1MESSENGER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct L1Message {
    pub from: Address,
    pub data: H256,
}

impl L1Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.from.to_fixed_bytes());
        bytes.extend_from_slice(&self.data.0);
        bytes
    }
}

#[derive(Debug, Error)]
pub enum L1MessagingError {
    #[error("Withdrawal transaction was invalid")]
    InvalidWithdrawalTransaction,
    #[error("Failed to merkelize withdrawals")]
    FailedToMerkelize,
    #[error("Failed to create withdrawal selector")]
    WithdrawalSelector,
    #[error("Failed to get withdrawal hash")]
    WithdrawalHash,
    #[error("Failed to encode L1 message")]
    L1MessageEncode,
}

pub fn get_l1message_hash(msg: &L1Message) -> H256 {
    keccak(msg.encode())
}

pub fn get_block_message_hashes(receipts: &[Receipt]) -> Result<Vec<H256>, L1MessagingError> {
    Ok(get_block_messages(receipts)
        .iter()
        .map(get_l1message_hash)
        .collect())
}

pub fn get_block_messages(receipts: &[Receipt]) -> Vec<L1Message> {
    static L1MESSAGE_EVENT_SELECTOR: LazyLock<H256> =
        LazyLock::new(|| keccak("L1Message(address,bytes)".as_bytes()));

    receipts
        .iter()
        .flat_map(|receipt| {
            receipt
                .logs
                .iter()
                .filter(|log| {
                    log.address == L1MESSENGER_ADDRESS
                        && log.topics.contains(&L1MESSAGE_EVENT_SELECTOR)
                })
                .map(|log| L1Message {
                    from: Address::from_slice(&log.data.slice(0..20)),
                    data: H256::from_slice(&log.data.slice(20..52)),
                })
        })
        .collect()
}

pub fn compute_merkle_root(withdrawals_hashes: &[H256]) -> Result<H256, L1MessagingError> {
    if !withdrawals_hashes.is_empty() {
        merkelize(withdrawals_hashes)
    } else {
        Ok(H256::zero())
    }
}

pub fn merkelize(data: &[H256]) -> Result<H256, L1MessagingError> {
    let mut data = data.to_vec();
    let mut first = true;
    while data.len() > 1 || first {
        first = false;
        data = data
            .chunks(2)
            .flat_map(|chunk| -> Result<H256, L1MessagingError> {
                let left = chunk.first().ok_or(L1MessagingError::FailedToMerkelize)?;
                let right = *chunk.get(1).unwrap_or(left);
                Ok(keccak([left.as_bytes(), right.as_bytes()].concat())
                    .as_fixed_bytes()
                    .into())
            })
            .collect();
    }
    data.first()
        .copied()
        .ok_or(L1MessagingError::FailedToMerkelize)
}
