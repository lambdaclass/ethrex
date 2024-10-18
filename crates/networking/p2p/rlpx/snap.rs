use bytes::BufMut;
use ethereum_rust_core::{types::AccountState, H256};
use ethereum_rust_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use snap::raw::Decoder as SnappyDecoder;

use super::{message::RLPxMessage, utils::snappy_encode};

// https://github.com/ethereum/devp2p/blob/master/caps/snap.md#getaccountrange-0x00
#[derive(Debug)]
pub(crate) struct GetAccountRange {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    id: u64,
    root_hash: H256,
    starting_hash: H256,
    limit_hash: H256,
    response_bytes: u64,
}

impl GetAccountRange {
    pub fn new(
        id: u64,
        root_hash: H256,
        starting_hash: H256,
        limit_hash: H256,
        response_bytes: u64,
    ) -> Self {
        Self {
            id,
            root_hash,
            starting_hash,
            limit_hash,
            response_bytes,
        }
    }
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

        Ok(Self::new(
            id,
            root_hash,
            starting_hash,
            limit_hash,
            response_bytes,
        ))
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/snap.md#accountrange-0x01
#[derive(Debug)]
pub(crate) struct AccountRange {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    id: u64,
    accounts: Vec<(H256, AccountState)>,
    proof: Vec<u8>,
}

impl AccountRange {
    pub fn new(id: u64, accounts: Vec<(H256, AccountState)>, proof: Vec<u8>) -> Self {
        Self {
            id,
            accounts,
            proof,
        }
    }
}

impl RLPxMessage for AccountRange {
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_slim_field(&self.accounts)
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
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (accounts, decoder): (Vec<(H256, AccountState)>, _) =
            decoder.decode_field("accounts")?;
        let (proof, _): (Vec<u8>, _) = decoder.decode_field("proof")?;

        Ok(Self::new(id, accounts, proof))
    }
}
