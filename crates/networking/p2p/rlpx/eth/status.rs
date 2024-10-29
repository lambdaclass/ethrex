use bytes::BufMut;
use ethereum_rust_core::{
    types::{BlockHash, ForkId},
    U256,
};
use ethereum_rust_rlp::{
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use snap::raw::Decoder as SnappyDecoder;

use crate::rlpx::{message::RLPxMessage, utils::snappy_encode};

#[derive(Debug)]
pub(crate) struct StatusMessage {
    eth_version: u32,
    network_id: u64,
    total_difficulty: U256,
    block_hash: BlockHash,
    genesis: BlockHash,
    fork_id: ForkId,
}

impl StatusMessage {
    pub fn new(
        eth_version: u32,
        network_id: u64,
        total_difficulty: U256,
        block_hash: BlockHash,
        genesis: BlockHash,
        fork_id: ForkId,
    ) -> Self {
        Self {
            eth_version,
            network_id,
            total_difficulty,
            block_hash,
            genesis,
            fork_id,
        }
    }
}

impl RLPxMessage for StatusMessage {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        16_u8.encode(buf); // msg_id

        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.eth_version)
            .encode_field(&self.network_id)
            .encode_field(&self.total_difficulty)
            .encode_field(&self.block_hash)
            .encode_field(&self.genesis)
            .encode_field(&self.fork_id)
            .finish();

        let msg_data = snappy_encode(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let mut snappy_decoder = SnappyDecoder::new();
        let decompressed_data = snappy_decoder
            .decompress_vec(msg_data)
            .map_err(|e| RLPDecodeError::Custom(e.to_string()))?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (eth_version, decoder): (u32, _) = decoder.decode_field("protocolVersion")?;

        assert_eq!(eth_version, 68, "only eth version 68 is supported");

        let (network_id, decoder): (u64, _) = decoder.decode_field("networkId")?;

        let (total_difficulty, decoder): (U256, _) = decoder.decode_field("totalDifficulty")?;

        let (block_hash, decoder): (BlockHash, _) = decoder.decode_field("blockHash")?;

        let (genesis, decoder): (BlockHash, _) = decoder.decode_field("genesis")?;

        let (fork_id, decoder): (ForkId, _) = decoder.decode_field("forkId")?;

        // Implementations must ignore any additional list elements
        let _padding = decoder.finish_unchecked();

        Ok(Self::new(
            eth_version,
            network_id,
            total_difficulty,
            block_hash,
            genesis,
            fork_id,
        ))
    }
}
