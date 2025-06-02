use bytes::BufMut;
use ethrex_common::types::Block;
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
}

impl RLPxMessage for NewBlockMessage {
    const CODE: u8 = 0x0;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.block)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (block, _) = decoder.decode_field("block")?;
        Ok(NewBlockMessage { block })
    }
}
