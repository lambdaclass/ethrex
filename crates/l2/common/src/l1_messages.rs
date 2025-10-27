use bytes::Bytes;
use ethereum_types::{Address, H256};
use ethrex_common::types::l2_to_l2_message::L2toL2Message;
use ethrex_common::utils::keccak;
use ethrex_common::{H160, U256, types::Receipt};

use serde::{Deserialize, Serialize};

use crate::calldata::Value;

pub const L1MESSENGER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

// keccak256("L1Message(address,bytes32,uint256)")
static L1MESSAGE_EVENT_SELECTOR: H256 = H256([
    0x18, 0xd7, 0xb7, 0x05, 0x34, 0x4d, 0x61, 0x6d, 0x1b, 0x61, 0xda, 0xa6, 0xa8, 0xcc, 0xfc, 0xf9,
    0xf1, 0x0c, 0x27, 0xad, 0xe0, 0x07, 0xcc, 0x45, 0xcf, 0x87, 0x0d, 0x1e, 0x12, 0x1f, 0x1a, 0x9d,
]);
// keccak256("L2ToL2Message(uint256,address,address,uint256,uint256,bytes)")
static L2_MESSAGE_SELECTOR: H256 = H256([
    0x09, 0xdb, 0x04, 0xf0, 0x10, 0xf1, 0x0e, 0xf2, 0x0f, 0xce, 0xf9, 0xca, 0xe9, 0xf6, 0x4a, 0xbb,
    0xde, 0x92, 0xfe, 0xe1, 0x2c, 0x68, 0xf6, 0x92, 0xc2, 0x3a, 0x72, 0xcc, 0x54, 0xb2, 0x96, 0x9e,
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

pub fn get_l2_to_l2_messages(receipts: &[Receipt]) -> Vec<L2toL2Message> {
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

fn l2_message_from_log_data(log_data: &[u8]) -> Option<L2toL2Message> {
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

    Some(L2toL2Message {
        chain_id,
        from,
        to,
        value,
        gas_limit,
        data,
    })
}

pub fn value_from_l2_to_l2_message(msg: &L2toL2Message) -> Value {
    Value::Tuple(vec![
        Value::Uint(msg.chain_id),
        Value::Address(msg.to),
        Value::Uint(msg.value),
        Value::Uint(msg.gas_limit),
        Value::Bytes(msg.data.clone()),
    ])
}
