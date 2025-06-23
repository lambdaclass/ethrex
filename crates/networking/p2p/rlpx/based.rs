use bytes::BufMut;
use ethrex_common::types::{Block, batch::Batch};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use sha3::{Digest, Keccak256};

use super::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
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
pub struct NewBatchSealedMessage {
    pub batch: Batch,
    pub signature: [u8; 64],
    pub recovery_id: [u8; 4],
}
impl RLPxMessage for NewBatchSealedMessage {
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
        Ok(NewBatchSealedMessage {
            batch,
            signature,
            recovery_id,
        })
    }
}

pub fn get_hash_batch_sealed(batch: &Batch) -> [u8; 32] {
    let withdrawal_bytes: Vec<u8> = batch
        .withdrawal_hashes
        .iter()
        .flat_map(|hash| hash.as_bytes().to_vec())
        .collect();

    let mut hasher = Keccak256::new();
    hasher.update(batch.number.to_be_bytes());
    hasher.update(batch.first_block.to_be_bytes());
    hasher.update(batch.last_block.to_be_bytes());
    hasher.update(batch.state_root.as_bytes());
    hasher.update(batch.deposit_logs_hash.as_bytes());
    hasher.update(&withdrawal_bytes);
    // missing blobs_bundle for now
    let next_batch_hash = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&next_batch_hash);
    hash
}

#[derive(Debug, Clone)]
pub struct GetBatchSealedMessage {
    pub batch_number: u64,
}

impl RLPxMessage for GetBatchSealedMessage {
    const CODE: u8 = 0x2;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.batch_number)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (batch_number, decoder) = decoder.decode_field("batch_number")?;
        decoder.finish()?;
        Ok(GetBatchSealedMessage { batch_number })
    }
}

#[derive(Debug, Clone)]
pub struct GetBatchSealedResponseMessage {
    pub batch: Batch,
}
impl RLPxMessage for GetBatchSealedResponseMessage {
    const CODE: u8 = 0x3;

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
        Ok(GetBatchSealedResponseMessage { batch })
    }
}
