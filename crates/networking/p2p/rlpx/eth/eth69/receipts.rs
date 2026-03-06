use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use ethrex_common::types::Receipt;
use librlp::{Header, RlpBuf, RlpDecode, RlpEncode, RlpError, decode_list, encode_list};

#[derive(Debug, Clone)]
pub struct Receipts69 {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub receipts: Vec<Vec<Receipt>>,
}

impl Receipts69 {
    pub fn new(id: u64, receipts: Vec<Vec<Receipt>>) -> Self {
        Self { receipts, id }
    }

    pub fn get_id(&self) -> u64 {
        self.id
    }
}

impl RLPxMessage for Receipts69 {
    const CODE: u8 = 0x10;

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.id.encode(buf);
            buf.list(|inner| {
                for block_receipts in &self.receipts {
                    encode_list(block_receipts, inner);
                }
            });
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
        let receipts = {
            let list_header = Header::decode(&mut payload)?;
            if !list_header.list {
                return Err(RlpError::UnexpectedString);
            }
            let mut list_payload = &payload[..list_header.payload_length];
            let mut receipts = Vec::new();
            while !list_payload.is_empty() {
                receipts.push(decode_list::<Receipt>(&mut list_payload)?);
            }
            receipts
        };

        Ok(Self::new(id, receipts))
    }
}
