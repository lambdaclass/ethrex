use ethrex_common::types::{BlockHash, BlockNumber};
#[cfg(feature = "redb")]
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
#[cfg(feature = "libmdbx")]
use libmdbx::orm::{Decodable, Encodable};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockNumHash {
    pub block_number: BlockNumber,
    pub block_hash: BlockHash,
}

impl From<(BlockNumber, BlockHash)> for BlockNumHash {
    fn from(value: (BlockNumber, BlockHash)) -> Self {
        Self {
            block_number: value.0,
            block_hash: value.1,
        }
    }
}

#[cfg(feature = "libmdbx")]
impl Encodable for BlockNumHash {
    type Encoded = [u8; 40];

    fn encode(self) -> Self::Encoded {
        let mut encoded = [0u8; 40];
        encoded[0..8].copy_from_slice(&self.block_number.to_be_bytes());
        encoded[8..40].copy_from_slice(&self.block_hash.0);
        encoded
    }
}

#[cfg(feature = "libmdbx")]
impl Decodable for BlockNumHash {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        if b.len() != 40 {
            anyhow::bail!("Invalid length for (BlockNumber, BlockHash)");
        }
        let block_number = BlockNumber::from_be_bytes(b[0..8].try_into()?);
        let block_hash = ethereum_types::H256::from_slice(&b[8..40]);
        Ok(Self {
            block_number,
            block_hash,
        })
    }
}

#[cfg(feature = "redb")]
impl RLPEncode for BlockNumHash {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.block_number)
            .encode_field(&self.block_hash)
            .finish();
    }
}

#[cfg(feature = "redb")]
impl RLPDecode for BlockNumHash {
    fn decode_unfinished(rlp: &[u8]) -> Result<(BlockNumHash, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (block_number, decoder) = decoder.decode_field("block_number")?;
        let (block_hash, decoder) = decoder.decode_field("block_hash")?;
        Ok((
            BlockNumHash {
                block_number,
                block_hash,
            },
            decoder.finish()?,
        ))
    }
}
