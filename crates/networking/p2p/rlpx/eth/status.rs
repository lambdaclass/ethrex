use std::fmt::Debug;

use crate::rlpx::utils::{snappy_compress, snappy_decompress};
use crate::rlpx::{error::RLPxError, p2p::Capability};
use bytes::BufMut;
use ethrex_common::U256;
use ethrex_common::types::{BlockHash, ForkId};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use ethrex_storage::Store;

pub const CODE: u8 = 0x00;

pub trait StatusMessage: Debug {
    fn eth_version(&self) -> u8;
    fn network_id(&self) -> u64;
    fn genesis(&self) -> BlockHash;
    fn fork_id(&self) -> ForkId;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError>;
    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError>
    where
        Self: Sized;
}

#[derive(Debug)]
pub struct Status68Message {
    pub(crate) eth_version: u8,
    pub(crate) network_id: u64,
    pub(crate) total_difficulty: U256,
    pub(crate) genesis: BlockHash,
    pub(crate) fork_id: ForkId,
    pub(crate) latest_block_hash: BlockHash,
}

#[derive(Debug)]
pub struct Status69Message {
    pub(crate) eth_version: u8,
    pub(crate) network_id: u64,
    pub(crate) genesis: BlockHash,
    pub(crate) fork_id: ForkId,
    pub(crate) earliest_block: u64,
    pub(crate) latest_block: u64,
    pub(crate) latest_block_hash: BlockHash,
}

impl Status68Message {
    pub async fn new(storage: &Store, eth: &Capability) -> Result<Self, RLPxError> {
        let chain_config = storage.get_chain_config()?;
        let total_difficulty =
            U256::from(chain_config.terminal_total_difficulty.unwrap_or_default());
        let network_id = chain_config.chain_id;

        // These blocks must always be available
        let genesis_header = storage
            .get_block_header(0)?
            .ok_or(RLPxError::NotFound("Genesis Block".to_string()))?;
        let latest_block = storage.get_latest_block_number().await?;
        let block_header = storage
            .get_block_header(latest_block)?
            .ok_or(RLPxError::NotFound(format!("Block {latest_block}")))?;

        let genesis = genesis_header.hash();
        let latest_block_hash = block_header.hash();
        let fork_id = ForkId::new(
            chain_config,
            genesis_header,
            block_header.timestamp,
            latest_block,
        );

        Ok(Self {
            eth_version: eth.version,
            network_id,
            total_difficulty,
            genesis,
            fork_id,
            latest_block_hash,
        })
    }
}

impl Status69Message {
    pub async fn new(storage: &Store, eth: &Capability) -> Result<Self, RLPxError> {
        let chain_config = storage.get_chain_config()?;
        let network_id = chain_config.chain_id;

        // These blocks must always be available
        let genesis_header = storage
            .get_block_header(0)?
            .ok_or(RLPxError::NotFound("Genesis Block".to_string()))?;
        let latest_block = storage.get_latest_block_number().await?;
        let block_header = storage
            .get_block_header(latest_block)?
            .ok_or(RLPxError::NotFound(format!("Block {latest_block}")))?;

        let genesis = genesis_header.hash();
        let latest_block_hash = block_header.hash();
        let fork_id = ForkId::new(
            chain_config,
            genesis_header,
            block_header.timestamp,
            latest_block,
        );

        Ok(Self {
            eth_version: eth.version,
            network_id,
            genesis,
            fork_id,
            earliest_block: 0,
            latest_block,
            latest_block_hash,
        })
    }
}

impl StatusMessage for Status68Message {
    fn eth_version(&self) -> u8 {
        self.eth_version
    }
    fn network_id(&self) -> u64 {
        self.network_id
    }
    fn genesis(&self) -> BlockHash {
        self.genesis
    }
    fn fork_id(&self) -> ForkId {
        self.fork_id.clone()
    }

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.eth_version)
            .encode_field(&self.network_id)
            .encode_field(&self.total_difficulty)
            .encode_field(&self.latest_block_hash)
            .encode_field(&self.genesis)
            .encode_field(&self.fork_id)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (eth_version, decoder): (u32, _) = decoder.decode_field("protocolVersion")?;

        if eth_version != 68 {
            return Err(RLPDecodeError::MalformedData);
        }

        let (network_id, decoder): (u64, _) = decoder.decode_field("networkId")?;
        let (total_difficulty, decoder): (U256, _) = decoder.decode_field("totalDifficulty")?;
        let (latest_block_hash, decoder): (BlockHash, _) = decoder.decode_field("blockHash")?;
        let (genesis, decoder): (BlockHash, _) = decoder.decode_field("genesis")?;
        let (fork_id, decoder): (ForkId, _) = decoder.decode_field("forkId")?;

        // Implementations must ignore any additional list elements
        let _padding = decoder.finish_unchecked();

        Ok(Self {
            eth_version: eth_version as u8,
            network_id,
            total_difficulty,
            genesis,
            fork_id,
            latest_block_hash,
        })
    }
}

impl StatusMessage for Status69Message {
    fn eth_version(&self) -> u8 {
        self.eth_version
    }
    fn network_id(&self) -> u64 {
        self.network_id
    }
    fn genesis(&self) -> BlockHash {
        self.genesis
    }
    fn fork_id(&self) -> ForkId {
        self.fork_id.clone()
    }

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.eth_version)
            .encode_field(&self.network_id)
            .encode_field(&self.genesis)
            .encode_field(&self.fork_id)
            .encode_field(&self.earliest_block)
            .encode_field(&self.latest_block)
            .encode_field(&self.latest_block_hash)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (eth_version, decoder): (u32, _) = decoder.decode_field("protocolVersion")?;

        if eth_version != 69 {
            return Err(RLPDecodeError::MalformedData);
        }

        let (network_id, decoder): (u64, _) = decoder.decode_field("networkId")?;
        let (genesis, decoder): (BlockHash, _) = decoder.decode_field("genesis")?;
        let (fork_id, decoder): (ForkId, _) = decoder.decode_field("forkId")?;
        let (earliest_block, decoder): (u64, _) = decoder.decode_field("earliestBlock")?;
        let (latest_block, decoder): (u64, _) = decoder.decode_field("lastestBlock")?;
        let (latest_block_hash, decoder): (BlockHash, _) = decoder.decode_field("latestHash")?;
        // Implementations must ignore any additional list elements
        let _padding = decoder.finish_unchecked();

        Ok(Self {
            eth_version: eth_version as u8,
            network_id,
            genesis,
            fork_id,
            earliest_block,
            latest_block,
            latest_block_hash,
        })
    }
}
