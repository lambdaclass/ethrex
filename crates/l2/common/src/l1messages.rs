use std::sync::LazyLock;

use ethereum_types::{Address, H256};
use ethrex_common::{
    types::{Receipt, Transaction, TxKind},
    H160, U256,
};
use keccak_hash::keccak;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ethrex_l2_sdk::calldata::{Value, encode_tuple};

pub const L1MESSENGER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct L1Message {
    pub from: Address,
    pub data: Bytes,
}

impl L1Message {
    pub fn encode(&self) -> Result<Vec<u8>, L1MessagingError> {
        encode_tuple(&vec![
            Value::Address(self.from),
            Value::Bytes(self.data)
        ])
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

pub fn get_block_message_hashes(
    receipts: &[Receipt],
) -> Result<Vec<H256>, L1MessagingError> {
    get_block_messages(receipts)
        .iter()
        .map(|msg| keccak(msg.encode()))
        .collect()
}

pub fn get_block_messages(receipts: &[Receipt]) -> Vec<L1Message> {
    static L1MESSAGE_EVENT_SELECTOR: LazyLock<H256> =
        LazyLock::new(|| keccak("L1Message(address,bytes)".as_bytes()));

    receipts.iter().flat_map(|receipt| {
        receipt.logs.iter().filter(|log| {
            log.address == L1MESSENGER_ADDRESS &&
            log.topics
                .iter()
                .any(|topic| *topic == *L1MESSAGE_EVENT_SELECTOR)
        }).map(|log| log.data)
    })
}

pub fn compute_merkle_root(
    withdrawals_hashes: &[H256],
) -> Result<H256, L1MessagingError> {
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
