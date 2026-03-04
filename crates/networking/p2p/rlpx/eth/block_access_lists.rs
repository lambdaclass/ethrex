use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_common::types::BlockHash;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::{RLPEncode, encode_length},
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

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

/// RLP-encodes a list of optional BALs.
/// A missing BAL (`None`) is encoded as an empty list `[]` (0xc0).
/// A present BAL (`Some(bal)`) is encoded as its RLP list encoding.
fn encode_optional_bal_list(items: &[Option<BlockAccessList>], buf: &mut dyn BufMut) {
    // Calculate payload length
    let payload_len: usize = items
        .iter()
        .map(|opt| match opt {
            None => 1, // empty list = 0xc0, 1 byte
            Some(bal) => bal.length(),
        })
        .sum();
    encode_length(payload_len, buf);
    for opt in items {
        match opt {
            None => buf.put_u8(0xc0),
            Some(bal) => bal.encode(buf),
        }
    }
}

/// Decodes a list of optional BALs from RLP.
/// An empty list `[]` is decoded as `None`.
/// A non-empty list is decoded as `Some(BlockAccessList)`.
fn decode_optional_bal_list(data: &[u8]) -> Result<(Vec<Option<BlockAccessList>>, &[u8]), RLPDecodeError> {
    // The outer list is a sequence of items
    if data.is_empty() {
        return Err(RLPDecodeError::InvalidLength);
    }
    let first_byte = data[0];
    // Parse list prefix
    let (payload, rest) = if first_byte < 0xc0 {
        return Err(RLPDecodeError::MalformedData);
    } else if first_byte <= 0xf7 {
        // Short list
        let payload_len = (first_byte - 0xc0) as usize;
        if data.len() < 1 + payload_len {
            return Err(RLPDecodeError::InvalidLength);
        }
        (&data[1..1 + payload_len], &data[1 + payload_len..])
    } else {
        // Long list
        let len_of_len = (first_byte - 0xf7) as usize;
        if data.len() < 1 + len_of_len {
            return Err(RLPDecodeError::InvalidLength);
        }
        let mut payload_len = 0usize;
        for &b in &data[1..1 + len_of_len] {
            payload_len = (payload_len << 8) | b as usize;
        }
        if data.len() < 1 + len_of_len + payload_len {
            return Err(RLPDecodeError::InvalidLength);
        }
        (
            &data[1 + len_of_len..1 + len_of_len + payload_len],
            &data[1 + len_of_len + payload_len..],
        )
    };

    // Decode each item from payload
    let mut items = Vec::new();
    let mut remaining = payload;
    while !remaining.is_empty() {
        let first = *remaining.first().ok_or(RLPDecodeError::InvalidLength)?;
        if first < 0xc0 {
            return Err(RLPDecodeError::MalformedData);
        }
        // Determine the length of this list item
        let (item_len, header_len) = if first <= 0xf7 {
            ((first - 0xc0) as usize, 1usize)
        } else {
            let len_of_len = (first - 0xf7) as usize;
            if remaining.len() < 1 + len_of_len {
                return Err(RLPDecodeError::InvalidLength);
            }
            let mut payload_len = 0usize;
            for &b in &remaining[1..1 + len_of_len] {
                payload_len = (payload_len << 8) | b as usize;
            }
            (payload_len, 1 + len_of_len)
        };
        let total_len = header_len + item_len;
        if remaining.len() < total_len {
            return Err(RLPDecodeError::InvalidLength);
        }
        let item_data = &remaining[..total_len];
        remaining = &remaining[total_len..];

        if item_len == 0 {
            // Empty list = None (BAL unavailable for this block)
            items.push(None);
        } else {
            // Non-empty list = Some(BlockAccessList)
            let (bal, _) = BlockAccessList::decode_unfinished(item_data)?;
            items.push(Some(bal));
        }
    }

    Ok((items, rest))
}

impl RLPxMessage for BlockAccessLists {
    const CODE: u8 = 0x13;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        // Manually encode outer list: [id, [bal_or_empty, ...]]
        let id_encoded = {
            let mut v = vec![];
            self.id.encode(&mut v);
            v
        };
        let bals_encoded = {
            let mut v = vec![];
            encode_optional_bal_list(&self.block_access_lists, &mut v);
            v
        };
        // Outer RLP list payload = id_encoded + bals_encoded
        let payload_len = id_encoded.len() + bals_encoded.len();
        encode_length(payload_len, &mut encoded_data);
        encoded_data.extend_from_slice(&id_encoded);
        encoded_data.extend_from_slice(&bals_encoded);

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        // The remaining data in the decoder is the bals list
        // We need to decode the rest manually
        let remaining = decoder.finish()?;
        let (block_access_lists, _) = decode_optional_bal_list(remaining)?;
        Ok(Self::new(id, block_access_lists))
    }
}
