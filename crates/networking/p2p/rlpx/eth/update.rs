use crate::rlpx::error::RLPxError;
use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::types::BlockHash;
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use ethrex_storage::Store;

#[derive(Debug, Clone)]
pub struct BlockRangeUpdate {
    pub earliest_block: u64,
    pub lastest_block: u64,
    pub lastest_block_hash: BlockHash,
}

impl BlockRangeUpdate {
    pub async fn new(storage: &Store) -> Result<Self, RLPxError> {
        let lastest_block = storage.get_latest_block_number().await?;
        let block_header = storage
            .get_block_header(lastest_block)?
            .ok_or(RLPxError::NotFound(format!("Block {lastest_block}")))?;
        let lastest_block_hash = block_header.hash();

        Ok(Self {
            earliest_block: 0,
            lastest_block,
            lastest_block_hash,
        })
    }

    /// Validates an incoming BlockRangeUpdate from a peer
    pub fn validate(&self) -> Result<(), RLPxError> {
        if self.earliest_block > self.lastest_block || self.lastest_block_hash.is_zero() {
            return Err(RLPxError::InvalidBlockRangeUpdate);
        }
        Ok(())
    }
}

impl RLPxMessage for BlockRangeUpdate {
    const CODE: u8 = 0x11;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
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
        let (earliest_block, decoder): (u64, _) = decoder.decode_field("earliestBlock")?;
        let (lastest_block, decoder): (u64, _) = decoder.decode_field("lastestBlock")?;
        let (lastest_block_hash, decoder): (BlockHash, _) = decoder.decode_field("latestHash")?;
        // Implementations must ignore any additional list elements
        let _padding = decoder.finish_unchecked();

        Ok(Self {
            earliest_block,
            lastest_block,
            lastest_block_hash,
        })
    }
}
