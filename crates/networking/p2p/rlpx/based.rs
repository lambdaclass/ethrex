use bytes::BufMut;
use ethrex_common::{types::Block, H256};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

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

#[derive(Debug)]
pub struct BatchSealedMessage {
    pub batch_number: u64,
    pub block_numbers: Vec<u64>,
    pub withdrawal_hashes: Vec<H256>,
}
impl RLPxMessage for BatchSealedMessage {
    const CODE: u8 = 0x1;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.batch_number)
            .encode_field(&self.block_numbers)
            .encode_field(&self.withdrawal_hashes)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (batch_number, decoder) = decoder.decode_field("batch_number")?;
        let (block_numbers, decoder) = decoder.decode_field("block_numbers")?;
        let (withdrawal_hashes, decoder) = decoder.decode_field("withdrawal_hashes")?;
        decoder.finish()?;
        Ok(BatchSealedMessage {
            batch_number,
            block_numbers,
            withdrawal_hashes,
        })
    }
}
