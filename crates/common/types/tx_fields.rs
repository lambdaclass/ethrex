use crate::{Address, H256, U256};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use serde::{Deserialize, Serialize};

pub type AccessList = Vec<AccessListItem>;
pub type AccessListItem = (Address, Vec<H256>);

pub type AuthorizationList = Vec<AuthorizationTuple>;
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AuthorizationTuple {
    pub chain_id: U256,
    pub address: Address,
    pub nonce: u64,
    pub y_parity: U256,
    pub r_signature: U256,
    pub s_signature: U256,
}

impl RLPEncode for AuthorizationTuple {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.chain_id)
            .encode_field(&self.address)
            .encode_field(&self.nonce)
            .encode_field(&self.y_parity)
            .encode_field(&self.r_signature)
            .encode_field(&self.s_signature)
            .finish();
    }
}

impl RLPDecode for AuthorizationTuple {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp).unwrap();
        let (chain_id, decoder) = decoder.decode_field("chain_id").unwrap();
        let (address, decoder) = decoder.decode_field("address").unwrap();
        let (nonce, decoder) = decoder.decode_field("nonce").unwrap();
        let (y_parity, decoder) = decoder.decode_field("y_parity").unwrap();
        let (r_signature, decoder) = decoder.decode_field("r_signature").unwrap();
        let (s_signature, decoder) = decoder.decode_field("s_signature").unwrap();
        let rest = decoder.finish().unwrap();
        Ok((
            AuthorizationTuple {
                chain_id,
                address,
                nonce,
                y_parity,
                r_signature,
                s_signature,
            },
            rest,
        ))
    }
}
