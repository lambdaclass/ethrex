use crate::rlpx::{
    message::{Message, RLPxMessage},
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::{
    H256,
    types::{Block, batch::Batch},
};
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use ethrex_rlp::structs::{Decoder, Encoder};
use keccak_hash::keccak;
use secp256k1::{Message as SecpMessage, SecretKey};
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
    pub batch: Box<Batch>,
    pub signature: [u8; 64],
    pub recovery_id: [u8; 4],
}

impl BatchSealed {
    pub fn from_batch_and_key(batch: Batch, secret_key: &SecretKey) -> Self {
        let hash = batch_hash(&batch);
        let (recovery_id, signature) = secp256k1::SECP256K1
            .sign_ecdsa_recoverable(&SecpMessage::from_digest(hash.into()), secret_key)
            .serialize_compact();
        let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();
        Self {
            batch: Box::new(batch),
            recovery_id,
            signature,
        }
    }
    pub fn new(batch: Batch, signature: [u8; 64], recovery_id: [u8; 4]) -> Self {
        Self {
            batch: Box::new(batch),
            signature,
            recovery_id,
        }
    }
}

pub fn batch_hash(sealed_batch: &Batch) -> H256 {
    let input = [
        sealed_batch.first_block.to_be_bytes(),
        sealed_batch.last_block.to_be_bytes(),
        sealed_batch.number.to_be_bytes(),
    ];
    keccak(input.as_flattened())
}

impl RLPxMessage for BatchSealed {
    const CODE: u8 = 0x1;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&*self.batch)
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
        let (batch, decoder) = decoder.decode_field("batch")?;
        let (signature, decoder) = decoder.decode_field("signature")?;
        let (recovery_id, decoder) = decoder.decode_field("recovery_id")?;
        decoder.finish()?;

        Ok(BatchSealed::new(batch, signature, recovery_id))
    }
}

#[derive(Debug, Clone)]
pub struct GetBatchSealed {
    pub first_batch: u64,
    pub last_batch: u64,
}

impl RLPxMessage for GetBatchSealed {
    const CODE: u8 = 0x2;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.first_batch)
            .encode_field(&self.last_batch)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (first_batch, decoder) = decoder.decode_field("first_batch")?;
        let (last_batch, decoder) = decoder.decode_field("last_batch")?;
        decoder.finish()?;
        Ok(GetBatchSealed {
            first_batch,
            last_batch,
        })
    }
}

#[derive(Debug, Clone)]
pub struct GetBatchSealedResponse {
    pub batches: Vec<Batch>,
}
impl RLPxMessage for GetBatchSealedResponse {
    const CODE: u8 = 0x3;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.batches)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (batches, decoder) = decoder.decode_field("batches")?;
        decoder.finish()?;
        Ok(GetBatchSealedResponse { batches })
    }
}

#[derive(Debug, Clone)]
pub enum L2Message {
    BatchSealed(BatchSealed),
    NewBlock(NewBlock),
    GetBatchSealed(GetBatchSealed),
    GetBatchSealedResponse(GetBatchSealedResponse),
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

impl From<GetBatchSealed> for crate::rlpx::message::Message {
    fn from(value: GetBatchSealed) -> Self {
        L2Message::GetBatchSealed(value).into()
    }
}

impl From<GetBatchSealedResponse> for crate::rlpx::message::Message {
    fn from(value: GetBatchSealedResponse) -> Self {
        L2Message::GetBatchSealedResponse(value).into()
    }
}

impl From<L2Message> for crate::rlpx::message::Message {
    fn from(value: L2Message) -> Self {
        Message::L2(value)
    }
}
