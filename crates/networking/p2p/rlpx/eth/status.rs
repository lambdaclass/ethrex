pub use super::eth68::status::StatusMessage68;
pub use super::eth69::status::StatusMessage69;
pub use super::eth71::status::StatusMessage71;
use crate::rlpx::{
    error::PeerConnectionError,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::types::{BlockHash, ForkId};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use ethrex_storage::Store;

pub trait StatusMessage {
    fn get_network_id(&self) -> u64;

    fn get_eth_version(&self) -> u8;

    fn get_fork_id(&self) -> ForkId;

    fn get_genesis(&self) -> BlockHash;
}

/// Shared status data for eth/69+ protocols (eth/69, eth/71, ...).
/// The wire format is identical; only the version field differs.
#[derive(Debug, Clone)]
pub struct StatusDataPost68 {
    pub eth_version: u8,
    pub network_id: u64,
    pub genesis: BlockHash,
    pub fork_id: ForkId,
    pub earliest_block: u64,
    pub lastest_block: u64,
    pub lastest_block_hash: BlockHash,
}

impl StatusDataPost68 {
    pub async fn new(eth_version: u8, storage: &Store) -> Result<Self, PeerConnectionError> {
        let chain_config = storage.get_chain_config();
        let network_id = chain_config.chain_id;

        let genesis_header = storage
            .get_block_header(0)?
            .ok_or(PeerConnectionError::NotFound("Genesis Block".to_string()))?;
        let lastest_block = storage.get_latest_block_number().await?;
        let block_header =
            storage
                .get_block_header(lastest_block)?
                .ok_or(PeerConnectionError::NotFound(format!(
                    "Block {lastest_block}"
                )))?;

        let genesis = genesis_header.hash();
        let lastest_block_hash = block_header.hash();
        let fork_id = ForkId::new(
            chain_config,
            genesis_header,
            block_header.timestamp,
            lastest_block,
        );

        Ok(Self {
            eth_version,
            network_id,
            genesis,
            fork_id,
            earliest_block: 0,
            lastest_block,
            lastest_block_hash,
        })
    }

    pub fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.eth_version)
            .encode_field(&self.network_id)
            .encode_field(&self.genesis)
            .encode_field(&self.fork_id)
            .encode_field(&self.earliest_block)
            .encode_field(&self.lastest_block)
            .encode_field(&self.lastest_block_hash)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    pub fn decode(msg_data: &[u8], expected_version: u8) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (eth_version, decoder): (u32, _) = decoder.decode_field("protocolVersion")?;

        if eth_version != expected_version as u32 {
            return Err(RLPDecodeError::IncompatibleProtocol(format!(
                "Received message is encoded in eth version {} when negotiated eth version was {}",
                eth_version, expected_version
            )));
        }

        let (network_id, decoder): (u64, _) = decoder.decode_field("networkId")?;
        let (genesis, decoder): (BlockHash, _) = decoder.decode_field("genesis")?;
        let (fork_id, decoder): (ForkId, _) = decoder.decode_field("forkId")?;
        let (earliest_block, decoder): (u64, _) = decoder.decode_field("earliestBlock")?;
        let (lastest_block, decoder): (u64, _) = decoder.decode_field("lastestBlock")?;
        let (lastest_block_hash, decoder): (BlockHash, _) = decoder.decode_field("latestHash")?;
        let _padding = decoder.finish_unchecked();

        Ok(Self {
            eth_version: eth_version as u8,
            network_id,
            genesis,
            fork_id,
            earliest_block,
            lastest_block,
            lastest_block_hash,
        })
    }
}
