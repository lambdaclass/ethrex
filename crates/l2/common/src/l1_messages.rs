use std::sync::LazyLock;

use bytes::Bytes;
use ethereum_types::{Address, H256};
use ethrex_common::utils::keccak;
use ethrex_common::{H160, U256, types::Receipt};

use serde::{Deserialize, Serialize};

use crate::calldata::Value;

pub const L1MESSENGER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

#[derive(Serialize, Deserialize, Debug)]
pub struct L1MessageProof {
    pub batch_number: u64,
    pub message_id: U256,
    pub message_hash: H256,
    pub merkle_proof: Vec<H256>,
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Represents a message from the L2 to the L1
pub struct L2Message {
    pub chain_id: U256,
    /// Address that called the L1Messanger
    pub from: Address,
    pub to: Address,
    pub value: U256,
    pub data: Bytes,
    /// Message id emitted by the bridge contract
    pub message_id: U256,
}

impl From<L2Message> for Value {
    fn from(msg: L2Message) -> Self {
        Value::Tuple(vec![
            Value::Uint(msg.chain_id),
            Value::Address(msg.to),
            Value::Uint(msg.value),
            Value::Uint(U256::from(10000000)),
            Value::Bytes(msg.data),
        ])
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

pub fn get_l2_to_l2_messages(receipts: &[Receipt]) -> Vec<L2Message> {
    static L2_MESSAGE_SELECTOR: LazyLock<H256> = LazyLock::new(|| {
        keccak("L2ToL2Message(uint256,address,address,uint256,bytes,uint256)".as_bytes())
    });

    receipts
        .iter()
        .flat_map(|receipt| {
            receipt
                .logs
                .iter()
                .filter(|log| {
                    log.address == L1MESSENGER_ADDRESS && log.topics.contains(&L2_MESSAGE_SELECTOR)
                })
                .flat_map(|log| l2_message_from_log_data(&log.data))
        })
        .collect()
}

fn l2_message_from_log_data(log_data: &[u8]) -> Option<L2Message> {
    let chain_id = U256::from_big_endian(log_data.get(..32)?);
    let from = Address::from_slice(log_data.get(44..64)?);
    let to = Address::from_slice(log_data.get(76..96)?);
    let value = U256::from_big_endian(log_data.get(96..128)?);
    let message_id = U256::from_big_endian(log_data.get(160..192)?);
    let data_len: usize = U256::from_big_endian(log_data.get(192..224)?).as_usize();
    let data = Bytes::copy_from_slice(log_data.get(224..224 + data_len)?);

    Some(L2Message {
        chain_id,
        from,
        to,
        value,
        data,
        message_id,
    })
}
