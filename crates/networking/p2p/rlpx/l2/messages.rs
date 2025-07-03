use crate::rlpx::{
    message::{Message, RLPxMessage},
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::types::{Block, batch::Batch};
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use ethrex_rlp::structs::{Decoder, Encoder};
use std::{ops::Deref as _, sync::Arc};

#[derive(Debug, Clone)]
pub struct NewBlock {
    // Not ideal to have an Arc here, but without it, clippy complains
    // that this struct is bigger than the other variant when used in the
    // L2Message enum definition. Since we don't modify this
    // block field, we don't need a Box, and we also get the benefit
    // of (almost) freely cloning the pointer instead of the block iself
    // when broadcasting this message.
    pub block: Arc<Block>,
    pub signature: [u8; 64],
    pub recovery_id: [u8; 4],
}

impl RLPxMessage for NewBlock {
    const CODE: u8 = 0x0;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.block.deref().clone())
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
        Ok(NewBlock {
            block: Arc::new(block),
            signature,
            recovery_id,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BatchSealed {
    pub batch: Batch,
    pub signature: [u8; 64],
    pub recovery_id: [u8; 4],
}

impl RLPxMessage for BatchSealed {
    const CODE: u8 = 0x1;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.batch.number)
            .encode_field(&self.batch.first_block)
            .encode_field(&self.batch.last_block)
            .encode_field(&self.batch.state_root)
            .encode_field(&self.batch.deposit_logs_hash)
            .encode_field(&self.batch.withdrawal_hashes)
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
        let (deposit_logs_hash, decoder) = decoder.decode_field("deposit_logs_hash")?;
        let (withdrawal_hashes, decoder) = decoder.decode_field("withdrawal_hashes")?;
        let (blobs, decoder) = decoder.decode_field("blobs")?;
        let (commitments, decoder) = decoder.decode_field("commitments")?;
        let (proofs, decoder) = decoder.decode_field("proofs")?;
        let (signature, decoder) = decoder.decode_field("signature")?;
        let (recovery_id, decoder) = decoder.decode_field("recovery_id")?;
        decoder.finish()?;
        let batch = Batch {
            number: batch_number,
            first_block,
            last_block,
            state_root,
            deposit_logs_hash,
            withdrawal_hashes,
            blobs_bundle: ethrex_common::types::blobs_bundle::BlobsBundle {
                blobs,
                commitments,
                proofs,
            },
        };
        Ok(BatchSealed {
            batch,
            signature,
            recovery_id,
        })
    }
}
#[derive(Debug, Clone)]
pub enum L2Message {
    BatchSealed(BatchSealed),
    NewBlock(NewBlock),
}

// I don't really like doing ad-hoc 'from' implementations,
// but this makes creating messages for the L2 variants
// less verbose, if we ever end up with too many variants,
// we could check into a more definitive solution (derive_more, strum, etc.).
impl From<BatchSealed> for crate::rlpx::message::Message {
    fn from(value: BatchSealed) -> Self {
        L2Message::BatchSealed(value).into()
    }
}

impl From<NewBlock> for crate::rlpx::message::Message {
    fn from(value: NewBlock) -> Self {
        L2Message::NewBlock(value).into()
    }
}

impl From<L2Message> for crate::rlpx::message::Message {
    fn from(value: L2Message) -> Self {
        Message::L2(value)
    }
}
