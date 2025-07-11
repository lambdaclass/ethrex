use super::eth68::receipts::Receipts68;
use super::eth69::receipts::Receipts69;
use crate::rlpx::{
    error::RLPxError,
    message::RLPxMessage,
    p2p::Capability,
    utils::{snappy_compress, snappy_decompress},
};
use ethereum_types::Bloom;

use bytes::BufMut;
use ethrex_common::types::{BlockHash, Receipt};
use ethrex_rlp::{
    decode::static_left_pad,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#getreceipts-0x0f
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub(crate) enum Receipts {
    Receipts68(Receipts68),
    Receipts69(Receipts69),
}

impl Receipts {
    pub const CODE: u8 = 0x10;
    pub fn new(id: u64, receipts: Vec<Vec<Receipt>>, eth: &Capability) -> Result<Self, RLPxError> {
        match eth.version {
            68 => Ok(Receipts::Receipts68(Receipts68::new(id, receipts))),
            69 => Ok(Receipts::Receipts69(Receipts69::new(id, receipts))),
            _ => Err(RLPxError::IncompatibleProtocol),
        }
    }

    pub fn get_receipts(&self) -> Vec<Vec<Receipt>> {
        match self {
            Receipts::Receipts68(msg) => msg.get_receipts(),
            Receipts::Receipts69(msg) => msg.receipts.clone(),
        }
    }

    pub fn get_id(&self) -> u64 {
        match self {
            Receipts::Receipts68(msg) => msg.id,
            Receipts::Receipts69(msg) => msg.id,
        }
    }
}

#[cfg(test)]
mod tests {
    //use crate::rlpx::eth::receipts::has_bloom;
    use crate::rlpx::{
        eth::receipts::{GetReceipts, Receipts},
        message::RLPxMessage,
        p2p::Capability,
    };
    use ethrex_common::types::transaction::TxType;
    use ethrex_common::types::{BlockHash, Receipt};

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

    // #[test]
    // fn receipts_empty_message() {
    //     let receipts = vec![];
    //     let receipts = Receipts::new(1, receipts, &Capability::eth(68)).unwrap();

    //     let mut buf = Vec::new();
    //     receipts.encode(&mut buf).unwrap();

    //     let decoded = Receipts::decode(&buf).unwrap();

    //     assert_eq!(decoded.get_id(), 1);
    //     assert_eq!(decoded.get_receipts(), Vec::<Vec<Receipt>>::new());
    // }

    // #[test]
    // fn receipts_check_bloom() {
    //     let receipts = vec![vec![
    //         Receipt::new(TxType::EIP7702, true, 210000, vec![]),
    //         Receipt::new(TxType::EIP7702, true, 210000, vec![]),
    //         Receipt::new(TxType::EIP7702, true, 210000, vec![]),
    //         Receipt::new(TxType::EIP7702, true, 210000, vec![]),
    //     ]];
    //     let receipts68 = Receipts::new(255, receipts.clone(), &Capability::eth(68)).unwrap();
    //     let receipts69 = Receipts::new(255, receipts, &Capability::eth(69)).unwrap();

    //     let mut buf = Vec::new();
    //     receipts68.encode(&mut buf).unwrap();
    //     assert!(has_bloom(&buf).unwrap());

    //     let mut buf = Vec::new();
    //     receipts69.encode(&mut buf).unwrap();
    //     assert!(!has_bloom(&buf).unwrap());
    // }
}
