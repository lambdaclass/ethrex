use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    structs::{Decoder, Encoder},
};

use crate::H256;

use super::BlobsBundle;

#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub number: u64,
    pub first_block: u64,
    pub last_block: u64,
    pub state_root: H256,
    pub deposit_logs_hash: H256,
    pub withdrawal_hashes: Vec<H256>,
    pub blobs_bundle: BlobsBundle,
}

impl RLPEncode for Batch {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.number)
            .encode_field(&self.first_block)
            .encode_field(&self.last_block)
            .encode_field(&self.state_root)
            .encode_field(&self.deposit_logs_hash)
            .encode_field(&self.withdrawal_hashes)
            .encode_field(&self.blobs_bundle)
            .finish();
    }
}

impl RLPDecode for Batch {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (number, decoder) = decoder.decode_field("number")?;
        let (first_block, decoder) = decoder.decode_field("first_block")?;
        let (last_block, decoder) = decoder.decode_field("last_block")?;
        let (state_root, decoder) = decoder.decode_field("state_root")?;
        let (deposit_logs_hash, decoder) = decoder.decode_field("deposit_logs_hash")?;
        let (withdrawal_hashes, decoder) = decoder.decode_field("withdrawal_hashes")?;
        let (blobs_bundle, decoder) = decoder.decode_field("blobs_bundle")?;
        Ok((
            Batch {
                number,
                first_block,
                last_block,
                state_root,
                deposit_logs_hash,
                withdrawal_hashes,
                blobs_bundle,
            },
            decoder.finish()?,
        ))
    }
}
