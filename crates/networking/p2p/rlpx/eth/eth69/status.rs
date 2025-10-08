use crate::rlpx::{
    error::PeerConnectionError,
    eth::status::StatusMessage,
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::types::{BlockHash, ForkId};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use ethrex_storage::Store;

#[derive(Debug, Clone)]
pub struct StatusMessage69 {
    pub(crate) eth_version: u8,
    pub(crate) network_id: u64,
    pub(crate) genesis: BlockHash,
    pub(crate) fork_id: ForkId,
    pub(crate) earliest_block: u64,
    pub(crate) lastest_block: u64,
    pub(crate) lastest_block_hash: BlockHash,
}

impl RLPxMessage for StatusMessage69 {
    const CODE: u8 = 0x00;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
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

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (eth_version, decoder): (u32, _) = decoder.decode_field("protocolVersion")?;

        if eth_version != 69 {
            return Err(RLPDecodeError::IncompatibleProtocol(format!(
                "Received message is encoded in eth version {} when negotiated eth version was 69",
                eth_version
            )));
        }

        let (network_id, decoder): (u64, _) = decoder.decode_field("networkId")?;
        let (genesis, decoder): (BlockHash, _) = decoder.decode_field("genesis")?;
        let (fork_id, decoder): (ForkId, _) = decoder.decode_field("forkId")?;
        let (earliest_block, decoder): (u64, _) = decoder.decode_field("earliestBlock")?;
        let (lastest_block, decoder): (u64, _) = decoder.decode_field("lastestBlock")?;
        let (lastest_block_hash, decoder): (BlockHash, _) = decoder.decode_field("latestHash")?;
        // Implementations must ignore any additional list elements
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

impl StatusMessage69 {
    pub async fn new(storage: &Store) -> Result<Self, PeerConnectionError> {
        let chain_config = storage.get_chain_config()?;
        let network_id = chain_config.chain_id;

        // These blocks must always be available
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

        Ok(StatusMessage69 {
            eth_version: 69,
            network_id,
            genesis,
            fork_id,
            earliest_block: 0,
            lastest_block,
            lastest_block_hash,
        })
    }
}

impl StatusMessage for StatusMessage69 {
    fn get_network_id(&self) -> u64 {
        self.network_id
    }

    fn get_eth_version(&self) -> u8 {
        self.eth_version
    }

    fn get_fork_id(&self) -> ForkId {
        self.fork_id.clone()
    }

    fn get_genesis(&self) -> BlockHash {
        self.genesis
    }
}
