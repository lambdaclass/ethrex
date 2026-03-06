use crate::rlpx::{
    error::PeerConnectionError,
    message::{Message, RLPxMessage},
    utils::{snappy_compress, snappy_decompress},
};
use ethrex_common::utils::keccak;
use ethrex_common::{
    H256, Signature,
    types::{Block, balance_diff::BalanceDiff, batch::Batch, fee_config::FeeConfig},
};
use librlp::{Header, RlpBuf, RlpDecode, RlpEncode, RlpError};
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

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.block.deref().clone().encode(buf);
            self.signature.encode(buf);
            self.fee_config.to_vec().encode(buf);
        });
        let msg_data = snappy_compress(rlp_buf.finish())?;
        buf.extend_from_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RlpError> {
        let decompressed_data =
            snappy_decompress(msg_data).map_err(|e| RlpError::Custom(e.to_string().into()))?;
        let mut buf = decompressed_data.as_slice();
        let header = Header::decode(&mut buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let block = Block::decode(&mut payload)?;
        let signature = Signature::decode(&mut payload)?;
        let fee_config_bytes = Vec::<u8>::decode(&mut payload)?;
        let (_, fee_config) = FeeConfig::decode(&fee_config_bytes)
            .map_err(|e| RlpError::Custom(format!("fee_config decode: {e}").into()))?;
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

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.batch.number.encode(buf);
            self.batch.first_block.encode(buf);
            self.batch.last_block.encode(buf);
            self.batch.state_root.encode(buf);
            self.batch.l1_in_messages_rolling_hash.encode(buf);
            librlp::encode_list(&self.batch.l2_in_message_rolling_hashes, buf);
            self.batch.non_privileged_transactions.encode(buf);
            librlp::encode_list(&self.batch.l1_out_message_hashes, buf);
            librlp::encode_list(&self.batch.blobs_bundle.blobs, buf);
            librlp::encode_list(&self.batch.blobs_bundle.commitments, buf);
            librlp::encode_list(&self.batch.blobs_bundle.proofs, buf);
            // encode optional fields: commit_tx, verify_tx
            if let Some(ref commit_tx) = self.batch.commit_tx {
                commit_tx.encode(buf);
            } else {
                // Encode empty string for None
                Vec::<u8>::new().encode(buf);
            }
            if let Some(ref verify_tx) = self.batch.verify_tx {
                verify_tx.encode(buf);
            } else {
                Vec::<u8>::new().encode(buf);
            }
            self.signature.encode(buf);
            librlp::encode_list(&self.batch.balance_diffs, buf);
        });
        let msg_data = snappy_compress(rlp_buf.finish())?;
        buf.extend_from_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RlpError> {
        let decompressed_data =
            snappy_decompress(msg_data).map_err(|e| RlpError::Custom(e.to_string().into()))?;
        let mut buf = decompressed_data.as_slice();
        let header = Header::decode(&mut buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let batch_number = u64::decode(&mut payload)?;
        let first_block = u64::decode(&mut payload)?;
        let last_block = u64::decode(&mut payload)?;
        let state_root = RlpDecode::decode(&mut payload)?;
        let l1_in_messages_rolling_hash = RlpDecode::decode(&mut payload)?;
        let l2_in_message_rolling_hashes: Vec<(u64, H256)> =
            librlp::decode_list(&mut payload)?;
        let non_privileged_transactions = u64::decode(&mut payload)?;
        let l1_out_message_hashes: Vec<H256> = librlp::decode_list(&mut payload)?;
        let blobs: Vec<[u8; 131072]> = librlp::decode_list(&mut payload)?;
        let commitments: Vec<[u8; 48]> = librlp::decode_list(&mut payload)?;
        let proofs: Vec<[u8; 48]> = librlp::decode_list(&mut payload)?;
        // Decode optional commit_tx and verify_tx
        // Try to decode; if it's an empty string, treat as None
        let commit_tx = {
            let peek_header = Header::decode(&mut payload.clone())?;
            if !peek_header.list && peek_header.payload_length == 0 {
                // empty string — skip it
                let _ = Header::decode(&mut payload)?;
                let _ = &payload[..0]; // consume nothing extra
                None
            } else {
                Some(RlpDecode::decode(&mut payload)?)
            }
        };
        let verify_tx = {
            let peek_header = Header::decode(&mut payload.clone())?;
            if !peek_header.list && peek_header.payload_length == 0 {
                let _ = Header::decode(&mut payload)?;
                None
            } else {
                Some(RlpDecode::decode(&mut payload)?)
            }
        };
        let signature = Signature::decode(&mut payload)?;
        let balance_diffs: Vec<BalanceDiff> = librlp::decode_list(&mut payload)?;

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
                version: 0,
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
