use super::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::types::{Block, batch::Batch};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

#[derive(Debug, Clone)]
pub struct NewBlockMessage {
    pub block: Block,
    pub signature: [u8; 64],
    pub recovery_id: [u8; 4],
}

impl RLPxMessage for NewBlockMessage {
    const CODE: u8 = 0x0;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.block)
            .encode_field(&self.signature)
            .encode_field(&self.recovery_id)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (block, decoder) = decoder.decode_field("block")?;
        let (signature, decoder) = decoder.decode_field("signature")?;
        let (recovery_id, decoder) = decoder.decode_field("recovery_id")?;
        decoder.finish()?;
        Ok(NewBlockMessage {
            block,
            signature,
            recovery_id,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BatchSealedMessage {
    pub batch: Batch,
    pub signature: [u8; 64],
    pub recovery_id: [u8; 4],
}
impl RLPxMessage for BatchSealedMessage {
    const CODE: u8 = 0x1;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.batch.number)
            .encode_field(&self.batch.first_block)
            .encode_field(&self.batch.last_block)
            .encode_field(&self.batch.state_root)
            .encode_field(&self.batch.privileged_transactions_hash)
            .encode_field(&self.batch.message_hashes)
            .encode_field(&self.batch.blobs_bundle.blobs)
            .encode_field(&self.batch.blobs_bundle.commitments)
            .encode_field(&self.batch.blobs_bundle.proofs)
            .encode_field(&self.signature)
            .encode_field(&self.recovery_id)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (batch_number, decoder) = decoder.decode_field("batch_number")?;
        let (first_block, decoder) = decoder.decode_field("first_block")?;
        let (last_block, decoder) = decoder.decode_field("last_block")?;
        let (state_root, decoder) = decoder.decode_field("state_root")?;
        let (blobs, decoder) = decoder.decode_field("blobs")?;
        let (commitments, decoder) = decoder.decode_field("commitments")?;
        let (proofs, decoder) = decoder.decode_field("proofs")?;
        let (signature, decoder) = decoder.decode_field("signature")?;
        let (recovery_id, decoder) = decoder.decode_field("recovery_id")?;
        let (commit_tx, decoder) = decoder.decode_optional_field();
        let (message_hashes, decoder) = decoder.decode_field("message_hashes")?;
        let (privileged_transactions_hash, decoder) =
            decoder.decode_field("privileged_transactions_hash")?;
        let (verify_tx, decoder) = decoder.decode_optional_field();
        decoder.finish()?;
        let batch = Batch {
            number: batch_number,
            first_block,
            last_block,
            state_root,
            blobs_bundle: ethrex_common::types::blobs_bundle::BlobsBundle {
                blobs,
                commitments,
                proofs,
            },
            privileged_transactions_hash,
            message_hashes,
            commit_tx,
            verify_tx,
        };
        Ok(BatchSealedMessage {
            batch,
            signature,
            recovery_id,
        })
    }
}
