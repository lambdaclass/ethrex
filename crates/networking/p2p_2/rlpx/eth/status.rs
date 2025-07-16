use super::eth68::status::StatusMessage68;
use super::eth69::status::StatusMessage69;
use crate::rlpx::message::RLPxMessage;
use crate::rlpx::utils::snappy_decompress;
use crate::rlpx::{error::RLPxError, p2p::Capability};
use bytes::BufMut;
use ethrex_common::U256;
use ethrex_common::types::{BlockHash, ForkId};
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use ethrex_rlp::structs::Decoder;
use ethrex_storage::Store;

#[derive(Debug, Clone)]
pub enum StatusMessage {
    StatusMessage68(StatusMessage68),
    StatusMessage69(StatusMessage69),
}

impl RLPxMessage for StatusMessage {
    const CODE: u8 = 0x00;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        match self {
            StatusMessage::StatusMessage68(msg) => msg.encode(buf),
            StatusMessage::StatusMessage69(msg) => msg.encode(buf),
        }
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (eth_version, _): (u32, _) = decoder.decode_field("protocolVersion")?;

        match eth_version {
            68 => Ok(StatusMessage::StatusMessage68(StatusMessage68::decode(
                msg_data,
            )?)),
            69 => Ok(StatusMessage::StatusMessage69(StatusMessage69::decode(
                msg_data,
            )?)),
            _ => Err(RLPDecodeError::IncompatibleProtocol),
        }
    }
}

impl StatusMessage {
    pub async fn new(storage: &Store, eth: &Capability) -> Result<Self, RLPxError> {
        let chain_config = storage.get_chain_config()?;
        let total_difficulty =
            U256::from(chain_config.terminal_total_difficulty.unwrap_or_default());
        let network_id = chain_config.chain_id;

        // These blocks must always be available
        let genesis_header = storage
            .get_block_header(0)?
            .ok_or(RLPxError::NotFound("Genesis Block".to_string()))?;
        let lastest_block = storage.get_latest_block_number().await?;
        let block_header = storage
            .get_block_header(lastest_block)?
            .ok_or(RLPxError::NotFound(format!("Block {lastest_block}")))?;

        let genesis = genesis_header.hash();
        let lastest_block_hash = block_header.hash();
        let fork_id = ForkId::new(
            chain_config,
            genesis_header,
            block_header.timestamp,
            lastest_block,
        );

        match eth.version {
            68 => Ok(StatusMessage::StatusMessage68(StatusMessage68 {
                eth_version: eth.version,
                network_id,
                total_difficulty,
                block_hash: lastest_block_hash,
                genesis,
                fork_id,
            })),
            69 => Ok(StatusMessage::StatusMessage69(StatusMessage69 {
                eth_version: eth.version,
                network_id,
                genesis,
                fork_id,
                earliest_block: 0,
                lastest_block,
                lastest_block_hash,
            })),
            _ => Err(RLPxError::IncompatibleProtocol),
        }
    }

    pub fn get_network_id(&self) -> u64 {
        match self {
            StatusMessage::StatusMessage68(msg) => msg.network_id,
            StatusMessage::StatusMessage69(msg) => msg.network_id,
        }
    }

    pub fn get_eth_version(&self) -> u8 {
        match self {
            StatusMessage::StatusMessage68(msg) => msg.eth_version,
            StatusMessage::StatusMessage69(msg) => msg.eth_version,
        }
    }

    pub fn get_fork_id(&self) -> ForkId {
        match self {
            StatusMessage::StatusMessage68(msg) => msg.fork_id.clone(),
            StatusMessage::StatusMessage69(msg) => msg.fork_id.clone(),
        }
    }

    pub fn get_genesis(&self) -> BlockHash {
        match self {
            StatusMessage::StatusMessage68(msg) => msg.genesis,
            StatusMessage::StatusMessage69(msg) => msg.genesis,
        }
    }
}
