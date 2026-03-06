use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::types::BlockHash;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

/// Maximum number of BALs to serve per request (same as block bodies limit in geth).
pub const BLOCK_ACCESS_LIST_LIMIT: usize = 1024;

/// Wrapper for optional BAL in eth/71 protocol messages.
/// `None` (BAL unavailable) is encoded as an empty RLP list (0xc0).
/// `Some(bal)` is encoded as the BAL's normal RLP list encoding.
#[derive(Debug, Clone)]
struct OptionalBal(Option<BlockAccessList>);

impl RLPEncode for OptionalBal {
    fn encode(&self, buf: &mut dyn BufMut) {
        match &self.0 {
            None => buf.put_u8(0xc0),
            Some(bal) => bal.encode(buf),
        }
    }

    fn length(&self) -> usize {
        match &self.0 {
            None => 1, // empty list = 0xc0
            Some(bal) => bal.length(),
        }
    }
}

impl RLPDecode for OptionalBal {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.first() == Some(&0xc0) {
            return Ok((OptionalBal(None), &rlp[1..]));
        }
        let (bal, rest) = BlockAccessList::decode_unfinished(rlp)?;
        Ok((OptionalBal(Some(bal)), rest))
    }
}

// https://eips.ethereum.org/EIPS/eip-8159 (eth/71 BAL exchange)
#[derive(Debug, Clone)]
pub struct GetBlockAccessLists {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub block_hashes: Vec<BlockHash>,
}

impl GetBlockAccessLists {
    pub fn new(id: u64, block_hashes: Vec<BlockHash>) -> Self {
        Self { id, block_hashes }
    }
}

impl RLPxMessage for GetBlockAccessLists {
    const CODE: u8 = 0x12;

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

// https://eips.ethereum.org/EIPS/eip-8159 (eth/71 BAL exchange)
#[derive(Debug, Clone)]
pub struct BlockAccessLists {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    /// One entry per requested block hash. `None` means the BAL is unavailable for that block.
    pub block_access_lists: Vec<Option<BlockAccessList>>,
}

impl BlockAccessLists {
    pub fn new(id: u64, block_access_lists: Vec<Option<BlockAccessList>>) -> Self {
        Self {
            id,
            block_access_lists,
        }
    }
}

impl RLPxMessage for BlockAccessLists {
    const CODE: u8 = 0x13;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        let bals: Vec<OptionalBal> = self
            .block_access_lists
            .iter()
            .cloned()
            .map(OptionalBal)
            .collect();
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&bals)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (bals, decoder): (Vec<OptionalBal>, _) =
            decoder.decode_field("blockAccessLists")?;
        decoder.finish()?;
        let block_access_lists = bals.into_iter().map(|b| b.0).collect();
        Ok(Self::new(id, block_access_lists))
    }
}
