use ethereum_types::H256;
use ethrex_common::types::{BlockHash, BlockNumber};
use libmdbx::orm::{Decodable, Encodable};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct BlockNumHash(pub BlockNumber, pub BlockHash);

impl From<(BlockNumber, BlockHash)> for BlockNumHash {
    fn from(value: (BlockNumber, BlockHash)) -> Self {
        Self(value.0, value.1)
    }
}

impl Encodable for BlockNumHash {
    type Encoded = [u8; 40];

    fn encode(self) -> Self::Encoded {
        let mut encoded = [0u8; 40];
        encoded[0..8].copy_from_slice(&self.0.to_be_bytes());
        encoded[8..40].copy_from_slice(&self.1.0);
        encoded
    }
}

impl Decodable for BlockNumHash {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        if b.len() != 40 {
            anyhow::bail!("Invalid length for (BlockNumber, BlockHash)");
        }
        let block_number = BlockNumber::from_be_bytes(b[0..8].try_into()?);
        let block_hash = H256::from_slice(&b[8..40]);
        Ok((block_number, block_hash).into())
    }
}
