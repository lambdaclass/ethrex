use std::fmt::Debug;

use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};

use bytes::BufMut;
use ethrex_common::types::{BlockHash, Receipt, ReceiptWithBloom};
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#getreceipts-0x0f
#[derive(Debug)]
pub(crate) struct GetReceipts {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub block_hashes: Vec<BlockHash>,
}

impl GetReceipts {
    pub fn new(id: u64, block_hashes: Vec<BlockHash>) -> Self {
        Self { block_hashes, id }
    }
}

impl RLPxMessage for GetReceipts {
    const CODE: u8 = 0x0F;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.block_hashes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (block_hashes, _): (Vec<BlockHash>, _) = decoder.decode_field("blockHashes")?;

        Ok(Self::new(id, block_hashes))
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#receipts-0x10
#[derive(Debug)]
pub(crate) struct Receipts68 {
    pub id: u64,
    pub receipts: Vec<Vec<Receipt>>,
}

impl Receipts68 {
    pub fn new(id: u64, receipts: Vec<Vec<Receipt>>) -> Self {
        Self { id, receipts }
    }
}

impl RLPxMessage for Receipts68 {
    const CODE: u8 = 0x10;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];

        // Map nested Receipts to ReceiptWithBloom
        let receipts_with_bloom: Vec<Vec<ReceiptWithBloom>> = self
            .receipts
            .iter()
            .map(|receipt_list| receipt_list.iter().map(|receipt| receipt.into()).collect())
            .collect();

        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&receipts_with_bloom)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (receipts_with_bloom, _): (Vec<Vec<ReceiptWithBloom>>, _) =
            decoder.decode_field("receipts")?;

        // Map nested ReceiptWithBloom to Receipts
        let receipts: Vec<Vec<Receipt>> = receipts_with_bloom
            .iter()
            .map(|receipt_list| receipt_list.iter().map(|receipt| receipt.into()).collect())
            .collect();

        Ok(Self::new(id, receipts))
    }
}

#[derive(Debug)]
pub(crate) struct Receipts69 {
    pub id: u64,
    pub receipts: Vec<Vec<Receipt>>,
}

impl Receipts69 {
    pub fn new(id: u64, receipts: Vec<Vec<Receipt>>) -> Self {
        Self { id, receipts }
    }
}

impl RLPxMessage for Receipts69 {
    const CODE: u8 = 0x10;

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
        let (receipts, _): (Vec<Vec<Receipt>>, _) = decoder.decode_field("receipts")?;

        Ok(Self::new(id, receipts))
    }
}

#[cfg(test)]
mod tests {
    use crate::rlpx::{
        eth::receipts::{GetReceipts, Receipts68, Receipts69},
        message::RLPxMessage,
    };
    use ethrex_common::types::{BlockHash, Receipt, TxType};

    #[test]
    fn get_receipts_empty_message() {
        let blocks_hash = vec![];
        let get_receipts = GetReceipts::new(1, blocks_hash.clone());

        let mut buf = Vec::new();
        get_receipts.encode(&mut buf).unwrap();

        let decoded = GetReceipts::decode(&buf).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.block_hashes, blocks_hash);
    }

    #[test]
    fn get_receipts_not_empty_message() {
        let blocks_hash = vec![
            BlockHash::from([0; 32]),
            BlockHash::from([1; 32]),
            BlockHash::from([2; 32]),
        ];
        let get_receipts = GetReceipts::new(1, blocks_hash.clone());

        let mut buf = Vec::new();
        get_receipts.encode(&mut buf).unwrap();

        let decoded = GetReceipts::decode(&buf).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.block_hashes, blocks_hash);
    }

    #[test]
    fn receipts68_empty_message() {
        let receipts = vec![];
        let receipts = Receipts68::new(1, receipts);

        let mut buf = Vec::new();
        receipts.encode(&mut buf).unwrap();

        let decoded = Receipts68::decode(&buf).unwrap();

        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.receipts, Vec::<Vec<Receipt>>::new());
    }

    #[test]
    fn receipts68_not_empty_message() {
        let receipts = vec![vec![
            Receipt::new(TxType::EIP7702, true, 210000, vec![]),
            Receipt::new(TxType::EIP7702, true, 210001, vec![]),
            Receipt::new(TxType::EIP7702, true, 210002, vec![]),
            Receipt::new(TxType::EIP7702, true, 210003, vec![]),
        ]];
        let id = 255;
        let receipts68 = Receipts68::new(id, receipts.clone());

        let mut buf = Vec::new();
        receipts68.encode(&mut buf).unwrap();
        let receipts68_decoded = Receipts68::decode(&buf).unwrap();
        assert_eq!(receipts68_decoded.id, id);
        assert_eq!(receipts68_decoded.receipts, receipts);
    }

    #[test]
    fn receipts69_empty_message() {
        let receipts = vec![];
        let receipts = Receipts69::new(1, receipts);

        let mut buf = Vec::new();
        receipts.encode(&mut buf).unwrap();

        let decoded = Receipts69::decode(&buf).unwrap();

        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.receipts, Vec::<Vec<Receipt>>::new());
    }

    #[test]
    fn receipts69_not_empty_message() {
        let receipts = vec![vec![
            Receipt::new(TxType::EIP7702, true, 210000, vec![]),
            Receipt::new(TxType::EIP7702, true, 210000, vec![]),
            Receipt::new(TxType::EIP7702, true, 210000, vec![]),
            Receipt::new(TxType::EIP7702, true, 210000, vec![]),
        ]];
        let id = 255;
        let receipts69 = Receipts69::new(id, receipts.clone());

        let mut buf = Vec::new();
        receipts69.encode(&mut buf).unwrap();
        let receipts69_decoded = Receipts69::decode(&buf).unwrap();
        assert_eq!(receipts69_decoded.id, id);
        assert_eq!(receipts69_decoded.receipts, receipts);
    }
}
