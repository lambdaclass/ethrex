use crate::rlpx::{
    error::PeerConnectionError,
    message::{Message, RLPxMessage},
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::utils::keccak;
use ethrex_common::{
    H256, Signature,
    types::{Block, batch::Batch, fee_config::FeeConfig},
};
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use ethrex_rlp::structs::{Decoder, Encoder};
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
    pub signature: Signature,
    pub fee_config: FeeConfig,
}

impl RLPxMessage for NewBlock {
    const CODE: u8 = 0x0;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.block.deref().clone())
            .encode_field(&self.signature)
            .encode_field(&self.fee_config.to_vec())
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
        let (fee_config_bytes, decoder): (Vec<u8>, _) = decoder.decode_field("fee_config")?;
        decoder.finish()?;
        let (_, fee_config) = FeeConfig::decode(&fee_config_bytes)
            .map_err(|e| RLPDecodeError::Custom(format!("fee_config decode: {e}")))?;
        Ok(NewBlock {
            block: Arc::new(block),
            signature,
            fee_config,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BatchSealed {
    pub batch: Arc<Batch>,
    pub signature: Signature,
}

impl BatchSealed {
    pub fn from_batch_and_key(
        batch: Batch,
        secret_key: &SecretKey,
    ) -> Result<Self, PeerConnectionError> {
        let hash = batch_hash(&batch);
        let (recovery_id, signature) = secp256k1::SECP256K1
            .sign_ecdsa_recoverable(&SecpMessage::from_digest(hash.into()), secret_key)
            .serialize_compact();
        let recovery_id: u8 = Into::<i32>::into(recovery_id).try_into().map_err(|e| {
            PeerConnectionError::InternalError(format!(
                "Failed to convert recovery id to u8: {e}. This is a bug."
            ))
        })?;
        let mut sig = [0u8; 65];
        sig[..64].copy_from_slice(&signature);
        sig[64] = recovery_id;
        let signature = Signature::from_slice(&sig);
        Ok(Self {
            batch: Arc::new(batch),
            signature,
        })
    }
    pub fn new(batch: Batch, signature: Signature) -> Self {
        Self {
            batch: Arc::new(batch),
            signature,
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
            .encode_field(&self.batch.number)
            .encode_field(&self.batch.first_block)
            .encode_field(&self.batch.last_block)
            .encode_field(&self.batch.state_root)
            .encode_field(&self.batch.l1_in_messages_rolling_hash)
            .encode_field(&self.batch.l2_in_message_rolling_hashes)
            .encode_field(&self.batch.non_privileged_transactions)
            .encode_field(&self.batch.l1_out_message_hashes)
            .encode_field(&self.batch.blobs_bundle.blobs)
            .encode_field(&self.batch.blobs_bundle.commitments)
            .encode_field(&self.batch.blobs_bundle.proofs)
            .encode_optional_field(&self.batch.commit_tx)
            .encode_optional_field(&self.batch.verify_tx)
            .encode_field(&self.signature)
            .encode_field(&self.batch.balance_diffs)
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
        let (l1_in_messages_rolling_hash, decoder) =
            decoder.decode_field("l1_in_messages_rolling_hash")?;
        let (l2_in_message_rolling_hashes, decoder) =
            decoder.decode_field("l2_in_message_rolling_hashes")?;
        let (non_privileged_transactions, decoder) =
            decoder.decode_field("non_privileged_transactions")?;
        let (l1_out_message_hashes, decoder) = decoder.decode_field("l1_out_message_hashes")?;
        let (blobs, decoder) = decoder.decode_field("blobs")?;
        let (commitments, decoder) = decoder.decode_field("commitments")?;
        let (proofs, decoder) = decoder.decode_field("proofs")?;
        let (commit_tx, decoder) = decoder.decode_optional_field();
        let (verify_tx, decoder) = decoder.decode_optional_field();
        let (signature, decoder) = decoder.decode_field("signature")?;
        let (balance_diffs, decoder) = decoder.decode_field("balance_diffs")?;
        decoder.finish()?;

        let batch = Batch {
            number: batch_number,
            first_block,
            last_block,
            state_root,
            l1_in_messages_rolling_hash,
            l2_in_message_rolling_hashes,
            l1_out_message_hashes,
            non_privileged_transactions,
            blobs_bundle: ethrex_common::types::blobs_bundle::BlobsBundle {
                blobs,
                commitments,
                proofs,
                // `based/1` predates the sidecar version field, so keep its wire
                // schema unchanged for rolling upgrades. New nodes are v1-only
                // and validate the proof shape before accepting the batch.
                version: 1,
            },
            commit_tx,
            verify_tx,
            balance_diffs,
        };
        Ok(BatchSealed::new(batch, signature))
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

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::BlobsBundle;

    #[test]
    fn batch_sealed_keeps_the_based_1_wire_schema() {
        let batch = Batch {
            number: 7,
            first_block: 11,
            last_block: 13,
            blobs_bundle: BlobsBundle {
                version: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let message = BatchSealed::new(batch, Signature::from_slice(&[0; 65]));

        let mut actual = Vec::new();
        message.encode(&mut actual).expect("encode BatchSealed");

        // Encode the original based/1 field list explicitly. A version field
        // must not be appended because older peers reject trailing RLP data.
        let mut legacy_payload = Vec::new();
        Encoder::new(&mut legacy_payload)
            .encode_field(&message.batch.number)
            .encode_field(&message.batch.first_block)
            .encode_field(&message.batch.last_block)
            .encode_field(&message.batch.state_root)
            .encode_field(&message.batch.l1_in_messages_rolling_hash)
            .encode_field(&message.batch.l2_in_message_rolling_hashes)
            .encode_field(&message.batch.non_privileged_transactions)
            .encode_field(&message.batch.l1_out_message_hashes)
            .encode_field(&message.batch.blobs_bundle.blobs)
            .encode_field(&message.batch.blobs_bundle.commitments)
            .encode_field(&message.batch.blobs_bundle.proofs)
            .encode_optional_field(&message.batch.commit_tx)
            .encode_optional_field(&message.batch.verify_tx)
            .encode_field(&message.signature)
            .encode_field(&message.batch.balance_diffs)
            .finish();
        let expected = snappy_compress(legacy_payload).expect("compress based/1 message");

        assert_eq!(actual, expected);

        let decoded = BatchSealed::decode(&actual).expect("decode based/1 message");
        assert_eq!(decoded.batch.blobs_bundle.version, 1);
    }

    #[test]
    fn batch_sealed_rejects_legacy_v0_proof_shape() {
        use ethrex_common::types::{BYTES_PER_BLOB, CELLS_PER_EXT_BLOB};

        // Wire format has no version field. Decode stamps version=1, then the
        // proof-count invariant must reject a legacy one-proof-per-blob sidecar.
        let batch = Batch {
            number: 9,
            first_block: 1,
            last_block: 2,
            blobs_bundle: BlobsBundle {
                blobs: vec![[7u8; BYTES_PER_BLOB]],
                commitments: vec![[8u8; 48]],
                proofs: vec![[9u8; 48]], // v0 shape: one proof
                version: 1,
            },
            ..Default::default()
        };
        let message = BatchSealed::new(batch, Signature::from_slice(&[0; 65]));

        let mut encoded = Vec::new();
        message.encode(&mut encoded).expect("encode");
        let decoded = BatchSealed::decode(&encoded).expect("decode stamps version 1");
        assert_eq!(decoded.batch.blobs_bundle.version, 1);
        assert_ne!(
            decoded.batch.blobs_bundle.proofs.len(),
            CELLS_PER_EXT_BLOB,
            "fixture must use the legacy one-proof layout"
        );
        assert!(
            decoded.batch.blobs_bundle.validate_v1_structure().is_err(),
            "followers must reject legacy v0-shaped BatchSealed proofs"
        );
    }
}
