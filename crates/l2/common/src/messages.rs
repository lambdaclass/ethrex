use std::collections::BTreeMap;
use std::sync::LazyLock;

use bytes::Bytes;
use ethereum_types::{Address, H256};
use ethrex_common::types::balance_diff::{AssetDiff, BalanceDiff};
use ethrex_common::utils::keccak;
use ethrex_common::{H160, U256, types::Receipt};

use serde::{Deserialize, Serialize};
use tracing::warn;
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

// crosschainMintERC20(address,address,address,address,uint256)
pub static CROSSCHAIN_MINT_ERC20_SELECTOR: [u8; 4] = [0xf0, 0x26, 0x31, 0x95];

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Represents a message from the L2 to another L2
pub struct L2Message {
    /// Chain id of the destination chain
    pub dest_chain_id: U256,
    /// Chain id of the source chain
    pub source_chain_id: u64,
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
        [
            U256::from(self.source_chain_id).to_big_endian().as_ref(),
            self.from.as_bytes(),
            self.to.as_bytes(),
            &self.tx_id.to_big_endian(),
            &self.value.to_big_endian(),
            &self.gas_limit.to_big_endian(),
            keccak(&self.data).as_bytes(),
        ]
        .concat()
    }
    pub fn from_log(log: &ethrex_common::types::Log, source_chain_id: u64) -> Option<L2Message> {
        let dest_chain_id = U256::from_big_endian(&log.topics.get(1)?.0);
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
            dest_chain_id,
            source_chain_id,
            from,
            to,
            value,
            gas_limit,
            tx_id,
            data: Bytes::copy_from_slice(calldata),
        })
    }
}

pub fn get_block_l2_out_messages(receipts: &[Receipt], source_chain_id: u64) -> Vec<L2Message> {
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
                .filter_map(|log| L2Message::from_log(log, source_chain_id))
        })
        .collect()
}

pub fn get_balance_diffs(messages: &[L2Message]) -> Vec<BalanceDiff> {
    let mut balance_diffs: BTreeMap<U256, BalanceDiff> = BTreeMap::new();
    for message in messages {
        let mut offset = 4;
        let (value, value_per_token_decoded) = if let Some(selector) = message.data.get(..4)
            && *selector == CROSSCHAIN_MINT_ERC20_SELECTOR
        {
            let Some(token_l1) = message.data.get(offset + 12..offset + 32) else {
                warn!("Failed to decode token_l1 from crosschainMintERC20 message");
                continue;
            };
            offset += 32;
            let Some(token_src_l2) = message.data.get(offset + 12..offset + 32) else {
                warn!("Failed to decode token_src_l2 from crosschainMintERC20 message");
                continue;
            };
            offset += 32;
            let Some(token_dst_l2) = message.data.get(offset + 12..offset + 32) else {
                warn!("Failed to decode token_dst_l2 from crosschainMintERC20 message");
                continue;
            };
            offset += 32;
            offset += 32; // skip "to" param
            let Some(value_bytes) = message.data.get(offset..offset + 32) else {
                warn!("Failed to decode value from crosschainMintERC20 message");
                continue;
            };
            (
                U256::zero(),
                Some(AssetDiff {
                    token_l1: Address::from_slice(token_l1),
                    token_src_l2: Address::from_slice(token_src_l2),
                    token_dst_l2: Address::from_slice(token_dst_l2),
                    value: U256::from_big_endian(value_bytes),
                }),
            )
        } else {
            let mut value = message.value;
            if message.to == BRIDGE_ADDRESS && message.from == BRIDGE_ADDRESS {
                // This is the mint transaction, ignore the value
                value = U256::zero();
            }
            (value, None)
        };
        let entry = balance_diffs
            .entry(message.dest_chain_id)
            .or_insert(BalanceDiff {
                chain_id: message.dest_chain_id,
                value: U256::zero(),
                value_per_token: Vec::new(),
                message_hashes: Vec::new(),
            });
        if let Some(value_per_token_decoded) = value_per_token_decoded {
            if let Some(existing) = entry.value_per_token.iter_mut().find(|v| {
                v.token_l1 == value_per_token_decoded.token_l1
                    && v.token_src_l2 == value_per_token_decoded.token_src_l2
                    && v.token_dst_l2 == value_per_token_decoded.token_dst_l2
            }) {
                existing.value += value_per_token_decoded.value;
            } else {
                entry.value_per_token.push(value_per_token_decoded);
            }
        }
        entry.value += value;
        entry.message_hashes.push(get_l2_message_hash(message));
    }
    balance_diffs.into_values().collect()
}
