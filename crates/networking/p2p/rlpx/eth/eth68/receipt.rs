use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::types::Receipt;
use ethrex_rlp::{
    encode::encode_length,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

#[derive(Debug)]
pub(crate) struct Receipts68 {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub receipts: Vec<Vec<Receipt>>,
}

impl Receipts68 {
    pub fn new(id: u64, receipts: Vec<Vec<Receipt>>) -> Self {
        Self { receipts, id }
    }
}

impl RLPxMessage for Receipts68 {
    const CODE: u8 = 0x1F;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut tmp_buf = vec![];
        if self.receipts.is_empty() {
            tmp_buf.put_u8(0xc0);
        } else {
            let mut inner_buf_1 = vec![];
            for item in &self.receipts {
                if item.is_empty() {
                    inner_buf_1.put_u8(0xc0);
                } else {
                    let mut inner_buf_2 = vec![];
                    for receipt in item {
                        receipt.encode68(&mut inner_buf_2);
                    }
                    encode_length(inner_buf_2.len(), &mut inner_buf_1);
                    inner_buf_1.put_slice(&inner_buf_2);
                }
            }
            encode_length(inner_buf_1.len(),&mut  tmp_buf);
            tmp_buf.put_slice(&inner_buf_1);
        }

        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data).encode_field(&self.id).encode_raw(&tmp_buf[..]).finish();

        // .encode_field(&self.receipts)
        // .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (receipts, _): (Vec<Vec<Receipt>>, _) = decoder.decode_field("receipts")?;

        Ok(Self::new(id, receipts))
    }
}
