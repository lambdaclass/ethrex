use crate::rlpx::error::PeerConnectionError;
use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use ethrex_common::types::BlockHash;
use ethrex_storage::Store;
use librlp::{Header, RlpBuf, RlpDecode, RlpEncode, RlpError};

#[derive(Debug, Clone)]
pub struct BlockRangeUpdate {
    pub earliest_block: u64,
    pub latest_block: u64,
    pub latest_block_hash: BlockHash,
}

impl BlockRangeUpdate {
    pub async fn new(storage: &Store) -> Result<Self, PeerConnectionError> {
        let latest_block = storage.get_latest_block_number().await?;
        let block_header =
            storage
                .get_block_header(latest_block)?
                .ok_or(PeerConnectionError::NotFound(format!(
                    "Block {latest_block}"
                )))?;
        let latest_block_hash = block_header.hash();

        Ok(Self {
            earliest_block: 0,
            latest_block,
            latest_block_hash,
        })
    }

    /// Validates an incoming BlockRangeUpdate from a peer
    pub fn validate(&self) -> Result<(), PeerConnectionError> {
        if self.earliest_block > self.latest_block || self.latest_block_hash.is_zero() {
            return Err(PeerConnectionError::InvalidBlockRangeUpdate);
        }
        Ok(())
    }
}

impl RLPxMessage for BlockRangeUpdate {
    const CODE: u8 = 0x11;
    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.earliest_block.encode(buf);
            self.latest_block.encode(buf);
            self.latest_block_hash.encode(buf);
        });
        let msg_data = snappy_compress(rlp_buf.finish())?;
        buf.extend_from_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RlpError> {
        let decompressed_data =
            snappy_decompress(msg_data).map_err(|e| RlpError::Custom(e.to_string().into()))?;
        let mut buf = decompressed_data.as_slice();
        let header = Header::decode(&mut buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let earliest_block = u64::decode(&mut payload)?;
        let latest_block = u64::decode(&mut payload)?;
        let latest_block_hash = BlockHash::decode(&mut payload)?;
        // Implementations must ignore any additional list elements

        Ok(Self {
            earliest_block,
            latest_block,
            latest_block_hash,
        })
    }
}
