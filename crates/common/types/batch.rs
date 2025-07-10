use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    structs::{Decoder, Encoder},
};
use serde::{Deserialize, Serialize};

use crate::H256;

use super::BlobsBundle;

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Batch {
    pub number: u64,
    pub first_block: u64,
    pub last_block: u64,
    pub state_root: H256,
    pub privileged_transactions_hash: H256,
    pub message_hashes: Vec<H256>,
    #[serde(skip_serializing, skip_deserializing)]
    pub blobs_bundle: BlobsBundle,
    pub commit_tx: Option<H256>,
    pub verify_tx: Option<H256>,
}

impl RLPEncode for Batch {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.number)
            .encode_field(&self.first_block)
            .encode_field(&self.last_block)
            .encode_field(&self.state_root)
            .encode_field(&self.privileged_transactions_hash)
            .encode_field(&self.message_hashes)
            .encode_field(&self.blobs_bundle)
            .encode_optional_field(&self.commit_tx)
            .encode_optional_field(&self.verify_tx)
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
        let (privileged_transactions_hash, decoder) =
            decoder.decode_field("privileged_transactions_hash")?;
        let (message_hashes, decoder) = decoder.decode_field("message_hashes")?;
        let (blobs_bundle, decoder) = decoder.decode_field("blobs_bundle")?;
        let (commit_tx, decoder) = decoder.decode_optional_field();
        let (verify_tx, decoder) = decoder.decode_optional_field();
        Ok((
            Batch {
                number,
                first_block,
                last_block,
                state_root,
                privileged_transactions_hash,
                message_hashes,
                blobs_bundle,
                commit_tx,
                verify_tx,
            },
            decoder.finish()?,
        ))
    }
}
