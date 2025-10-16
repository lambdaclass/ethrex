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
/// Represents a message from the L2 to another L2
pub struct L2Message {
    /// Chain id of the destination chain
    pub chain_id: U256,
    /// Address that originated the transaction
    pub from: Address,
    /// Address of the recipient in the destination chain
    pub to: Address,
    /// Amount of ETH to send to the recipient
    pub value: U256,
    /// Gas limit for the transaction execution in the destination chain
    pub gas_limit: U256,
    /// Calldata for the transaction in the destination chain
    pub data: Bytes,
}

impl From<L2Message> for Value {
    fn from(msg: L2Message) -> Self {
        Value::Tuple(vec![
            Value::Uint(msg.chain_id),
            Value::Address(msg.to),
            Value::Uint(msg.value),
            Value::Uint(msg.gas_limit),
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
        keccak("L2ToL2Message(uint256,address,address,uint256,uint256,bytes,uint256)".as_bytes())
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
    let mut offset = 0;

    let chain_id = U256::from_big_endian(log_data.get(offset..offset + 32)?);
    offset += 32;

    let from = Address::from_slice(log_data.get(offset + 12..offset + 32)?);
    offset += 32;

    let to = Address::from_slice(log_data.get(offset + 12..offset + 32)?);
    offset += 32;

    let value = U256::from_big_endian(log_data.get(offset..offset + 32)?);
    offset += 32;

    let gas_limit = U256::from_big_endian(log_data.get(offset..offset + 32)?);
    offset += 64; // 32 from gas_limit + 32 from data offset

    let data_len: usize = U256::from_big_endian(log_data.get(offset..offset + 32)?).as_usize();
    let data = Bytes::copy_from_slice(log_data.get(offset + 32..offset + 32 + data_len)?);

    Some(L2Message {
        chain_id,
        from,
        to,
        value,
        gas_limit,
        data,
    })
}
