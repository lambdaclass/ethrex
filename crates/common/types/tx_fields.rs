use crate::{Address, H256, U256};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
/// A list of addresses and storage keys that the transaction plans to access.
/// See [EIP-2930](https://eips.ethereum.org/EIPS/eip-2930)
pub type AccessList = Vec<AccessListItem>;
pub type AccessListItem = (Address, Vec<H256>);

/// Used in Type-4 transactions. Added in [EIP-7702](https://eips.ethereum.org/EIPS/eip-7702)
pub type AuthorizationList = Vec<AuthorizationTuple>;
#[derive(
    Debug,
    Clone,
    Default,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    RSerialize,
    RDeserialize,
    Archive,
)]
#[serde(rename_all = "camelCase")]
/// Used in Type-4 transactions. Added in [EIP-7702](https://eips.ethereum.org/EIPS/eip-7702)
pub struct AuthorizationTuple {
    #[rkyv(with = crate::rkyv_utils::U256Wrapper)]
    pub chain_id: U256,
    #[rkyv(with = crate::rkyv_utils::H160Wrapper)]
    pub address: Address,
    #[serde(
        default,
        deserialize_with = "crate::serde_utils::u64::deser_hex_or_dec_str"
    )]
    pub nonce: u64,
    // EIP-7702 bounds y_parity to < 2**8; a u8 makes that a type invariant and lets
    // RLP decoding reject any out-of-range value, matching geth's `uint8`.
    pub y_parity: u8,
    #[serde(rename = "r")]
    #[rkyv(with = crate::rkyv_utils::U256Wrapper)]
    pub r_signature: U256,
    #[serde(rename = "s")]
    #[rkyv(with = crate::rkyv_utils::U256Wrapper)]
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

#[cfg(test)]
mod eip7702_y_parity_tests {
    //! Regression test for the `eip7702-auth-y-parity-bound` finding. EIP-7702 bounds
    //! an authorization tuple's `y_parity` to `< 2**8`; geth models it as a `u8` and
    //! rejects an out-of-range value at RLP decode. ethrex must do the same, otherwise
    //! an L1 block carrying a type-4 transaction whose `y_parity >= 256` is accepted
    //! here but rejected by other clients (consensus split). The authorization-tuple
    //! decode is the chokepoint: a type-4 transaction decodes its `authorization_list`
    //! through it.
    use super::*;

    #[test]
    fn authorization_tuple_rejects_y_parity_at_or_above_256() {
        // Hand-build the RLP of an authorization tuple whose `y_parity` field encodes
        // 256 (two bytes, 0x01 0x00), independent of the field's Rust type.
        let mut buf = Vec::new();
        Encoder::new(&mut buf)
            .encode_field(&U256::zero()) // chain_id
            .encode_field(&Address::zero()) // address
            .encode_field(&0u64) // nonce
            .encode_field(&U256::from(256u64)) // y_parity = 256, out of the < 2**8 bound
            .encode_field(&U256::one()) // r_signature
            .encode_field(&U256::one()) // s_signature
            .finish();

        let result = AuthorizationTuple::decode(&buf);
        assert!(
            result.is_err(),
            "authorization tuple with y_parity >= 2**8 must be rejected at decode, got: {result:?}"
        );
    }
}
