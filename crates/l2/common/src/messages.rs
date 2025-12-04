use std::collections::BTreeMap;
use std::sync::LazyLock;

use bytes::Bytes;
use ethereum_types::{Address, H256};
use ethrex_common::types::balance_diff::BalanceDiff;
use ethrex_common::utils::keccak;
use ethrex_common::{H160, U256, types::Receipt};

use serde::{Deserialize, Serialize};
pub const MESSENGER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

pub static L1MESSAGE_EVENT_SELECTOR: LazyLock<H256> =
    LazyLock::new(|| keccak("L1Message(address,bytes32,uint256)".as_bytes()));

// keccak256("L2Message(uint256,address,address,uint256,uint256,uint256,bytes)")
pub static L2MESSAGE_EVENT_SELECTOR: LazyLock<H256> = LazyLock::new(|| {
    keccak("L2Message(uint256,address,address,uint256,uint256,uint256,bytes)".as_bytes())
});

pub const BRIDGE_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
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

pub fn get_l2_message_hash(msg: &L2Message) -> H256 {
    keccak(msg.encode())
}

pub fn get_block_l1_messages(receipts: &[Receipt]) -> Vec<L1Message> {
    receipts
        .iter()
        .flat_map(|receipt| {
            receipt
                .logs
                .iter()
                .filter(|log| {
                    log.address == MESSENGER_ADDRESS
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

#[derive(Serialize, Deserialize, Debug)]
pub struct L2MessageProof {
    pub batch_number: u64,
    pub message_hash: H256,
    pub merkle_proof: Vec<H256>,
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
    /// Unique transaction id for the message in the destination chain
    pub tx_id: U256,
    /// Calldata for the transaction in the destination chain
    pub data: Bytes,
}

impl L2Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.chain_id.to_big_endian());
        bytes.extend_from_slice(&self.from.to_fixed_bytes());
        bytes.extend_from_slice(&self.to.to_fixed_bytes());
        bytes.extend_from_slice(&self.value.to_big_endian());
        bytes.extend_from_slice(&self.gas_limit.to_big_endian());
        bytes.extend_from_slice(&self.data);
        bytes
    }
    pub fn from_log(log: &ethrex_common::types::Log) -> Option<L2Message> {
        let chain_id = U256::from_big_endian(&log.topics.get(1)?.0);
        let from = H256::from_slice(log.data.get(0..32)?);
        let from = Address::from_slice(&from.as_fixed_bytes()[12..]);
        let to = H256::from_slice(log.data.get(32..64)?);
        let to = Address::from_slice(&to.as_fixed_bytes()[12..]);
        let value = U256::from_big_endian(log.data.get(64..96)?);
        let gas_limit = U256::from_big_endian(log.data.get(96..128)?);
        let tx_id = U256::from_big_endian(log.data.get(128..160)?);
        // 160 to 192 is the offset for calldata
        let calldata_len = U256::from_big_endian(log.data.get(192..224)?);
        let calldata = log.data.get(224..224 + calldata_len.as_usize())?;

        Some(L2Message {
            chain_id,
            from,
            to,
            value,
            gas_limit,
            tx_id,
            data: Bytes::copy_from_slice(calldata),
        })
    }
}

pub fn get_block_l2_messages(receipts: &[Receipt]) -> Vec<L2Message> {
    receipts
        .iter()
        .flat_map(|receipt| {
            receipt
                .logs
                .iter()
                .filter(|log| {
                    log.address == MESSENGER_ADDRESS
                        && log.topics.first() == Some(&*L2MESSAGE_EVENT_SELECTOR)
                        && log.topics.len() >= 2 // need chainId
                })
                .filter_map(L2Message::from_log)
        })
        .collect()
}

pub fn get_balance_diffs(messages: &[L2Message]) -> Vec<BalanceDiff> {
    let mut balance_diffs: BTreeMap<U256, BalanceDiff> = BTreeMap::new();
    for message in messages {
        if message.to == BRIDGE_ADDRESS && message.from == BRIDGE_ADDRESS {
            continue;
        }
        let entry = balance_diffs
            .entry(message.chain_id)
            .or_insert(BalanceDiff {
                chain_id: message.chain_id,
                value: U256::zero(),
            });
        entry.value += message.value;
    }
    balance_diffs.into_values().collect()
}
