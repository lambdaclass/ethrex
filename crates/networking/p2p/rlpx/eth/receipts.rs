pub use super::eth68::receipts::Receipts68;
pub use super::eth69::receipts::Receipts69;
use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};

use ethrex_common::types::BlockHash;
use librlp::{Header, RlpBuf, RlpDecode, RlpEncode, RlpError, decode_list, encode_list};

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#getreceipts-0x0f
#[derive(Debug, Clone)]
pub struct GetReceipts {
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
    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.id.encode(buf);
            encode_list(&self.block_hashes, buf);
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
        let id = u64::decode(&mut payload)?;
        let block_hashes = decode_list::<BlockHash>(&mut payload)?;

        Ok(Self::new(id, block_hashes))
    }
}

#[cfg(test)]
mod tests {
    use ethrex_common::types::Receipt;

    use super::*;

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
    fn receipts_empty_message() {
        let receipts = vec![];
        let receipts = Receipts68::new(1, receipts);

        let mut buf = Vec::new();
        receipts.encode(&mut buf).unwrap();

        let decoded = Receipts68::decode(&buf).unwrap();

        assert_eq!(decoded.get_id(), 1);
        assert_eq!(decoded.get_receipts(), Vec::<Vec<Receipt>>::new());
    }
}
