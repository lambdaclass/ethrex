use std::sync::LazyLock;

use ethereum_types::{Address, H256};
use ethrex_common::{H160, U256, types::Receipt};
use keccak_hash::keccak;

use serde::{Deserialize, Serialize};

pub const L1MESSENGER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Represents a message from the L2 to the L1
pub struct L1Message {
    /// Address that called the L1Messanger
    pub from: Address,
    /// Hash of the data given to the L1Messenger
    pub data_hash: H256,
    /// Message id emitted by the bridge contract
    pub message_id: U256,
}

impl L1Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.from.to_fixed_bytes());
        bytes.extend_from_slice(&self.data_hash.0);
        bytes.extend_from_slice(&self.message_id.to_big_endian());
        bytes
    }
}

pub fn get_l1_message_hash(msg: &L1Message) -> H256 {
    keccak(msg.encode())
}

pub fn get_block_l1_message_hashes(receipts: &[Receipt]) -> Vec<H256> {
    get_block_l1_messages(receipts)
        .iter()
        .map(get_l1_message_hash)
        .collect()
}

pub fn get_block_l1_messages(receipts: &[Receipt]) -> Vec<L1Message> {
    static L1MESSAGE_EVENT_SELECTOR: LazyLock<H256> =
        LazyLock::new(|| keccak("L1Message(address,bytes32,uint256)".as_bytes()));

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
                .flat_map(|log| -> Option<L1Message> {
                    Some(L1Message {
                        from: Address::from_slice(&log.topics.get(1)?.0[12..32]),
                        data_hash: *log.topics.get(2)?,
                        message_id: U256::from_big_endian(&log.topics.get(3)?.to_fixed_bytes()),
                    })
                })
        })
        .collect()
}
