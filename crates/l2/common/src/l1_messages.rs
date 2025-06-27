use std::sync::LazyLock;

use ethereum_types::{Address, H256};
use ethrex_common::{
    H160,
    types::{Receipt, Transaction},
};
use keccak_hash::keccak;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const L1MESSENGER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Represents a message from the L2 to the L1
pub struct L1Message {
    /// L2 Transaction the message was included in, for ease of usage
    pub tx_hash: H256,
    /// Address that called the L1Messanger
    pub from: Address,
    /// Hash of the data given to the L1Messenger
    pub data_hash: H256,
}

impl L1Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.tx_hash.0);
        bytes.extend_from_slice(&self.from.to_fixed_bytes());
        bytes.extend_from_slice(&self.data_hash.0);
        bytes
    }
}

#[derive(Debug, Error)]
pub enum L1MessagingError {
    #[error("Failed to merkelize messages")]
    FailedToMerkelize,
}

pub fn get_l1_message_hash(msg: &L1Message) -> H256 {
    keccak(msg.encode())
}

pub fn get_block_l1_message_hashes(
    txs: &[Transaction],
    receipts: &[Receipt],
) -> Result<Vec<H256>, L1MessagingError> {
    Ok(get_block_l1_messages(txs, receipts)
        .iter()
        .map(get_l1_message_hash)
        .collect())
}

pub fn get_block_l1_messages(txs: &[Transaction], receipts: &[Receipt]) -> Vec<L1Message> {
    static L1MESSAGE_EVENT_SELECTOR: LazyLock<H256> =
        LazyLock::new(|| keccak("L1Message(address,bytes32)".as_bytes()));

    receipts
        .iter()
        .zip(txs.iter())
        .flat_map(|(receipt, tx)| {
            receipt
                .logs
                .iter()
                .filter(|log| {
                    log.address == L1MESSENGER_ADDRESS
                        && log.topics.contains(&L1MESSAGE_EVENT_SELECTOR)
                })
                .flat_map(|log| -> Option<L1Message> {
                    Some(L1Message {
                        from: Address::from_slice(&log.topics.get(1)?.0[12..32]),
                        data_hash: *log.topics.get(2)?,
                        tx_hash: tx.compute_hash(),
                    })
                })
        })
        .collect()
}

pub fn compute_merkle_root(hashes: &[H256]) -> Result<H256, L1MessagingError> {
    if !hashes.is_empty() {
        merkelize(hashes)
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
