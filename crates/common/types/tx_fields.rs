use crate::{Address, H256, U256};
use librlp::{RlpDecode, RlpEncode, RlpError};
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
    #[rkyv(with = crate::rkyv_utils::U256Wrapper)]
    pub y_parity: U256,
    #[serde(rename = "r")]
    #[rkyv(with = crate::rkyv_utils::U256Wrapper)]
    pub r_signature: U256,
    #[serde(rename = "s")]
    #[rkyv(with = crate::rkyv_utils::U256Wrapper)]
    pub s_signature: U256,
}

/// Encode an access list item (address, storage_keys) as an RLP list.
pub fn encode_access_list_item(item: &AccessListItem, buf: &mut librlp::RlpBuf) {
    buf.list(|buf| {
        item.0.encode(buf);
        librlp::encode_list(&item.1, buf);
    });
}

/// Compute the encoded length of an access list item.
pub fn access_list_item_encoded_length(item: &AccessListItem) -> usize {
    let payload = item.0.encoded_length() + crate::constants::vec_encoded_length(&item.1);
    crate::constants::list_encoded_length(payload)
}

/// Decode an access list item from RLP.
pub fn decode_access_list_item(buf: &mut &[u8]) -> Result<AccessListItem, RlpError> {
    let header = librlp::Header::decode(buf)?;
    if !header.list {
        return Err(RlpError::UnexpectedString);
    }
    let mut payload = &buf[..header.payload_length];
    let address = RlpDecode::decode(&mut payload)?;
    let storage_keys = librlp::decode_list(&mut payload)?;
    *buf = &buf[header.payload_length..];
    Ok((address, storage_keys))
}

/// Encode an access list as an RLP list of items.
pub fn encode_access_list(list: &AccessList, buf: &mut librlp::RlpBuf) {
    buf.list(|buf| {
        for item in list {
            encode_access_list_item(item, buf);
        }
    });
}

/// Compute the encoded length of an access list.
pub fn access_list_encoded_length(list: &AccessList) -> usize {
    let payload: usize = list.iter().map(access_list_item_encoded_length).sum();
    crate::constants::list_encoded_length(payload)
}

/// Decode an access list from RLP.
pub fn decode_access_list(buf: &mut &[u8]) -> Result<AccessList, RlpError> {
    let header = librlp::Header::decode(buf)?;
    if !header.list {
        return Err(RlpError::UnexpectedString);
    }
    let mut payload = &buf[..header.payload_length];
    let mut list = Vec::new();
    while !payload.is_empty() {
        list.push(decode_access_list_item(&mut payload)?);
    }
    *buf = &buf[header.payload_length..];
    Ok(list)
}

impl RlpEncode for AuthorizationTuple {
    fn encode(&self, buf: &mut librlp::RlpBuf) {
        buf.list(|buf| {
            self.chain_id.encode(buf);
            self.address.encode(buf);
            self.nonce.encode(buf);
            self.y_parity.encode(buf);
            self.r_signature.encode(buf);
            self.s_signature.encode(buf);
        });
    }

    fn encoded_length(&self) -> usize {
        let payload = self.chain_id.encoded_length()
            + self.address.encoded_length()
            + self.nonce.encoded_length()
            + self.y_parity.encoded_length()
            + self.r_signature.encoded_length()
            + self.s_signature.encoded_length();
        crate::constants::list_encoded_length(payload)
    }
}

impl RlpDecode for AuthorizationTuple {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let header = librlp::Header::decode(buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let chain_id = RlpDecode::decode(&mut payload)?;
        let address = RlpDecode::decode(&mut payload)?;
        let nonce = RlpDecode::decode(&mut payload)?;
        let y_parity = RlpDecode::decode(&mut payload)?;
        let r_signature = RlpDecode::decode(&mut payload)?;
        let s_signature = RlpDecode::decode(&mut payload)?;
        *buf = &buf[header.payload_length..];
        Ok(AuthorizationTuple {
            chain_id,
            address,
            nonce,
            y_parity,
            r_signature,
            s_signature,
        })
    }
}
