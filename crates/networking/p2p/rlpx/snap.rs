use super::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::{BufMut, Bytes};
use ethrex_common::{
    H256, U256,
    types::{AccountState, AccountStateSlimCodec},
};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

// Snap Capability Messages

#[derive(Debug, Clone)]
pub struct GetAccountRange {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    pub id: u64,
    pub root_hash: H256,
    pub starting_hash: H256,
    pub limit_hash: H256,
    pub response_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct AccountRange {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    pub id: u64,
    pub accounts: Vec<AccountRangeUnit>,
    pub proof: Vec<Bytes>,
}

#[derive(Debug, Clone)]
pub struct GetStorageRanges {
    pub id: u64,
    pub root_hash: H256,
    pub account_hashes: Vec<H256>,
    pub starting_hash: H256,
    pub limit_hash: H256,
    pub response_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct StorageRanges {
    pub id: u64,
    pub slots: Vec<Vec<StorageSlot>>,
    pub proof: Vec<Bytes>,
}

#[derive(Debug, Clone)]
pub struct GetByteCodes {
    pub id: u64,
    pub hashes: Vec<H256>,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct ByteCodes {
    pub id: u64,
    pub codes: Vec<Bytes>,
}

#[derive(Debug, Clone)]
pub struct GetTrieNodes {
    pub id: u64,
    pub root_hash: H256,
    // [[acc_path, slot_path_1, slot_path_2,...]...]
    // The paths can be either full paths (hash) or only the partial path (compact-encoded nibbles)
    pub paths: Vec<Vec<Bytes>>,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct TrieNodes {
    pub id: u64,
    pub nodes: Vec<Bytes>,
}

impl RLPxMessage for GetAccountRange {
    const CODE: u8 = 0x00;
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
    const CODE: u8 = 0x01;
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
    const CODE: u8 = 0x02;
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
        let (starting_hash, decoder): (Bytes, _) = decoder.decode_field("startingHash")?;
        let starting_hash = if !starting_hash.is_empty() {
            H256::from_slice(&starting_hash)
        } else {
            Default::default()
        };
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
    const CODE: u8 = 0x03;
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
    const CODE: u8 = 0x04;
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
    const CODE: u8 = 0x05;
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
    const CODE: u8 = 0x06;
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
    const CODE: u8 = 0x07;
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

// Intermediate structures

#[derive(Debug, Clone)]
pub struct AccountRangeUnit {
    pub hash: H256,
    pub account: AccountState,
}

#[derive(Debug, Clone)]
pub struct StorageSlot {
    pub hash: H256,
    pub data: U256,
}

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
