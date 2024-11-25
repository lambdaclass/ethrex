use super::{
    constants::{RLP_EMPTY_LIST, RLP_NULL},
    error::RLPDecodeError,
};
use bytes::{Bytes, BytesMut};
use ethereum_types::{
    Address, Bloom, Signature, H128, H160, H256, H264, H32, H512, H520, H64, U256,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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
            return Err(RLPDecodeError::InvalidLength);
        }

        Ok(decoded)
    }
}

impl RLPDecode for bool {
    #[inline(always)]
    fn decode_unfinished(buf: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if buf.is_empty() {
            return Err(RLPDecodeError::InvalidLength);
        }
        let value = match buf[0] {
            RLP_NULL => false,
            0x01 => true,
            _ => return Err(RLPDecodeError::MalformedBoolean),
        };

        Ok((value, &buf[1..]))
    }
}

impl RLPDecode for u8 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let first_byte = rlp.first().ok_or(RLPDecodeError::InvalidLength)?;
        match first_byte {
            // Single byte in the range [0x00, 0x7f]
            0..=0x7f => {
                let rest = rlp.get(1..).ok_or(RLPDecodeError::MalformedData)?;
                Ok((*first_byte, rest))
            }

            // RLP_NULL represents zero
            &RLP_NULL => {
                let rest = rlp.get(1..).ok_or(RLPDecodeError::MalformedData)?;
                Ok((0, rest))
            }

            // Two bytes, where the first byte is RLP_NULL + 1
            x if rlp.len() >= 2 && *x == RLP_NULL + 1 => {
                let rest = rlp.get(2..).ok_or(RLPDecodeError::MalformedData)?;
                Ok((rlp[1], rest))
            }

            // Any other case is invalid for u8
            _ => Err(RLPDecodeError::MalformedData),
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
            .map_err(|_| RLPDecodeError::InvalidLength);

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
            String::from_utf8(str_bytes.to_vec()).map_err(|_| RLPDecodeError::MalformedData)?;
        Ok((value, rest))
    }
}

impl RLPDecode for Ipv4Addr {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (ip_bytes, rest) = decode_bytes(rlp)?;
        let octets: [u8; 4] = ip_bytes
            .try_into()
            .map_err(|_| RLPDecodeError::InvalidLength)?;
        Ok((Ipv4Addr::from(octets), rest))
    }
}

impl RLPDecode for Ipv6Addr {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (ip_bytes, rest) = decode_bytes(rlp)?;
        let octets: [u8; 16] = ip_bytes
            .try_into()
            .map_err(|_| RLPDecodeError::InvalidLength)?;
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
                    .map_err(|_| RLPDecodeError::InvalidLength)?;
                Ok((IpAddr::V4(Ipv4Addr::from(octets)), rest))
            }
            16 => {
                let octets: [u8; 16] = ip_bytes
                    .try_into()
                    .map_err(|_| RLPDecodeError::InvalidLength)?;
                Ok((IpAddr::V6(Ipv6Addr::from(octets)), rest))
            }
            _ => Err(RLPDecodeError::InvalidLength),
        }
    }
}

// Here we interpret a Vec<T> as a list of elements of the same type.
// If you need to decode a slice of bytes, you should decode it via the
// [u8; N] implementation or similar (Bytes, BytesMut, etc).
impl<T: RLPDecode> RLPDecode for Vec<T> {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.is_empty() {
            return Err(RLPDecodeError::InvalidLength);
        }

        if rlp[0] == RLP_EMPTY_LIST {
            return Ok((Vec::new(), &rlp[1..]));
        }

        let (is_list, payload, input_rest) = decode_rlp_item(rlp)?;
        if !is_list {
            return Err(RLPDecodeError::MalformedData);
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
            return Err(RLPDecodeError::InvalidLength);
        }

        let (is_list, payload, input_rest) = decode_rlp_item(rlp)?;
        if !is_list {
            return Err(RLPDecodeError::MalformedData);
        }

        let (first, first_rest) = T1::decode_unfinished(payload)?;
        let (second, second_rest) = T2::decode_unfinished(first_rest)?;

        // check that there is no more data to parse after the second element.
        if !second_rest.is_empty() {
            return Err(RLPDecodeError::MalformedData);
        }

        Ok(((first, second), input_rest))
    }
}

impl<T1: RLPDecode, T2: RLPDecode, T3: RLPDecode> RLPDecode for (T1, T2, T3) {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.is_empty() {
            return Err(RLPDecodeError::InvalidLength);
        }
        let (is_list, payload, input_rest) = decode_rlp_item(rlp)?;
        if !is_list {
            return Err(RLPDecodeError::MalformedData);
        }
        let (first, first_rest) = T1::decode_unfinished(payload)?;
        let (second, second_rest) = T2::decode_unfinished(first_rest)?;
        let (third, third_rest) = T3::decode_unfinished(second_rest)?;
        // check that there is no more data to decode after the third element.
        if !third_rest.is_empty() {
            return Err(RLPDecodeError::MalformedData);
        }

        Ok(((first, second, third), input_rest))
    }
}

// This implementation is useful when the message is a list with elements of mixed types
// for example, the P2P message 'GetBlockHeaders', mixes hashes and numbers.
impl<T1: RLPDecode, T2: RLPDecode, T3: RLPDecode, T4: RLPDecode> RLPDecode for (T1, T2, T3, T4) {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if rlp.is_empty() {
            return Err(RLPDecodeError::InvalidLength);
        }
        let (is_list, payload, input_rest) = decode_rlp_item(rlp)?;
        if !is_list {
            return Err(RLPDecodeError::MalformedData);
        }
        let (first, first_rest) = T1::decode_unfinished(payload)?;
        let (second, second_rest) = T2::decode_unfinished(first_rest)?;
        let (third, third_rest) = T3::decode_unfinished(second_rest)?;
        let (fourth, fourth_rest) = T4::decode_unfinished(third_rest)?;
        // check that there is no more data to decode after the fourth element.
        if !fourth_rest.is_empty() {
            return Err(RLPDecodeError::MalformedData);
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
        return Err(RLPDecodeError::InvalidLength);
    }

    let first_byte = data[0];

    match first_byte {
        0..=0x7F => Ok((false, &data[..1], &data[1..])),
        0x80..=0xB7 => {
            let length = (first_byte - 0x80) as usize;
            if data.len() < length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            Ok((false, &data[1..length + 1], &data[length + 1..]))
        }
        0xB8..=0xBF => {
            let length_of_length = (first_byte - 0xB7) as usize;
            if data.len() < length_of_length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            let length_bytes = &data[1..length_of_length + 1];
            let length = usize::from_be_bytes(static_left_pad(length_bytes)?);
            if data.len() < length_of_length + length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            Ok((
                false,
                &data[length_of_length + 1..length_of_length + length + 1],
                &data[length_of_length + length + 1..],
            ))
        }
        RLP_EMPTY_LIST..=0xF7 => {
            let length = (first_byte - RLP_EMPTY_LIST) as usize;
            if data.len() < length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            Ok((true, &data[1..length + 1], &data[length + 1..]))
        }
        0xF8..=0xFF => {
            let list_length = (first_byte - 0xF7) as usize;
            if data.len() < list_length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            let length_bytes = &data[1..list_length + 1];
            let payload_length = usize::from_be_bytes(static_left_pad(length_bytes)?);
            if data.len() < list_length + payload_length + 1 {
                return Err(RLPDecodeError::InvalidLength);
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
        return Err(RLPDecodeError::InvalidLength);
    }

    let first_byte = data[0];

    match first_byte {
        0..=0x7F => Ok((&data[..1], &data[1..])),
        0x80..=0xB7 => {
            let length = (first_byte - 0x80) as usize;
            if data.len() < length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            Ok((&data[..length + 1], &data[length + 1..]))
        }
        0xB8..=0xBF => {
            let length_of_length = (first_byte - 0xB7) as usize;
            if data.len() < length_of_length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            let length_bytes = &data[1..length_of_length + 1];
            let length = usize::from_be_bytes(static_left_pad(length_bytes)?);
            if data.len() < length_of_length + length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            Ok((
                &data[..length_of_length + length + 1],
                &data[length_of_length + length + 1..],
            ))
        }
        RLP_EMPTY_LIST..=0xF7 => {
            let length = (first_byte - RLP_EMPTY_LIST) as usize;
            if data.len() < length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            Ok((&data[..length + 1], &data[length + 1..]))
        }
        0xF8..=0xFF => {
            let list_length = (first_byte - 0xF7) as usize;
            if data.len() < list_length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            let length_bytes = &data[1..list_length + 1];
            let payload_length = usize::from_be_bytes(static_left_pad(length_bytes)?);
            if data.len() < list_length + payload_length + 1 {
                return Err(RLPDecodeError::InvalidLength);
            }
            Ok((
                &data[..list_length + payload_length + 1],
                &data[list_length + payload_length + 1..],
            ))
        }
    }
}

pub fn is_encoded_as_bytes(rlp: &[u8]) -> Result<bool, RLPDecodeError> {
    let prefix = rlp.first().ok_or(RLPDecodeError::MalformedData)?;
    Ok((0xb8..=0xbf).contains(prefix))
}

/// Receives an RLP bytes item (prefix between 0xb8 and 0xbf) and returns its payload
pub fn get_rlp_bytes_item_payload(rlp: &[u8]) -> &[u8] {
    let prefix = rlp.first().unwrap();
    let offset: usize = (prefix - 0xb8 + 1).into();
    &rlp[offset + 1..]
}

/// Decodes the payload of an RLP item from a slice of bytes.
/// It returns a 2-element tuple with the following elements:
/// - The payload of the item.
/// - The remaining bytes after the item.
pub fn decode_bytes(data: &[u8]) -> Result<(&[u8], &[u8]), RLPDecodeError> {
    let (is_list, payload, rest) = decode_rlp_item(data)?;
    if is_list {
        return Err(RLPDecodeError::UnexpectedList);
    }
    Ok((payload, rest))
}

/// Pads a slice of bytes with zeros on the left to make it a fixed size slice.
/// The size of the data must be less than or equal to the size of the output array.
#[inline]
pub(crate) fn static_left_pad<const N: usize>(data: &[u8]) -> Result<[u8; N], RLPDecodeError> {
    let mut result = [0; N];

    if data.is_empty() {
        return Ok(result);
    }
    if data[0] == 0 {
        return Err(RLPDecodeError::MalformedData);
    }
    if data.len() > N {
        return Err(RLPDecodeError::InvalidLength);
    }
    let data_start_index = N.saturating_sub(data.len());
    result
        .get_mut(data_start_index..)
        .ok_or(RLPDecodeError::InvalidLength)?
        .copy_from_slice(data);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_decode_bool() {
        let rlp = vec![0x01];
        let decoded = bool::decode(&rlp).unwrap();
        assert!(decoded);

        let rlp = vec![RLP_NULL];
        let decoded = bool::decode(&rlp).unwrap();
        assert!(!decoded);
    }

    #[test]
    fn test_decode_u8() {
        let rlp = vec![0x01];
        let decoded = u8::decode(&rlp).unwrap();
        assert_eq!(decoded, 1);

        let rlp = vec![RLP_NULL];
        let decoded = u8::decode(&rlp).unwrap();
        assert_eq!(decoded, 0);

        let rlp = vec![0x7Fu8];
        let decoded = u8::decode(&rlp).unwrap();
        assert_eq!(decoded, 127);

        let rlp = vec![RLP_NULL + 1, RLP_NULL];
        let decoded = u8::decode(&rlp).unwrap();
        assert_eq!(decoded, 128);

        let rlp = vec![RLP_NULL + 1, 0x90];
        let decoded = u8::decode(&rlp).unwrap();
        assert_eq!(decoded, 144);

        let rlp = vec![RLP_NULL + 1, 0xFF];
        let decoded = u8::decode(&rlp).unwrap();
        assert_eq!(decoded, 255);
    }

    #[test]
    fn test_decode_u16() {
        let rlp = vec![0x01];
        let decoded = u8::decode(&rlp).unwrap();
        assert_eq!(decoded, 1);

        let rlp = vec![RLP_NULL];
        let decoded = u8::decode(&rlp).unwrap();
        assert_eq!(decoded, 0);

        let rlp = vec![0x81, 0xFF];
        let decoded = u8::decode(&rlp).unwrap();
        assert_eq!(decoded, 255);
    }

    #[test]
    fn test_decode_u32() {
        let rlp = vec![0x83, 0x01, 0x00, 0x00];
        let decoded = u32::decode(&rlp).unwrap();
        assert_eq!(decoded, 65536);
    }

    #[test]
    fn test_decode_fixed_length_array() {
        let rlp = vec![0x0f];
        let decoded = <[u8; 1]>::decode(&rlp).unwrap();
        assert_eq!(decoded, [0x0f]);

        let rlp = vec![RLP_NULL + 3, 0x02, 0x03, 0x04];
        let decoded = <[u8; 3]>::decode(&rlp).unwrap();
        assert_eq!(decoded, [0x02, 0x03, 0x04]);
    }

    #[test]
    fn test_decode_ip_addresses() {
        // IPv4
        let rlp = vec![RLP_NULL + 4, 192, 168, 0, 1];
        let decoded = Ipv4Addr::decode(&rlp).unwrap();
        let expected = Ipv4Addr::from_str("192.168.0.1").unwrap();
        assert_eq!(decoded, expected);

        // IPv6
        let rlp = vec![
            0x90, 0x20, 0x01, 0x00, 0x00, 0x13, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x09, 0xc0, 0x87,
            0x6a, 0x13, 0x0b,
        ];
        let decoded = Ipv6Addr::decode(&rlp).unwrap();
        let expected = Ipv6Addr::from_str("2001:0000:130F:0000:0000:09C0:876A:130B").unwrap();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_decode_u256() {
        let rlp = vec![RLP_NULL + 1, 0x01];
        let decoded = U256::decode(&rlp).unwrap();
        let expected = U256::from(1);
        assert_eq!(decoded, expected);

        let mut rlp = vec![RLP_NULL + 32];
        let number_bytes = [0x01; 32];
        rlp.extend(number_bytes);
        let decoded = U256::decode(&rlp).unwrap();
        let expected = U256::from_big_endian(&number_bytes);
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_decode_string() {
        let rlp = vec![RLP_NULL + 3, b'd', b'o', b'g'];
        let decoded = String::decode(&rlp).unwrap();
        let expected = String::from("dog");
        assert_eq!(decoded, expected);

        let rlp = vec![RLP_NULL];
        let decoded = String::decode(&rlp).unwrap();
        let expected = String::from("");
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_decode_lists() {
        // empty list
        let rlp = vec![RLP_EMPTY_LIST];
        let decoded: Vec<String> = Vec::decode(&rlp).unwrap();
        let expected: Vec<String> = vec![];
        assert_eq!(decoded, expected);

        //  list with a single number
        let rlp = vec![RLP_EMPTY_LIST + 1, 0x01];
        let decoded: Vec<u8> = Vec::decode(&rlp).unwrap();
        let expected = vec![1];
        assert_eq!(decoded, expected);

        // list with 3 numbers
        let rlp = vec![RLP_EMPTY_LIST + 3, 0x01, 0x02, 0x03];
        let decoded: Vec<u8> = Vec::decode(&rlp).unwrap();
        let expected = vec![1, 2, 3];
        assert_eq!(decoded, expected);

        // list of strings
        let rlp = vec![0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g'];
        let decoded: Vec<String> = Vec::decode(&rlp).unwrap();
        let expected = vec!["cat".to_string(), "dog".to_string()];
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_decode_list_of_lists() {
        // list of lists of numbers
        let rlp = vec![
            RLP_EMPTY_LIST + 6,
            RLP_EMPTY_LIST + 2,
            0x01,
            0x02,
            RLP_EMPTY_LIST + 2,
            0x03,
            0x04,
        ];
        let decoded: Vec<Vec<u8>> = Vec::decode(&rlp).unwrap();
        let expected = vec![vec![1, 2], vec![3, 4]];
        assert_eq!(decoded, expected);

        // list of list of strings
        let rlp = vec![
            0xd2, 0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g', 0xc8, 0x83, b'f', b'o',
            b'o', 0x83, b'b', b'a', b'r',
        ];
        let decoded: Vec<Vec<String>> = Vec::decode(&rlp).unwrap();
        let expected = vec![
            vec!["cat".to_string(), "dog".to_string()],
            vec!["foo".to_string(), "bar".to_string()],
        ];
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_decode_tuples() {
        // tuple with numbers
        let rlp = vec![RLP_EMPTY_LIST + 2, 0x01, 0x02];
        let decoded: (u8, u8) = <(u8, u8)>::decode(&rlp).unwrap();
        let expected = (1, 2);
        assert_eq!(decoded, expected);

        // tuple with string and number
        let rlp = vec![RLP_EMPTY_LIST + 5, 0x01, 0x83, b'c', b'a', b't'];
        let decoded: (u8, String) = <(u8, String)>::decode(&rlp).unwrap();
        let expected = (1, "cat".to_string());
        assert_eq!(decoded, expected);

        // tuple with bool and string
        let rlp = vec![RLP_EMPTY_LIST + 6, 0x01, 0x84, b't', b'r', b'u', b'e'];
        let decoded: (bool, String) = <(bool, String)>::decode(&rlp).unwrap();
        let expected = (true, "true".to_string());
        assert_eq!(decoded, expected);

        // tuple with list and number
        let rlp = vec![RLP_EMPTY_LIST + 2, RLP_EMPTY_LIST, 0x03];
        let decoded = <(Vec<u8>, u8)>::decode(&rlp).unwrap();
        let expected = (vec![], 3);
        assert_eq!(decoded, expected);

        // tuple with number and list
        let rlp = vec![RLP_EMPTY_LIST + 2, 0x03, RLP_EMPTY_LIST];
        let decoded = <(u8, Vec<u8>)>::decode(&rlp).unwrap();
        let expected = (3, vec![]);
        assert_eq!(decoded, expected);

        // tuple with tuples
        let rlp = vec![
            RLP_EMPTY_LIST + 6,
            RLP_EMPTY_LIST + 2,
            0x01,
            0x02,
            RLP_EMPTY_LIST + 2,
            0x03,
            0x04,
        ];
        let decoded = <((u8, u8), (u8, u8))>::decode(&rlp).unwrap();
        let expected = ((1, 2), (3, 4));
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_decode_tuples_3_elements() {
        // tuple with numbers
        let rlp = vec![RLP_EMPTY_LIST + 3, 0x01, 0x02, 0x03];
        let decoded: (u8, u8, u8) = <(u8, u8, u8)>::decode(&rlp).unwrap();
        let expected = (1, 2, 3);
        assert_eq!(decoded, expected);

        // tuple with string and number
        let rlp = vec![RLP_EMPTY_LIST + 6, 0x01, 0x02, 0x83, b'c', b'a', b't'];
        let decoded: (u8, u8, String) = <(u8, u8, String)>::decode(&rlp).unwrap();
        let expected = (1, 2, "cat".to_string());
        assert_eq!(decoded, expected);

        // tuple with bool and string
        let rlp = vec![RLP_EMPTY_LIST + 7, 0x01, 0x02, 0x84, b't', b'r', b'u', b'e'];
        let decoded: (u8, u8, String) = <(u8, u8, String)>::decode(&rlp).unwrap();
        let expected = (1, 2, "true".to_string());
        assert_eq!(decoded, expected);

        // tuple with tuples
        let rlp = vec![
            RLP_EMPTY_LIST + 9,
            RLP_EMPTY_LIST + 2,
            0x01,
            0x02,
            RLP_EMPTY_LIST + 2,
            0x03,
            0x04,
            RLP_EMPTY_LIST + 2,
            0x05,
            0x06,
        ];
        let decoded = <((u8, u8), (u8, u8), (u8, u8))>::decode(&rlp).unwrap();
        let expected = ((1, 2), (3, 4), (5, 6));
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_decode_list_as_string() {
        // [1, 2, 3, 4] != 0x01020304
        let rlp = vec![RLP_EMPTY_LIST + 4, 0x01, 0x02, 0x03, 0x04];
        let decoded: Result<[u8; 4], _> = RLPDecode::decode(&rlp);
        // It should fail because a list is not a string
        assert!(decoded.is_err());

        // [1, 2] != 0x0102
        let rlp = vec![RLP_EMPTY_LIST + 2, 0x01, 0x02];
        let decoded: Result<u16, _> = RLPDecode::decode(&rlp);
        // It should fail because a list is not a string
        assert!(decoded.is_err());
    }
}
