//! Snap protocol message encoding/decoding
//!
//! This module implements RLPxMessage for snap protocol messages,
//! as well as RLP encoding/decoding for helper types.

use super::messages::{
    AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges,
    GetTrieNodes, StorageRanges, StorageSlot, TrieNodes,
};
use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::Bytes;
use ethrex_common::{H256, U256, types::AccountStateSlimCodec};
use librlp::{Header, RlpBuf, RlpDecode, RlpEncode, RlpError};

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
}

// =============================================================================
// RLPX MESSAGE IMPLEMENTATIONS
// =============================================================================

impl RLPxMessage for GetAccountRange {
    const CODE: u8 = codes::GET_ACCOUNT_RANGE;

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.id.encode(buf);
            self.root_hash.encode(buf);
            self.starting_hash.encode(buf);
            self.limit_hash.encode(buf);
            self.response_bytes.encode(buf);
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
        let root_hash = H256::decode(&mut payload)?;
        let starting_hash = H256::decode(&mut payload)?;
        let limit_hash = H256::decode(&mut payload)?;
        let response_bytes = u64::decode(&mut payload)?;

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

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.id.encode(buf);
            librlp::encode_list(&self.accounts, buf);
            librlp::encode_list(&self.proof, buf);
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
        let accounts: Vec<AccountRangeUnit> = librlp::decode_list(&mut payload)?;
        let proof: Vec<Bytes> = librlp::decode_list(&mut payload)?;

        Ok(Self {
            id,
            accounts,
            proof,
        })
    }
}

impl RLPxMessage for GetStorageRanges {
    const CODE: u8 = codes::GET_STORAGE_RANGES;

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.id.encode(buf);
            self.root_hash.encode(buf);
            librlp::encode_list(&self.account_hashes, buf);
            self.starting_hash.encode(buf);
            self.limit_hash.encode(buf);
            self.response_bytes.encode(buf);
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
        let root_hash = H256::decode(&mut payload)?;
        let account_hashes: Vec<H256> = librlp::decode_list(&mut payload)?;
        // Handle empty starting_hash as default (zero hash)
        let starting_hash_bytes = Bytes::decode(&mut payload)?;
        let starting_hash = if !starting_hash_bytes.is_empty() {
            H256::from_slice(&starting_hash_bytes)
        } else {
            Default::default()
        };
        // Handle empty limit_hash as max hash
        let limit_hash_bytes = Bytes::decode(&mut payload)?;
        let limit_hash = if !limit_hash_bytes.is_empty() {
            H256::from_slice(&limit_hash_bytes)
        } else {
            H256([0xFF; 32])
        };
        let response_bytes = u64::decode(&mut payload)?;

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

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.id.encode(buf);
            // Vec<Vec<StorageSlot>>: encode as list of lists
            buf.list(|buf| {
                for inner in &self.slots {
                    librlp::encode_list(inner, buf);
                }
            });
            librlp::encode_list(&self.proof, buf);
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
        // Decode Vec<Vec<StorageSlot>>: outer list of inner lists
        let outer_header = Header::decode(&mut payload)?;
        if !outer_header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut outer_payload = &payload[..outer_header.payload_length];
        payload = &payload[outer_header.payload_length..];
        let mut slots = Vec::new();
        while !outer_payload.is_empty() {
            let inner: Vec<StorageSlot> = librlp::decode_list(&mut outer_payload)?;
            slots.push(inner);
        }
        let proof: Vec<Bytes> = librlp::decode_list(&mut payload)?;

        Ok(Self { id, slots, proof })
    }
}

impl RLPxMessage for GetByteCodes {
    const CODE: u8 = codes::GET_BYTE_CODES;

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.id.encode(buf);
            librlp::encode_list(&self.hashes, buf);
            self.bytes.encode(buf);
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
        let hashes: Vec<H256> = librlp::decode_list(&mut payload)?;
        let bytes = u64::decode(&mut payload)?;

        Ok(Self { id, hashes, bytes })
    }
}

impl RLPxMessage for ByteCodes {
    const CODE: u8 = codes::BYTE_CODES;

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.id.encode(buf);
            librlp::encode_list(&self.codes, buf);
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
        let codes: Vec<Bytes> = librlp::decode_list(&mut payload)?;

        Ok(Self { id, codes })
    }
}

impl RLPxMessage for GetTrieNodes {
    const CODE: u8 = codes::GET_TRIE_NODES;

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.id.encode(buf);
            self.root_hash.encode(buf);
            // Vec<Vec<Bytes>>: encode as list of lists
            buf.list(|buf| {
                for inner in &self.paths {
                    librlp::encode_list(inner, buf);
                }
            });
            self.bytes.encode(buf);
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
        let root_hash = H256::decode(&mut payload)?;
        // Decode Vec<Vec<Bytes>>: outer list of inner lists
        let outer_header = Header::decode(&mut payload)?;
        if !outer_header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut outer_payload = &payload[..outer_header.payload_length];
        payload = &payload[outer_header.payload_length..];
        let mut paths = Vec::new();
        while !outer_payload.is_empty() {
            let inner: Vec<Bytes> = librlp::decode_list(&mut outer_payload)?;
            paths.push(inner);
        }
        let bytes = u64::decode(&mut payload)?;

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

    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.id.encode(buf);
            librlp::encode_list(&self.nodes, buf);
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
        let nodes: Vec<Bytes> = librlp::decode_list(&mut payload)?;

        Ok(Self { id, nodes })
    }
}

// =============================================================================
// RLP IMPLEMENTATIONS FOR HELPER TYPES
// =============================================================================

impl RlpEncode for AccountRangeUnit {
    fn encode(&self, buf: &mut RlpBuf) {
        buf.list(|buf| {
            self.hash.encode(buf);
            AccountStateSlimCodec(self.account).encode(buf);
        });
    }

    fn encoded_length(&self) -> usize {
        let mut buf = RlpBuf::new();
        self.encode(&mut buf);
        buf.finish().len()
    }
}

impl RlpDecode for AccountRangeUnit {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        *buf = &buf[header.payload_length..];
        let hash = H256::decode(&mut payload)?;
        let AccountStateSlimCodec(account) = AccountStateSlimCodec::decode(&mut payload)?;
        Ok(Self { hash, account })
    }
}

impl RlpEncode for StorageSlot {
    fn encode(&self, buf: &mut RlpBuf) {
        buf.list(|buf| {
            self.hash.encode(buf);
            // Encode data as bytes wrapping the RLP encoding of U256
            let data_rlp = self.data.to_rlp();
            data_rlp.encode(buf);
        });
    }

    fn encoded_length(&self) -> usize {
        let mut buf = RlpBuf::new();
        self.encode(&mut buf);
        buf.finish().len()
    }
}

impl RlpDecode for StorageSlot {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        *buf = &buf[header.payload_length..];
        let hash = H256::decode(&mut payload)?;
        // Decode the bytes wrapper, then decode U256 from the inner bytes
        let data_bytes = Bytes::decode(&mut payload)?;
        let data = U256::decode(&mut data_bytes.as_ref())?;
        Ok(Self { hash, data })
    }
}
