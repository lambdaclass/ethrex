use super::{
    constants::{RLP_EMPTY_LIST, RLP_NULL},
    error::RLPDecodeError,
};
use bytes::{Bytes, BytesMut};
use ethereum_types::{
    Address, Bloom, H32, H64, H128, H160, H256, H264, H512, H520, Signature, U256,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Max payload size accepted when decoding.
/// While technically any size is RLP spec-compliant, there are no well-formed messages
/// in our protocols that could carry such big payloads, so they are either bugs or malicious.
const MAX_RLP_BYTES: usize = 1024 * 1024 * 1024;

/// Trait for decoding RLP encoded slices of data.
/// See <https://ethereum.org/en/developers/docs/data-structures-and-encoding/rlp/#rlp-decoding> for more information.
/// The [`decode_unfinished`](RLPDecode::decode_unfinished) method is used to decode an RLP encoded slice of data and return the decoded value along with the remaining bytes.
/// The [`decode`](RLPDecode::decode) method is used to decode an RLP encoded slice of data and return the decoded value.
/// Implementors need to implement the [`decode_unfinished`](RLPDecode::decode_unfinished) method.
/// While consumers can use the [`decode`](RLPDecode::decode) method to decode the RLP encoded data.
pub trait RLPDecode: Sized {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError>;

    fn decode(rlp: &[u8]) -> Result<Self, RLPDecodeError> {
        let (decoded, remaining) = Self::decode_unfinished(rlp)?;
        if !remaining.is_empty() {
            return Err(RLPDecodeError::invalid_length());
        }

        Ok(decoded)
    }
}

impl RLPDecode for bool {
    #[inline(always)]
    fn decode_unfinished(buf: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if buf.is_empty() {
            return Err(RLPDecodeError::invalid_length());
        }
        let value = match buf[0] {
            RLP_NULL => false,
            0x01 => true,
            b => return Err(RLPDecodeError::malformed_boolean(b)),
        };

        Ok((value, &buf[1..]))
    }
}

impl RLPDecode for u8 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let first_byte = rlp.first().ok_or(RLPDecodeError::invalid_length())?;
        match first_byte {
            // Single byte in the range [0x00, 0x7f]
            0..=0x7f => {
                let rest = rlp.get(1..).ok_or(RLPDecodeError::malformed_data())?;
                Ok((*first_byte, rest))
            }

            // RLP_NULL represents zero
            &RLP_NULL => {
                let rest = rlp.get(1..).ok_or(RLPDecodeError::malformed_data())?;
                Ok((0, rest))
            }

            // Two bytes, where the first byte is RLP_NULL + 1
            x if rlp.len() >= 2 && *x == RLP_NULL + 1 => {
                let rest = rlp.get(2..).ok_or(RLPDecodeError::malformed_data())?;
                Ok((rlp[1], rest))
            }

            // Any other case is invalid for u8
            _ => Err(RLPDecodeError::malformed_data()),
        }
    }
}

impl RLPDecode for u16 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (bytes, rest) = decode_bytes(rlp)?;
        let padded_bytes = static_left_pad(bytes)?;
        Ok((u16::from_be_bytes(padded_bytes), rest))
    }
}

impl RLPDecode for u32 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (bytes, rest) = decode_bytes(rlp)?;
        let padded_bytes = static_left_pad(bytes)?;
        Ok((u32::from_be_bytes(padded_bytes), rest))
    }
}

impl RLPDecode for u64 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (bytes, rest) = decode_bytes(rlp)?;
        let padded_bytes = static_left_pad(bytes)?;
        Ok((u64::from_be_bytes(padded_bytes), rest))
    }
}

impl RLPDecode for usize {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (bytes, rest) = decode_bytes(rlp)?;
        let padded_bytes = static_left_pad(bytes)?;
        Ok((usize::from_be_bytes(padded_bytes), rest))
    }
}

impl RLPDecode for u128 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (bytes, rest) = decode_bytes(rlp)?;
        let padded_bytes = static_left_pad(bytes)?;
        Ok((u128::from_be_bytes(padded_bytes), rest))
    }
}

// Decodes a slice of bytes of a fixed size. If you want to decode a list of elements,
// you should use the Vec<T> implementation (for elements of the same type),
// or use the decode implementation for tuples (for elements of different types)
impl<const N: usize> RLPDecode for [u8; N] {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (decoded_bytes, rest) = decode_bytes(rlp)?;
        let value = decoded_bytes
            .try_into()
            .map_err(|_| RLPDecodeError::invalid_length());

        Ok((value?, rest))
    }
}

impl RLPDecode for Bytes {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (decoded, rest) = decode_bytes(rlp)?;
        Ok((Bytes::copy_from_slice(decoded), rest))
    }
}

impl RLPDecode for BytesMut {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (decoded, rest) = decode_bytes(rlp)?;
        Ok((BytesMut::from(decoded), rest))
    }
}

impl RLPDecode for H32 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (value, rest) = RLPDecode::decode_unfinished(rlp)?;
        Ok((H32(value), rest))
    }
}

impl RLPDecode for H64 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (value, rest) = RLPDecode::decode_unfinished(rlp)?;
        Ok((H64(value), rest))
    }
}

impl RLPDecode for H128 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (value, rest) = RLPDecode::decode_unfinished(rlp)?;
        Ok((H128(value), rest))
    }
}

impl RLPDecode for H256 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (value, rest) = RLPDecode::decode_unfinished(rlp)?;
        Ok((H256(value), rest))
    }
}

impl RLPDecode for H264 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (value, rest) = RLPDecode::decode_unfinished(rlp)?;
        Ok((H264(value), rest))
    }
}

impl RLPDecode for Address {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (value, rest) = RLPDecode::decode_unfinished(rlp)?;
        Ok((H160(value), rest))
    }
}

impl RLPDecode for H512 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (value, rest) = RLPDecode::decode_unfinished(rlp)?;
        Ok((H512(value), rest))
    }
}

impl RLPDecode for Signature {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (value, rest) = RLPDecode::decode_unfinished(rlp)?;
        Ok((H520(value), rest))
    }
}

impl RLPDecode for U256 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (bytes, rest) = decode_bytes(rlp)?;
        let padded_bytes: [u8; 32] = static_left_pad(bytes)?;
        Ok((U256::from_big_endian(&padded_bytes), rest))
    }
}

impl RLPDecode for Bloom {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (value, rest) = RLPDecode::decode_unfinished(rlp)?;
        Ok((Bloom(value), rest))
    }
}

impl RLPDecode for String {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (str_bytes, rest) = decode_bytes(rlp)?;
        let value =
            String::from_utf8(str_bytes.to_vec()).map_err(|_| RLPDecodeError::malformed_data())?;
        Ok((value, rest))
    }
}

impl RLPDecode for Ipv4Addr {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (ip_bytes, rest) = decode_bytes(rlp)?;
        let octets: [u8; 4] = ip_bytes
            .try_into()
            .map_err(|_| RLPDecodeError::invalid_length())?;
        Ok((Ipv4Addr::from(octets), rest))
    }
}

impl RLPDecode for Ipv6Addr {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (ip_bytes, rest) = decode_bytes(rlp)?;
        let octets: [u8; 16] = ip_bytes
            .try_into()
            .map_err(|_| RLPDecodeError::invalid_length())?;
        Ok((Ipv6Addr::from(octets), rest))
    }
}

impl RLPDecode for IpAddr {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (ip_bytes, rest) = decode_bytes(rlp)?;

        match ip_bytes.len() {
            4 => {
                let octets: [u8; 4] = ip_bytes
                    .try_into()
                    .map_err(|_| RLPDecodeError::invalid_length())?;
                Ok((IpAddr::V4(Ipv4Addr::from(octets)), rest))
            }
            16 => {
                let octets: [u8; 16] = ip_bytes
                    .try_into()
                    .map_err(|_| RLPDecodeError::invalid_length())?;
                // Using to_canonical just in case it's an Ipv6-encoded Ipv4 address
                Ok((IpAddr::V6(Ipv6Addr::from(octets)).to_canonical(), rest))
            }
            _ => Err(RLPDecodeError::invalid_length()),
        }
    }
}

// Here we interpret a Vec<T> as a list of elements of the same type.
// If you need to decode a slice of bytes, you should decode it via the
// [u8; N] implementation or similar (Bytes, BytesMut, etc).
impl<T: RLPDecode> RLPDecode for Vec<T> {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.is_empty() {
            return Err(RLPDecodeError::invalid_length());
        }

        if rlp[0] == RLP_EMPTY_LIST {
            return Ok((Vec::new(), &rlp[1..]));
        }

        let (is_list, payload, input_rest) = decode_rlp_item(rlp)?;
        if !is_list {
            return Err(RLPDecodeError::malformed_data());
        }

        let mut result = Vec::new();
        let mut current_slice = payload;

        while !current_slice.is_empty() {
            let (item, rest_current_list) = T::decode_unfinished(current_slice)?;
            result.push(item);
            current_slice = rest_current_list;
        }

        Ok((result, input_rest))
    }
}

impl<T1: RLPDecode, T2: RLPDecode> RLPDecode for (T1, T2) {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.is_empty() {
            return Err(RLPDecodeError::invalid_length());
        }

        let (is_list, payload, input_rest) = decode_rlp_item(rlp)?;
        if !is_list {
            return Err(RLPDecodeError::malformed_data());
        }

        let (first, first_rest) = T1::decode_unfinished(payload)?;
        let (second, second_rest) = T2::decode_unfinished(first_rest)?;

        // check that there is no more data to parse after the second element.
        if !second_rest.is_empty() {
            return Err(RLPDecodeError::malformed_data());
        }

        Ok(((first, second), input_rest))
    }
}

impl<T1: RLPDecode, T2: RLPDecode, T3: RLPDecode> RLPDecode for (T1, T2, T3) {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.is_empty() {
            return Err(RLPDecodeError::invalid_length());
        }
        let (is_list, payload, input_rest) = decode_rlp_item(rlp)?;
        if !is_list {
            return Err(RLPDecodeError::malformed_data());
        }
        let (first, first_rest) = T1::decode_unfinished(payload)?;
        let (second, second_rest) = T2::decode_unfinished(first_rest)?;
        let (third, third_rest) = T3::decode_unfinished(second_rest)?;
        // check that there is no more data to decode after the third element.
        if !third_rest.is_empty() {
            return Err(RLPDecodeError::malformed_data());
        }

        Ok(((first, second, third), input_rest))
    }
}

// This implementation is useful when the message is a list with elements of mixed types
// for example, the P2P message 'GetBlockHeaders', mixes hashes and numbers.
impl<T1: RLPDecode, T2: RLPDecode, T3: RLPDecode, T4: RLPDecode> RLPDecode for (T1, T2, T3, T4) {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.is_empty() {
            return Err(RLPDecodeError::invalid_length());
        }
        let (is_list, payload, input_rest) = decode_rlp_item(rlp)?;
        if !is_list {
            return Err(RLPDecodeError::malformed_data());
        }
        let (first, first_rest) = T1::decode_unfinished(payload)?;
        let (second, second_rest) = T2::decode_unfinished(first_rest)?;
        let (third, third_rest) = T3::decode_unfinished(second_rest)?;
        let (fourth, fourth_rest) = T4::decode_unfinished(third_rest)?;
        // check that there is no more data to decode after the fourth element.
        if !fourth_rest.is_empty() {
            return Err(RLPDecodeError::malformed_data());
        }

        Ok(((first, second, third, fourth), input_rest))
    }
}

/// Decodes an RLP item from a slice of bytes.
/// It returns a 3-element tuple with the following elements:
/// - A boolean indicating if the item is a list or not.
/// - The payload of the item, without its prefix.
/// - The remaining bytes after the item.
pub fn decode_rlp_item(data: &[u8]) -> Result<(bool, &[u8], &[u8]), RLPDecodeError> {
    if data.is_empty() {
        return Err(RLPDecodeError::invalid_length());
    }

    let first_byte = data[0];

    match first_byte {
        0..=0x7F => Ok((false, &data[..1], &data[1..])),
        0x80..=0xB7 => {
            let length = (first_byte - 0x80) as usize;
            if length > MAX_RLP_BYTES || data.len() < length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            Ok((false, &data[1..length + 1], &data[length + 1..]))
        }
        0xB8..=0xBF => {
            let length_of_length = (first_byte - 0xB7) as usize;
            if data.len() < length_of_length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            let length_bytes = &data[1..length_of_length + 1];
            let length = usize::from_be_bytes(static_left_pad(length_bytes)?);
            if length > MAX_RLP_BYTES || data.len() < length_of_length + length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            Ok((
                false,
                &data[length_of_length + 1..length_of_length + length + 1],
                &data[length_of_length + length + 1..],
            ))
        }
        RLP_EMPTY_LIST..=0xF7 => {
            let length = (first_byte - RLP_EMPTY_LIST) as usize;
            if length > MAX_RLP_BYTES || data.len() < length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            Ok((true, &data[1..length + 1], &data[length + 1..]))
        }
        0xF8..=0xFF => {
            let list_length = (first_byte - 0xF7) as usize;
            if data.len() < list_length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            let length_bytes = &data[1..list_length + 1];
            let payload_length = usize::from_be_bytes(static_left_pad(length_bytes)?);
            if payload_length > MAX_RLP_BYTES || data.len() < list_length + payload_length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            Ok((
                true,
                &data[list_length + 1..list_length + payload_length + 1],
                &data[list_length + payload_length + 1..],
            ))
        }
    }
}

/// Splits an RLP item in two:
/// - The first item including its prefix
/// - The remaining bytes after the item
///
/// It returns a 2-element tuple with the following elements:
/// - The payload of the item, including its prefix.
/// - The remaining bytes after the item.
pub fn get_item_with_prefix(data: &[u8]) -> Result<(&[u8], &[u8]), RLPDecodeError> {
    if data.is_empty() {
        return Err(RLPDecodeError::invalid_length());
    }

    let first_byte = data[0];

    match first_byte {
        0..=0x7F => Ok((&data[..1], &data[1..])),
        0x80..=0xB7 => {
            let length = (first_byte - 0x80) as usize;
            if length > MAX_RLP_BYTES || data.len() < length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            Ok((&data[..length + 1], &data[length + 1..]))
        }
        0xB8..=0xBF => {
            let length_of_length = (first_byte - 0xB7) as usize;
            if data.len() < length_of_length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            let length_bytes = &data[1..length_of_length + 1];
            let length = usize::from_be_bytes(static_left_pad(length_bytes)?);
            if length > MAX_RLP_BYTES || data.len() < length_of_length + length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            Ok((
                &data[..length_of_length + length + 1],
                &data[length_of_length + length + 1..],
            ))
        }
        RLP_EMPTY_LIST..=0xF7 => {
            let length = (first_byte - RLP_EMPTY_LIST) as usize;
            if length > MAX_RLP_BYTES || data.len() < length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            Ok((&data[..length + 1], &data[length + 1..]))
        }
        0xF8..=0xFF => {
            let list_length = (first_byte - 0xF7) as usize;
            if data.len() < list_length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            let length_bytes = &data[1..list_length + 1];
            let payload_length = usize::from_be_bytes(static_left_pad(length_bytes)?);
            if payload_length > MAX_RLP_BYTES || data.len() < list_length + payload_length + 1 {
                return Err(RLPDecodeError::invalid_length());
            }
            Ok((
                &data[..list_length + payload_length + 1],
                &data[list_length + payload_length + 1..],
            ))
        }
    }
}

pub fn is_encoded_as_bytes(rlp: &[u8]) -> Result<bool, RLPDecodeError> {
    let prefix = rlp.first().ok_or(RLPDecodeError::malformed_data())?;
    Ok((0xb8..=0xbf).contains(prefix))
}

/// Receives an RLP bytes item (prefix between 0xb8 and 0xbf) and returns its payload
pub fn get_rlp_bytes_item_payload(rlp: &[u8]) -> Result<&[u8], RLPDecodeError> {
    let prefix = rlp.first().ok_or(RLPDecodeError::invalid_length())?;
    let offset: usize = (prefix - 0xb8 + 1).into();
    rlp.get(offset + 1..)
        .ok_or(RLPDecodeError::invalid_length())
}

/// Decodes the payload of an RLP item from a slice of bytes.
/// It returns a 2-element tuple with the following elements:
/// - The payload of the item.
/// - The remaining bytes after the item.
pub fn decode_bytes(data: &[u8]) -> Result<(&[u8], &[u8]), RLPDecodeError> {
    let (is_list, payload, rest) = decode_rlp_item(data)?;
    if is_list {
        return Err(RLPDecodeError::unexpected_list());
    }
    Ok((payload, rest))
}

/// Pads a slice of bytes with zeros on the left to make it a fixed size slice.
/// The size of the data must be less than or equal to the size of the output array.
#[inline]
pub fn static_left_pad<const N: usize>(data: &[u8]) -> Result<[u8; N], RLPDecodeError> {
    let mut result = [0; N];

    if data.is_empty() {
        return Ok(result);
    }
    if data[0] == 0 {
        return Err(RLPDecodeError::malformed_data());
    }
    if data.len() > N {
        return Err(RLPDecodeError::invalid_length());
    }
    let data_start_index = N.saturating_sub(data.len());
    result
        .get_mut(data_start_index..)
        .ok_or(RLPDecodeError::invalid_length())?
        .copy_from_slice(data);
    Ok(result)
}
