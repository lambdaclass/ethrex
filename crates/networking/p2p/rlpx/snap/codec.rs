//! Snap protocol message encoding/decoding
//!
//! This module implements RLPxMessage for snap protocol messages,
//! as well as RLP encoding/decoding for helper types.

use super::messages::{
    AccountRange, AccountRangeUnit, BlockAccessLists, ByteCodes, GetAccountRange,
    GetBlockAccessLists, GetByteCodes, GetStorageRanges, GetTrieNodes, StorageRanges, StorageSlot,
    TrieNodes,
};
use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::{BufMut, Bytes};
use ethrex_common::{
    H256, U256,
    types::{AccountStateSlimCodec, block_access_list::BlockAccessList},
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
    /// snap/1 only — rejected on snap/2 connections.
    pub const GET_TRIE_NODES: u8 = 0x06;
    /// snap/1 only — rejected on snap/2 connections.
    pub const TRIE_NODES: u8 = 0x07;
    /// snap/2 only (EIP-8189) — rejected on snap/1 connections.
    pub const GET_BLOCK_ACCESS_LISTS: u8 = 0x08;
    /// snap/2 only (EIP-8189) — rejected on snap/1 connections.
    pub const BLOCK_ACCESS_LISTS: u8 = 0x09;
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
// snap/2 RLPX MESSAGE IMPLEMENTATIONS (EIP-8189)
// =============================================================================

impl RLPxMessage for GetBlockAccessLists {
    const CODE: u8 = codes::GET_BLOCK_ACCESS_LISTS;

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

impl RLPxMessage for BlockAccessLists {
    const CODE: u8 = codes::BLOCK_ACCESS_LISTS;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        // Wrap bals in the OptionBalList newtype for encoding; clones only on encode path.
        let bal_list = OptionBalList(self.bals.clone());
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&bal_list)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (bals, decoder) = decoder.decode_field::<OptionBalList>("bals")?;
        decoder.finish()?;

        Ok(Self { id, bals: bals.0 })
    }
}

/// Wrapper to enable RLP encode/decode for `Vec<Option<BlockAccessList>>`.
///
/// Encoding: `None` → RLP empty byte string (`0x80`),
///           `Some(bal)` → RLP-encoded `BlockAccessList` (a list).
struct OptionBalList(pub Vec<Option<BlockAccessList>>);

impl RLPEncode for OptionBalList {
    fn encode(&self, buf: &mut dyn BufMut) {
        let mut list_buf = Vec::new();
        for item in self.0.iter() {
            match item {
                None => {
                    // RLP empty byte string = single byte 0x80
                    list_buf.put_u8(0x80);
                }
                Some(bal) => {
                    bal.encode(&mut list_buf);
                }
            }
        }
        let len = list_buf.len();
        ethrex_rlp::encode::encode_length(len, buf);
        buf.put_slice(&list_buf);
    }
}

impl RLPDecode for OptionBalList {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        // The outer list length prefix
        let (_, payload, remaining) = ethrex_rlp::decode::decode_rlp_item(rlp)?;
        let mut bals: Vec<Option<BlockAccessList>> = Vec::new();
        let mut rest = payload;
        while !rest.is_empty() {
            // Peek: 0x80 is RLP empty byte string, used as the None sentinel
            if rest[0] == 0x80 {
                bals.push(None);
                rest = &rest[1..];
            } else {
                let (bal, tail) = BlockAccessList::decode_unfinished(rest)?;
                bals.push(Some(bal));
                rest = tail;
            }
        }
        Ok((OptionBalList(bals), remaining))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rlpx::snap::messages::{BlockAccessLists, GetBlockAccessLists};
    use ethrex_common::{
        Address, H256,
        types::block_access_list::{AccountChanges, BlockAccessList},
    };

    fn make_bal(address_byte: u8) -> BlockAccessList {
        let mut bal = BlockAccessList::new();
        bal.add_account_changes(AccountChanges::new(Address::from([address_byte; 20])));
        bal
    }

    fn round_trip_get(msg: GetBlockAccessLists) -> GetBlockAccessLists {
        let mut buf = vec![];
        msg.encode(&mut buf).unwrap();
        GetBlockAccessLists::decode(&buf).unwrap()
    }

    fn round_trip_resp(msg: BlockAccessLists) -> BlockAccessLists {
        let mut buf = vec![];
        msg.encode(&mut buf).unwrap();
        BlockAccessLists::decode(&buf).unwrap()
    }

    // Case 1: empty block_hashes request
    #[test]
    fn get_bal_empty_request() {
        let msg = GetBlockAccessLists {
            id: 1,
            block_hashes: vec![],
            response_bytes: 0,
        };
        let decoded = round_trip_get(msg.clone());
        assert_eq!(decoded.id, msg.id);
        assert!(decoded.block_hashes.is_empty());
        assert_eq!(decoded.response_bytes, msg.response_bytes);
    }

    // Case 2: 5-hash request with 2 MiB response_bytes
    #[test]
    fn get_bal_five_hashes() {
        let hashes: Vec<H256> = (0u8..5).map(|i| H256::from([i; 32])).collect();
        let msg = GetBlockAccessLists {
            id: 42,
            block_hashes: hashes.clone(),
            response_bytes: 2 * 1024 * 1024,
        };
        let decoded = round_trip_get(msg);
        assert_eq!(decoded.id, 42);
        assert_eq!(decoded.block_hashes, hashes);
        assert_eq!(decoded.response_bytes, 2 * 1024 * 1024);
    }

    // Case 3: response with all Some (fully populated)
    #[test]
    fn bal_response_all_some() {
        let bals: Vec<Option<BlockAccessList>> = (0u8..3).map(|i| Some(make_bal(i))).collect();
        let msg = BlockAccessLists { id: 7, bals };
        let decoded = round_trip_resp(msg.clone());
        assert_eq!(decoded.id, 7);
        assert_eq!(decoded.bals.len(), 3);
        for item in &decoded.bals {
            assert!(item.is_some());
        }
    }

    // Case 4: response with mixed availability (one None)
    #[test]
    fn bal_response_mixed_availability() {
        let bals = vec![Some(make_bal(1)), None, Some(make_bal(3))];
        let msg = BlockAccessLists { id: 99, bals };
        let decoded = round_trip_resp(msg);
        assert_eq!(decoded.id, 99);
        assert_eq!(decoded.bals.len(), 3);
        assert!(decoded.bals[0].is_some());
        assert!(decoded.bals[1].is_none());
        assert!(decoded.bals[2].is_some());
    }

    // Case 5: response with single BAL (even if oversize, one element is still returned)
    #[test]
    fn bal_response_single_element() {
        let bals = vec![Some(make_bal(0xAB))];
        let msg = BlockAccessLists { id: 1, bals };
        let decoded = round_trip_resp(msg);
        assert_eq!(decoded.bals.len(), 1);
        assert!(decoded.bals[0].is_some());
    }

    // Case 6: all-None response (empty prefix)
    #[test]
    fn bal_response_all_none() {
        let bals = vec![None, None, None];
        let msg = BlockAccessLists { id: 5, bals };
        let decoded = round_trip_resp(msg);
        assert_eq!(decoded.bals.len(), 3);
        for item in &decoded.bals {
            assert!(item.is_none());
        }
    }
}
