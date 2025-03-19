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
#[serde(rename_all = "camelCase")]
pub struct AuthorizationTuple {
    pub chain_id: U256,
    pub address: Address,
    pub nonce: u64,
    pub y_parity: U256,
    #[serde(rename = "r")]
    pub r_signature: U256,
    #[serde(rename = "s")]
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
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (address, decoder) = decoder.decode_field("address")?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (y_parity, decoder) = decoder.decode_field("y_parity")?;
        let (r_signature, decoder) = decoder.decode_field("r_signature")?;
        let (s_signature, decoder) = decoder.decode_field("s_signature")?;
        let rest = decoder.finish()?;
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
