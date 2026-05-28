//! Snap protocol message encoding/decoding
//!
//! This module implements RLPxMessage for snap protocol messages,
//! as well as RLP encoding/decoding for helper types.

use super::messages::{
    AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges,
    GetTrieNodes, Snap2BlockAccessLists, Snap2GetBlockAccessLists, StorageRanges, StorageSlot,
    TrieNodes,
};
use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::{BufMut, Bytes};
use ethrex_common::{
    H256, U256, types::AccountStateSlimCodec, types::block_access_list::BlockAccessList,
};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

// =============================================================================
// MESSAGE CODES
// =============================================================================

/// Snap protocol message codes
pub mod codes {
    pub const GET_ACCOUNT_RANGE: u8 = 0x00;
    pub const ACCOUNT_RANGE: u8 = 0x01;
    pub const GET_STORAGE_RANGES: u8 = 0x02;
    pub const STORAGE_RANGES: u8 = 0x03;
    pub const GET_BYTE_CODES: u8 = 0x04;
    pub const BYTE_CODES: u8 = 0x05;
    pub const GET_TRIE_NODES: u8 = 0x06;
    pub const TRIE_NODES: u8 = 0x07;
    /// snap/2 only (EIP-8189).
    pub const SNAP2_GET_BLOCK_ACCESS_LISTS: u8 = 0x08;
    /// snap/2 only (EIP-8189).
    pub const SNAP2_BLOCK_ACCESS_LISTS: u8 = 0x09;
}

// =============================================================================
// RLPX MESSAGE IMPLEMENTATIONS
// =============================================================================

impl RLPxMessage for GetAccountRange {
    const CODE: u8 = codes::GET_ACCOUNT_RANGE;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.root_hash)
            .encode_field(&self.starting_hash)
            .encode_field(&self.limit_hash)
            .encode_field(&self.response_bytes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (root_hash, decoder) = decoder.decode_field("rootHash")?;
        let (starting_hash, decoder) = decoder.decode_field("startingHash")?;
        let (limit_hash, decoder) = decoder.decode_field("limitHash")?;
        let (response_bytes, decoder) = decoder.decode_field("responseBytes")?;
        decoder.finish()?;

        Ok(Self {
            id,
            root_hash,
            starting_hash,
            limit_hash,
            response_bytes,
        })
    }
}

impl RLPxMessage for AccountRange {
    const CODE: u8 = codes::ACCOUNT_RANGE;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.accounts)
            .encode_field(&self.proof)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (accounts, decoder) = decoder.decode_field("accounts")?;
        let (proof, decoder) = decoder.decode_field("proof")?;
        decoder.finish()?;

        Ok(Self {
            id,
            accounts,
            proof,
        })
    }
}

impl RLPxMessage for GetStorageRanges {
    const CODE: u8 = codes::GET_STORAGE_RANGES;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.root_hash)
            .encode_field(&self.account_hashes)
            .encode_field(&self.starting_hash)
            .encode_field(&self.limit_hash)
            .encode_field(&self.response_bytes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (root_hash, decoder) = decoder.decode_field("rootHash")?;
        let (account_hashes, decoder) = decoder.decode_field("accountHashes")?;
        // Handle empty starting_hash as default (zero hash)
        let (starting_hash, decoder): (Bytes, _) = decoder.decode_field("startingHash")?;
        let starting_hash = if !starting_hash.is_empty() {
            H256::from_slice(&starting_hash)
        } else {
            Default::default()
        };
        // Handle empty limit_hash as max hash
        let (limit_hash, decoder): (Bytes, _) = decoder.decode_field("limitHash")?;
        let limit_hash = if !limit_hash.is_empty() {
            H256::from_slice(&limit_hash)
        } else {
            H256([0xFF; 32])
        };
        let (response_bytes, decoder) = decoder.decode_field("responseBytes")?;
        decoder.finish()?;

        Ok(Self {
            id,
            root_hash,
            starting_hash,
            account_hashes,
            limit_hash,
            response_bytes,
        })
    }
}

impl RLPxMessage for StorageRanges {
    const CODE: u8 = codes::STORAGE_RANGES;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.slots)
            .encode_field(&self.proof)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (slots, decoder) = decoder.decode_field("slots")?;
        let (proof, decoder) = decoder.decode_field("proof")?;
        decoder.finish()?;

        Ok(Self { id, slots, proof })
    }
}

impl RLPxMessage for GetByteCodes {
    const CODE: u8 = codes::GET_BYTE_CODES;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.hashes)
            .encode_field(&self.bytes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (hashes, decoder) = decoder.decode_field("hashes")?;
        let (bytes, decoder) = decoder.decode_field("bytes")?;
        decoder.finish()?;

        Ok(Self { id, hashes, bytes })
    }
}

impl RLPxMessage for ByteCodes {
    const CODE: u8 = codes::BYTE_CODES;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.codes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (codes, decoder) = decoder.decode_field("codes")?;
        decoder.finish()?;

        Ok(Self { id, codes })
    }
}

impl RLPxMessage for GetTrieNodes {
    const CODE: u8 = codes::GET_TRIE_NODES;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.root_hash)
            .encode_field(&self.paths)
            .encode_field(&self.bytes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (root_hash, decoder) = decoder.decode_field("root_hash")?;
        let (paths, decoder) = decoder.decode_field("paths")?;
        let (bytes, decoder) = decoder.decode_field("bytes")?;
        decoder.finish()?;

        Ok(Self {
            id,
            root_hash,
            paths,
            bytes,
        })
    }
}

impl RLPxMessage for TrieNodes {
    const CODE: u8 = codes::TRIE_NODES;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.nodes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (nodes, decoder) = decoder.decode_field("nodes")?;
        decoder.finish()?;

        Ok(Self { id, nodes })
    }
}

// =============================================================================
// snap/2 CODEC (EIP-8189)
// =============================================================================

impl RLPxMessage for Snap2GetBlockAccessLists {
    const CODE: u8 = codes::SNAP2_GET_BLOCK_ACCESS_LISTS;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.block_hashes)
            .encode_field(&self.response_bytes)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (block_hashes, decoder) = decoder.decode_field("blockHashes")?;
        let (response_bytes, decoder) = decoder.decode_field("responseBytes")?;
        decoder.finish()?;
        Ok(Self {
            id,
            block_hashes,
            response_bytes,
        })
    }
}

/// Per-slot wrapper for `Option<BlockAccessList>` in snap/2 responses.
///
/// Wire format per EIP-8189 §50, §58:
///   `None` → RLP empty string (0x80)
///   `Some(bal)` → RLP-encoded `BlockAccessList`
#[derive(Debug, Clone)]
struct Snap2OptionalBal(Option<BlockAccessList>);

impl RLPEncode for Snap2OptionalBal {
    fn encode(&self, buf: &mut dyn BufMut) {
        match &self.0 {
            None => buf.put_u8(0x80), // RLP empty string per EIP-8189 §50,§58
            Some(bal) => bal.encode(buf),
        }
    }

    fn length(&self) -> usize {
        match &self.0 {
            None => 1,
            Some(bal) => bal.length(),
        }
    }
}

impl RLPDecode for Snap2OptionalBal {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.first() == Some(&0x80) {
            return Ok((Snap2OptionalBal(None), &rlp[1..]));
        }
        let (bal, rest) = BlockAccessList::decode_unfinished(rlp)?;
        Ok((Snap2OptionalBal(Some(bal)), rest))
    }
}

impl RLPxMessage for Snap2BlockAccessLists {
    const CODE: u8 = codes::SNAP2_BLOCK_ACCESS_LISTS;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        let bals: Vec<Snap2OptionalBal> = self.bals.iter().cloned().map(Snap2OptionalBal).collect();
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
        let (bals, decoder): (Vec<Snap2OptionalBal>, _) = decoder.decode_field("bals")?;
        decoder.finish()?;
        Ok(Self {
            id,
            bals: bals.into_iter().map(|b| b.0).collect(),
        })
    }
}

// =============================================================================
// RLP IMPLEMENTATIONS FOR HELPER TYPES
// =============================================================================

impl RLPEncode for AccountRangeUnit {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.hash)
            .encode_field(&AccountStateSlimCodec(self.account))
            .finish();
    }
}

impl RLPDecode for AccountRangeUnit {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (hash, decoder) = decoder.decode_field("hash")?;
        let (AccountStateSlimCodec(account), decoder) =
            decoder.decode_field::<AccountStateSlimCodec>("account")?;
        Ok((Self { hash, account }, decoder.finish()?))
    }
}

impl RLPEncode for StorageSlot {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.hash)
            .encode_bytes(&self.data.encode_to_vec())
            .finish();
    }
}

impl RLPDecode for StorageSlot {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (hash, decoder) = decoder.decode_field("hash")?;
        let (data, decoder) = decoder.get_encoded_item()?;
        let data = U256::decode(ethrex_rlp::decode::decode_bytes(&data)?.0)?;
        Ok((Self { hash, data }, decoder.finish()?))
    }
}
