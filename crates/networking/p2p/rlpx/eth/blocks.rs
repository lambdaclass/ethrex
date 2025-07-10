use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::types::{BlockBody, BlockHash, BlockHeader, BlockNumber};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use ethrex_storage::Store;
#[cfg(feature = "sync-test")]
use std::env;
use tracing::error;

pub const HASH_FIRST_BYTE_DECODER: u8 = 160;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum HashOrNumber {
    Hash(BlockHash),
    Number(BlockNumber),
}

impl RLPEncode for HashOrNumber {
    fn encode(&self, buf: &mut dyn BufMut) {
        match self {
            HashOrNumber::Hash(hash) => hash.encode(buf),
            HashOrNumber::Number(number) => number.encode(buf),
        }
    }

    fn length(&self) -> usize {
        match self {
            HashOrNumber::Hash(hash) => hash.length(),
            HashOrNumber::Number(number) => number.length(),
        }
    }
}

impl From<BlockHash> for HashOrNumber {
    fn from(value: BlockHash) -> Self {
        Self::Hash(value)
    }
}

impl RLPDecode for HashOrNumber {
    fn decode_unfinished(buf: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let first_byte = buf.first().ok_or(RLPDecodeError::InvalidLength)?;
        // https://ethereum.org/en/developers/docs/data-structures-and-encoding/rlp/
        // hashes are 32 bytes long, so they enter in the 0-55 bytes range for rlp. This means the first byte
        // is the value 0x80 + len, where len = 32 (0x20). so we get the result of 0xa0 which is 160 in decimal
        if *first_byte == HASH_FIRST_BYTE_DECODER {
            let (hash, rest) = BlockHash::decode_unfinished(buf)?;
            return Ok((Self::Hash(hash), rest));
        }

        let (number, rest) = u64::decode_unfinished(buf)?;
        Ok((Self::Number(number), rest))
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#getblockheaders-0x03
#[derive(Debug, Clone)]
pub struct GetBlockHeaders {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub startblock: HashOrNumber,
    pub limit: u64,
    pub skip: u64,
    pub reverse: bool,
}

// Limit taken from here: https://github.com/ethereum/go-ethereum/blob/20bf543a64d7c2a590b18a1e1d907cae65707013/eth/protocols/eth/handler.go#L40
pub const BLOCK_HEADER_LIMIT: u64 = 2;

impl GetBlockHeaders {
    pub fn new(id: u64, startblock: HashOrNumber, limit: u64, skip: u64, reverse: bool) -> Self {
        Self {
            id,
            startblock,
            limit,
            skip,
            reverse,
        }
    }
    pub async fn fetch_headers(&self, storage: &Store) -> Vec<BlockHeader> {
        let start_block = match self.startblock {
            // Check we have the given block hash and fetch its number
            HashOrNumber::Hash(block_hash) => {
                // TODO(#1073)
                // Research what we should do when an error is found in a P2P request.
                if let Ok(Some(block_number)) = storage.get_block_number(block_hash).await {
                    block_number
                } else {
                    error!("Could not fetch block number for hash {block_hash}");
                    return vec![];
                }
            }
            // Don't check if the block number is available
            // because if it it's not, the loop below will
            // break early and return an empty vec.
            HashOrNumber::Number(block_num) => block_num,
        };

        let mut headers = vec![];

        let mut current_block = start_block as i64;
        let block_skip = if self.reverse {
            -((self.skip + 1) as i64)
        } else {
            (self.skip + 1) as i64
        };

        #[cfg(feature = "sync-test")]
        let limit = if let Ok(env_var_block_limit) = env::var("BLOCK_HEADER_LIMIT") {
            env_var_block_limit
                .parse()
                .expect("Block header limit environmental variable is not a number")
        } else if self.limit > BLOCK_HEADER_LIMIT {
            BLOCK_HEADER_LIMIT
        } else {
            self.limit
        };

        #[cfg(not(feature = "sync-test"))]
        let limit = if self.limit > BLOCK_HEADER_LIMIT {
            BLOCK_HEADER_LIMIT
        } else {
            self.limit
        };

        for _ in 0..limit {
            match storage.get_block_header(current_block as u64) {
                Ok(Some(block_header)) => {
                    headers.push(block_header);
                    current_block += block_skip
                }
                Ok(None) => {
                    break;
                }
                // TODO(#1073)
                // Research what we should do when an error is found in a P2P request.
                Err(err) => {
                    error!("Error accessing DB while building header response for peer: {err}");
                    return vec![];
                }
            }
        }
        headers
    }
}

impl RLPxMessage for GetBlockHeaders {
    const CODE: u8 = 0x03;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        let limit = self.limit;
        let skip = self.skip;
        let reverse = self.reverse as u8;
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&(self.startblock.clone(), limit, skip, reverse))
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let ((start_block, limit, skip, reverse), _): ((HashOrNumber, u64, u64, bool), _) =
            decoder.decode_field("get headers request params")?;
        Ok(Self::new(id, start_block, limit, skip, reverse))
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#blockheaders-0x04
#[derive(Debug, Clone)]
pub struct BlockHeaders {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub block_headers: Vec<BlockHeader>,
}

impl BlockHeaders {
    pub fn new(id: u64, block_headers: Vec<BlockHeader>) -> Self {
        Self { block_headers, id }
    }
}

impl RLPxMessage for BlockHeaders {
    const CODE: u8 = 0x04;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        // Each message is encoded with its own
        // message identifier (code).
        // Go ethereum reference: https://github.com/ethereum/go-ethereum/blob/20bf543a64d7c2a590b18a1e1d907cae65707013/p2p/transport.go#L94
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.block_headers)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (block_headers, _): (Vec<BlockHeader>, _) = decoder.decode_field("headers")?;

        Ok(Self::new(id, block_headers))
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#getblockbodies-0x05
#[derive(Debug, Clone)]
pub struct GetBlockBodies {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub block_hashes: Vec<BlockHash>,
}

// Limit taken from here:
// https://github.com/ethereum/go-ethereum/blob/a1093d98eb3260f2abf340903c2d968b2b891c11/eth/protocols/eth/handler.go#L45
pub const BLOCK_BODY_LIMIT: usize = 1024;

impl GetBlockBodies {
    pub fn new(id: u64, block_hashes: Vec<BlockHash>) -> Self {
        Self { block_hashes, id }
    }
    pub async fn fetch_blocks(&self, storage: &Store) -> Vec<BlockBody> {
        let mut block_bodies = vec![];
        for block_hash in &self.block_hashes {
            match storage.get_block_body_by_hash(*block_hash).await {
                Ok(Some(block)) => {
                    block_bodies.push(block);
                    if block_bodies.len() >= BLOCK_BODY_LIMIT {
                        break;
                    }
                }
                Ok(None) => {
                    continue;
                }
                Err(err) => {
                    error!(
                        "Error accessing DB while building block bodies response for peer: {err}"
                    );
                    return vec![];
                }
            }
        }
        block_bodies
    }
}

impl RLPxMessage for GetBlockBodies {
    const CODE: u8 = 0x05;
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

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#blockbodies-0x06
#[derive(Debug, Clone)]
pub struct BlockBodies {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub block_bodies: Vec<BlockBody>,
}

impl BlockBodies {
    pub fn new(id: u64, block_bodies: Vec<BlockBody>) -> Self {
        Self { block_bodies, id }
    }
}

impl RLPxMessage for BlockBodies {
    const CODE: u8 = 0x06;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.block_bodies)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (block_bodies, _): (Vec<BlockBody>, _) = decoder.decode_field("blockBodies")?;

        Ok(Self::new(id, block_bodies))
    }
}

#[cfg(test)]
mod tests {
    use ethrex_common::types::BlockHash;

    use crate::rlpx::{
        eth::blocks::{BlockBodies, GetBlockBodies, GetBlockHeaders},
        message::RLPxMessage,
    };

    use super::HashOrNumber;

    #[test]
    fn get_block_headers_startblock_number_message() {
        let get_block_bodies = GetBlockHeaders::new(1, HashOrNumber::Number(1), 0, 0, false);

        let mut buf = Vec::new();
        get_block_bodies.encode(&mut buf).unwrap();

        let decoded = GetBlockHeaders::decode(&buf).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.startblock, HashOrNumber::Number(1));
    }

    #[test]
    fn get_block_headers_startblock_hash_message() {
        let get_block_bodies =
            GetBlockHeaders::new(1, HashOrNumber::Hash(BlockHash::from([1; 32])), 0, 0, false);

        let mut buf = Vec::new();
        get_block_bodies.encode(&mut buf).unwrap();

        let decoded = GetBlockHeaders::decode(&buf).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(
            decoded.startblock,
            HashOrNumber::Hash(BlockHash::from([1; 32]))
        );
    }

    #[test]
    fn get_block_bodies_empty_message() {
        let blocks_hash = vec![];
        let get_block_bodies = GetBlockBodies::new(1, blocks_hash.clone());

        let mut buf = Vec::new();
        get_block_bodies.encode(&mut buf).unwrap();

        let decoded = GetBlockBodies::decode(&buf).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.block_hashes, blocks_hash);
    }

    #[test]
    fn get_block_bodies_not_empty_message() {
        let blocks_hash = vec![
            BlockHash::from([0; 32]),
            BlockHash::from([1; 32]),
            BlockHash::from([2; 32]),
        ];
        let get_block_bodies = GetBlockBodies::new(1, blocks_hash.clone());

        let mut buf = Vec::new();
        get_block_bodies.encode(&mut buf).unwrap();

        let decoded = GetBlockBodies::decode(&buf).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.block_hashes, blocks_hash);
    }

    #[test]
    fn block_bodies_empty_message() {
        let block_bodies = vec![];
        let block_bodies = BlockBodies::new(1, block_bodies);

        let mut buf = Vec::new();
        block_bodies.encode(&mut buf).unwrap();

        let decoded = BlockBodies::decode(&buf).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.block_bodies, vec![]);
    }
}
