use bytes::{BufMut, Bytes};
use ethereum_rust_core::{
    types::{AccountState, EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    H256, U256,
};
use ethereum_rust_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use snap::raw::Decoder as SnappyDecoder;

use super::{message::RLPxMessage, utils::snappy_encode};

// Snap Capability Messages

#[derive(Debug)]
pub(crate) struct GetAccountRange {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    pub id: u64,
    pub root_hash: H256,
    pub starting_hash: H256,
    pub limit_hash: H256,
    pub response_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct AccountRange {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    pub id: u64,
    pub accounts: Vec<AccountRangeUnit>,
    pub proof: Vec<Bytes>,
}

impl RLPxMessage for GetAccountRange {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.root_hash)
            .encode_field(&self.starting_hash)
            .encode_field(&self.limit_hash)
            .encode_field(&self.response_bytes)
            .finish();

        let msg_data = snappy_encode(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let mut snappy_decoder = SnappyDecoder::new();
        let decompressed_data = snappy_decoder
            .decompress_vec(msg_data)
            .map_err(|e| RLPDecodeError::Custom(e.to_string()))?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (root_hash, decoder): (H256, _) = decoder.decode_field("rootHash")?;
        let (starting_hash, decoder): (H256, _) = decoder.decode_field("startingHash")?;
        let (limit_hash, decoder): (H256, _) = decoder.decode_field("limitHash")?;
        let (response_bytes, _): (u64, _) = decoder.decode_field("responseBytes")?;

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
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.accounts)
            .encode_field(&self.proof)
            .finish();

        let msg_data = snappy_encode(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let mut snappy_decoder = SnappyDecoder::new();
        let decompressed_data = snappy_decoder
            .decompress_vec(msg_data)
            .map_err(|e| RLPDecodeError::Custom(e.to_string()))?;
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

// Intermediate structures

#[derive(Debug)]
pub struct AccountRangeUnit {
    pub hash: H256,
    pub account: AccountStateSlim,
}

#[derive(Debug)]
pub struct AccountStateSlim {
    pub nonce: u64,
    pub balance: U256,
    pub storage_root: Bytes,
    pub code_hash: Bytes,
}

impl RLPEncode for AccountRangeUnit {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.hash)
            .encode_field(&self.account)
            .finish();
    }
}

impl RLPDecode for AccountRangeUnit {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (hash, decoder) = decoder.decode_field("hash")?;
        let (account, decoder) = decoder.decode_field("account")?;
        Ok((Self { hash, account }, decoder.finish()?))
    }
}

impl RLPEncode for AccountStateSlim {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.nonce)
            .encode_field(&self.balance)
            .encode_field(&self.storage_root)
            .encode_field(&self.code_hash)
            .finish();
    }
}

impl RLPDecode for AccountStateSlim {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (balance, decoder) = decoder.decode_field("balance")?;
        let (storage_root, decoder) = decoder.decode_field("storage_root")?;
        let (code_hash, decoder) = decoder.decode_field("code_hash")?;
        Ok((
            Self {
                nonce,
                balance,
                storage_root,
                code_hash,
            },
            decoder.finish()?,
        ))
    }
}

impl From<AccountState> for AccountStateSlim {
    fn from(value: AccountState) -> Self {
        let storage_root = if value.storage_root == *EMPTY_TRIE_HASH {
            Bytes::new()
        } else {
            Bytes::copy_from_slice(value.storage_root.as_bytes())
        };
        let code_hash = if value.code_hash == *EMPTY_KECCACK_HASH {
            Bytes::new()
        } else {
            Bytes::copy_from_slice(value.code_hash.as_bytes())
        };
        Self {
            nonce: value.nonce,
            balance: value.balance,
            storage_root,
            code_hash,
        }
    }
}

impl From<AccountStateSlim> for AccountState {
    fn from(value: AccountStateSlim) -> Self {
        let storage_root = if value.storage_root.is_empty() {
            *EMPTY_TRIE_HASH
        } else {
            H256::from_slice(value.storage_root.as_ref())
        };
        let code_hash = if value.code_hash.is_empty() {
            *EMPTY_KECCACK_HASH
        } else {
            H256::from_slice(value.code_hash.as_ref())
        };
        Self {
            nonce: value.nonce,
            balance: value.balance,
            storage_root,
            code_hash,
        }
    }
}
