use super::{requests::EncodedRequests, BlobsBundle, Block};
use ethereum_types::U256;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

#[derive(Debug, Clone)]
pub struct PayloadBundle {
    pub block: Block,
    pub block_value: U256,
    pub blobs_bundle: BlobsBundle,
    pub requests: Vec<EncodedRequests>,
    pub completed: bool,
}

impl PayloadBundle {
    pub fn from_block(block: Block) -> Self {
        PayloadBundle {
            block,
            block_value: U256::zero(),
            blobs_bundle: BlobsBundle::empty(),
            requests: Vec::default(),
            completed: false,
        }
    }
}

impl RLPEncode for PayloadBundle {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.block)
            .encode_field(&self.block_value)
            .encode_field(&self.blobs_bundle)
            .encode_field(&self.requests)
            .encode_field(&self.completed)
            .finish();
    }
}

impl RLPDecode for PayloadBundle {
    fn decode_unfinished(rlp: &[u8]) -> Result<(PayloadBundle, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (block, decoder) = decoder.decode_field("block")?;
        let (block_value, decoder) = decoder.decode_field("block_value")?;
        let (blobs_bundle, decoder) = decoder.decode_field("blobs_bundle")?;
        let (requests, decoder) = decoder.decode_field("requests")?;
        let (completed, decoder) = decoder.decode_field("completed")?;
        let state = PayloadBundle {
            block,
            block_value,
            blobs_bundle,
            requests,
            completed,
        };
        Ok((state, decoder.finish()?))
    }
}
