use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::types::{Receipt, ReceiptWithBloom};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

#[derive(Debug, Clone)]
pub struct Receipts68 {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub receipts: Vec<Vec<ReceiptWithBloom>>,
}

impl Receipts68 {
    pub fn new(id: u64, receipts: Vec<Vec<Receipt>>) -> Self {
        if receipts.is_empty() {
            return Self {
                id,
                receipts: vec![],
            };
        }
        let transformed_receipts = receipts
            .iter()
            .map(|receipts| receipts.iter().map(ReceiptWithBloom::from).collect())
            .collect();
        Self {
            id,
            receipts: transformed_receipts,
        }
    }

    pub fn get_receipts(&self) -> Vec<Vec<Receipt>> {
        if self.receipts.is_empty() {
            return vec![];
        }
        self.receipts
            .iter()
            .map(|receipts| receipts.iter().map(Receipt::from).collect())
            .collect()
    }
}

impl RLPxMessage for Receipts68 {
    const CODE: u8 = 0x0F;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.receipts)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (receipts, _): (Vec<Vec<ReceiptWithBloom>>, _) = decoder.decode_field("receipts")?;

        Ok(Receipts68 { id, receipts })
    }
}
