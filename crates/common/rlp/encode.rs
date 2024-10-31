use bytes::{BufMut, Bytes};
use ethereum_types::U256;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use tinyvec::ArrayVec;

use super::constants::RLP_NULL;

/// Function for encoding a value to RLP.
/// For encoding the value into a buffer directly, use [`RLPEncode::encode`].
pub fn encode<T: RLPEncode>(value: T) -> Vec<u8> {
    let mut buf = Vec::new();
    value.encode(&mut buf);
    buf
}

pub trait RLPEncode {
    fn encode(&self, buf: &mut dyn BufMut);

    fn length(&self) -> usize {
        let mut buf = Vec::new();
        self.encode(&mut buf);
        buf.len()
    }

    fn encode_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode(&mut buf);
        buf
    }
}

impl RLPEncode for bool {
    #[inline(always)]
    fn encode(&self, buf: &mut dyn BufMut) {
        if *self {
            buf.put_u8(0x01);
        } else {
            buf.put_u8(RLP_NULL);
        }
    }

    #[inline(always)]
    fn length(&self) -> usize {
        1
    }
}

// integer types impls

impl RLPEncode for u8 {
    fn encode(&self, buf: &mut dyn BufMut) {
        match *self {
            // 0, also known as null or the empty string is 0x80
            0 => buf.put_u8(RLP_NULL),
            // for a single byte whose value is in the [0x00, 0x7f] range, that byte is its own RLP encoding.
            n @ 1..=0x7f => buf.put_u8(n),
            // Otherwise, if a string is 0-55 bytes long, the RLP encoding consists of a
            // single byte with value RLP_NULL (0x80) plus the length of the string followed by the string.
            n => {
                let mut bytes = ArrayVec::<[u8; 8]>::new();
                bytes.extend_from_slice(&n.to_be_bytes());
                let start = bytes.iter().position(|&x| x != 0).unwrap();
                let len = bytes.len() - start;
                buf.put_u8(RLP_NULL + len as u8);
                buf.put_slice(&bytes[start..]);
            }
        }
    }
}

impl RLPEncode for u16 {
    fn encode(&self, buf: &mut dyn BufMut) {
        match *self {
            // 0, also known as null or the empty string is 0x80
            0 => buf.put_u8(RLP_NULL),
            // for a single byte whose value is in the [0x00, 0x7f] range, that byte is its own RLP encoding.
            n @ 1..=0x7f => buf.put_u8(n as u8),
            // Otherwise, if a string is 0-55 bytes long, the RLP encoding consists of a
            // single byte with value RLP_NULL (0x80) plus the length of the string followed by the string.
            n => {
                let mut bytes = ArrayVec::<[u8; 8]>::new();
                bytes.extend_from_slice(&n.to_be_bytes());
                let start = bytes.iter().position(|&x| x != 0).unwrap();
                let len = bytes.len() - start;
                buf.put_u8(RLP_NULL + len as u8);
                buf.put_slice(&bytes[start..]);
            }
        }
    }
}

impl RLPEncode for u32 {
    fn encode(&self, buf: &mut dyn BufMut) {
        match *self {
            // 0, also known as null or the empty string is 0x80
            0 => buf.put_u8(RLP_NULL),
            // for a single byte whose value is in the [0x00, 0x7f] range, that byte is its own RLP encoding.
            n @ 1..=0x7f => buf.put_u8(n as u8),
            // Otherwise, if a string is 0-55 bytes long, the RLP encoding consists of a
            // single byte with value RLP_NULL (0x80) plus the length of the string followed by the string.
            n => {
                let mut bytes = ArrayVec::<[u8; 8]>::new();
                bytes.extend_from_slice(&n.to_be_bytes());
                let start = bytes.iter().position(|&x| x != 0).unwrap();
                let len = bytes.len() - start;
                buf.put_u8(RLP_NULL + len as u8);
                buf.put_slice(&bytes[start..]);
            }
        }
    }
}

impl RLPEncode for u64 {
    fn encode(&self, buf: &mut dyn BufMut) {
        match *self {
            // 0, also known as null or the empty string is 0x80
            0 => buf.put_u8(RLP_NULL),
            // for a single byte whose value is in the [0x00, 0x7f] range, that byte is its own RLP encoding.
            n @ 1..=0x7f => buf.put_u8(n as u8),
            // Otherwise, if a string is 0-55 bytes long, the RLP encoding consists of a
            // single byte with value RLP_NULL (0x80) plus the length of the string followed by the string.
            n => {
                let mut bytes = ArrayVec::<[u8; 8]>::new();
                bytes.extend_from_slice(&n.to_be_bytes());
                let start = bytes.iter().position(|&x| x != 0).unwrap();
                let len = bytes.len() - start;
                buf.put_u8(RLP_NULL + len as u8);
                buf.put_slice(&bytes[start..]);
            }
        }
    }
}

impl RLPEncode for usize {
    fn encode(&self, buf: &mut dyn BufMut) {
        match *self {
            // 0, also known as null or the empty string is 0x80
            0 => buf.put_u8(RLP_NULL),
            // for a single byte whose value is in the [0x00, 0x7f] range, that byte is its own RLP encoding.
            n @ 1..=0x7f => buf.put_u8(n as u8),
            // Otherwise, if a string is 0-55 bytes long, the RLP encoding consists of a
            // single byte with value RLP_NULL (0x80) plus the length of the string followed by the string.
            n => {
                let mut bytes = ArrayVec::<[u8; 8]>::new();
                bytes.extend_from_slice(&n.to_be_bytes());
                let start = bytes.iter().position(|&x| x != 0).unwrap();
                let len = bytes.len() - start;
                buf.put_u8(RLP_NULL + len as u8);
                buf.put_slice(&bytes[start..]);
            }
        }
    }
}

impl RLPEncode for u128 {
    fn encode(&self, buf: &mut dyn BufMut) {
        match *self {
            // 0, also known as null or the empty string is 0x80
            0 => buf.put_u8(RLP_NULL),
            // for a single byte whose value is in the [0x00, 0x7f] range, that byte is its own RLP encoding.
            n @ 1..=0x7f => buf.put_u8(n as u8),
            // Otherwise, if a string is 0-55 bytes long, the RLP encoding consists of a
            // single byte with value RLP_NULL (0x80) plus the length of the string followed by the string.
            n => {
                let mut bytes = ArrayVec::<[u8; 16]>::new();
                bytes.extend_from_slice(&n.to_be_bytes());
                let start = bytes.iter().position(|&x| x != 0).unwrap();
                let len = bytes.len() - start;
                buf.put_u8(RLP_NULL + len as u8);
                buf.put_slice(&bytes[start..]);
            }
        }
    }
}

impl RLPEncode for () {
    fn encode(&self, buf: &mut dyn BufMut) {
        buf.put_u8(RLP_NULL);
    }
}

impl RLPEncode for [u8] {
    #[inline(always)]
    fn encode(&self, buf: &mut dyn BufMut) {
        if self.len() == 1 && self[0] < RLP_NULL {
            buf.put_u8(self[0]);
        } else {
            let len = self.len();
            if len < 56 {
                buf.put_u8(RLP_NULL + len as u8);
            } else {
                let mut bytes = ArrayVec::<[u8; 8]>::new();
                bytes.extend_from_slice(&len.to_be_bytes());
                let start = bytes.iter().position(|&x| x != 0).unwrap();
                let len = bytes.len() - start;
                buf.put_u8(0xb7 + len as u8);
                buf.put_slice(&bytes[start..]);
            }
            buf.put_slice(self);
        }
    }
}

impl<const N: usize> RLPEncode for [u8; N] {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_ref().encode(buf)
    }
}

impl RLPEncode for str {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for &str {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for String {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for U256 {
    fn encode(&self, buf: &mut dyn BufMut) {
        let leading_zeros_in_bytes: usize = (self.leading_zeros() / 8) as usize;
        let mut bytes: [u8; 32] = [0; 32];
        self.to_big_endian(&mut bytes);
        bytes[leading_zeros_in_bytes..].encode(buf)
    }
}

impl<T: RLPEncode> RLPEncode for Vec<T> {
    fn encode(&self, buf: &mut dyn BufMut) {
        if self.is_empty() {
            buf.put_u8(0xc0);
        } else {
            let mut total_len = 0;
            for item in self {
                total_len += item.length();
            }
            encode_length(total_len, buf);
            for item in self {
                item.encode(buf);
            }
        }
    }
}

pub(crate) fn encode_length(total_len: usize, buf: &mut dyn BufMut) {
    if total_len < 56 {
        buf.put_u8(0xc0 + total_len as u8);
    } else {
        let mut bytes = ArrayVec::<[u8; 8]>::new();
        bytes.extend_from_slice(&total_len.to_be_bytes());
        let start = bytes.iter().position(|&x| x != 0).unwrap();
        let len = bytes.len() - start;
        buf.put_u8(0xf7 + len as u8);
        buf.put_slice(&bytes[start..]);
    }
}

impl<S: RLPEncode, T: RLPEncode> RLPEncode for (S, T) {
    fn encode(&self, buf: &mut dyn BufMut) {
        let total_len = self.0.length() + self.1.length();
        encode_length(total_len, buf);
        self.0.encode(buf);
        self.1.encode(buf);
    }
}

impl<S: RLPEncode, T: RLPEncode, U: RLPEncode> RLPEncode for (S, T, U) {
    fn encode(&self, buf: &mut dyn BufMut) {
        let total_len = self.0.length() + self.1.length() + self.2.length();
        encode_length(total_len, buf);
        self.0.encode(buf);
        self.1.encode(buf);
        self.2.encode(buf);
    }
}

impl<S: RLPEncode, T: RLPEncode, U: RLPEncode, V: RLPEncode> RLPEncode for (S, T, U, V) {
    fn encode(&self, buf: &mut dyn BufMut) {
        let total_len = self.0.length() + self.1.length() + self.2.length() + self.3.length();
        encode_length(total_len, buf);
        self.0.encode(buf);
        self.1.encode(buf);
        self.2.encode(buf);
        self.3.encode(buf);
    }
}

impl<S: RLPEncode, T: RLPEncode, U: RLPEncode, V: RLPEncode, W: RLPEncode> RLPEncode
    for (S, T, U, V, W)
{
    fn encode(&self, buf: &mut dyn BufMut) {
        let total_len =
            self.0.length() + self.1.length() + self.2.length() + self.3.length() + self.4.length();
        encode_length(total_len, buf);
        self.0.encode(buf);
        self.1.encode(buf);
        self.2.encode(buf);
        self.3.encode(buf);
        self.4.encode(buf);
    }
}

impl RLPEncode for Ipv4Addr {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.octets().encode(buf)
    }
}

impl RLPEncode for Ipv6Addr {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.octets().encode(buf)
    }
}

impl RLPEncode for IpAddr {
    fn encode(&self, buf: &mut dyn BufMut) {
        match self {
            IpAddr::V4(ip) => ip.encode(buf),
            IpAddr::V6(ip) => ip.encode(buf),
        }
    }
}

impl RLPEncode for Bytes {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_ref().encode(buf)
    }
}

// encoding for Ethereum types

impl RLPEncode for ethereum_types::H32 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for ethereum_types::H64 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for ethereum_types::H128 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for ethereum_types::Address {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for ethereum_types::H256 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for ethereum_types::H264 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for ethereum_types::H512 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for ethereum_types::Signature {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }
}

impl RLPEncode for ethereum_types::Bloom {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.0.encode(buf)
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use ethereum_types::{Address, U256};
    use hex_literal::hex;

    use crate::constants::{RLP_EMPTY_LIST, RLP_NULL};

    use super::RLPEncode;

    #[test]
    fn can_encode_booleans() {
        let mut encoded = Vec::new();
        true.encode(&mut encoded);
        assert_eq!(encoded, vec![0x01]);

        let mut encoded = Vec::new();
        false.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL]);
    }

    #[test]
    fn can_encode_u32() {
        let mut encoded = Vec::new();
        0u16.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL]);

        let mut encoded = Vec::new();
        1u16.encode(&mut encoded);
        assert_eq!(encoded, vec![0x01]);

        let mut encoded = Vec::new();
        0x7Fu16.encode(&mut encoded);
        assert_eq!(encoded, vec![0x7f]);

        let mut encoded = Vec::new();
        0x80u16.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL + 1, 0x80]);

        let mut encoded = Vec::new();
        0x90u16.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL + 1, 0x90]);
    }

    #[test]
    fn can_encode_u16() {
        let mut encoded = Vec::new();
        0u16.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL]);

        let mut encoded = Vec::new();
        1u16.encode(&mut encoded);
        assert_eq!(encoded, vec![0x01]);

        let mut encoded = Vec::new();
        0x7Fu16.encode(&mut encoded);
        assert_eq!(encoded, vec![0x7f]);

        let mut encoded = Vec::new();
        0x80u16.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL + 1, 0x80]);

        let mut encoded = Vec::new();
        0x90u16.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL + 1, 0x90]);
    }

    #[test]
    fn can_encode_u8() {
        let mut encoded = Vec::new();
        0u8.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL]);

        let mut encoded = Vec::new();
        1u8.encode(&mut encoded);
        assert_eq!(encoded, vec![0x01]);

        let mut encoded = Vec::new();
        0x7Fu8.encode(&mut encoded);
        assert_eq!(encoded, vec![0x7f]);

        let mut encoded = Vec::new();
        0x80u8.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL + 1, 0x80]);

        let mut encoded = Vec::new();
        0x90u8.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL + 1, 0x90]);
    }

    #[test]
    fn can_encode_u64() {
        let mut encoded = Vec::new();
        0u8.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL]);

        let mut encoded = Vec::new();
        1u8.encode(&mut encoded);
        assert_eq!(encoded, vec![0x01]);

        let mut encoded = Vec::new();
        0x7Fu8.encode(&mut encoded);
        assert_eq!(encoded, vec![0x7f]);

        let mut encoded = Vec::new();
        0x80u8.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL + 1, 0x80]);

        let mut encoded = Vec::new();
        0x90u8.encode(&mut encoded);
        assert_eq!(encoded, vec![RLP_NULL + 1, 0x90]);
    }

    #[test]
    fn can_encode_usize() {
        let mut encoded = Vec::new();
        0u8.encode(&mut encoded);
        assert_eq!(encoded, vec![0x80]);

        let mut encoded = Vec::new();
        1u8.encode(&mut encoded);
        assert_eq!(encoded, vec![0x01]);

        let mut encoded = Vec::new();
        0x7Fu8.encode(&mut encoded);
        assert_eq!(encoded, vec![0x7f]);

        let mut encoded = Vec::new();
        0x80u8.encode(&mut encoded);
        assert_eq!(encoded, vec![0x80 + 1, 0x80]);

        let mut encoded = Vec::new();
        0x90u8.encode(&mut encoded);
        assert_eq!(encoded, vec![0x80 + 1, 0x90]);
    }

    #[test]
    fn can_encode_bytes() {
        // encode byte 0x00
        let message: [u8; 1] = [0x00];
        let encoded = {
            let mut buf = vec![];
            message.encode(&mut buf);
            buf
        };
        assert_eq!(encoded, vec![0x00]);

        // encode byte 0x0f
        let message: [u8; 1] = [0x0f];
        let encoded = {
            let mut buf = vec![];
            message.encode(&mut buf);
            buf
        };
        assert_eq!(encoded, vec![0x0f]);

        // encode bytes '\x04\x00'
        let message: [u8; 2] = [0x04, 0x00];
        let encoded = {
            let mut buf = vec![];
            message.encode(&mut buf);
            buf
        };
        assert_eq!(encoded, vec![RLP_NULL + 2, 0x04, 0x00]);
    }

    #[test]
    fn can_encode_strings() {
        // encode dog
        let message = "dog";
        let encoded = {
            let mut buf = vec![];
            message.encode(&mut buf);
            buf
        };
        let expected: [u8; 4] = [RLP_NULL + 3, b'd', b'o', b'g'];
        assert_eq!(encoded, expected);

        // encode empty string
        let message = "";
        let encoded = {
            let mut buf = vec![];
            message.encode(&mut buf);
            buf
        };
        let expected: [u8; 1] = [RLP_NULL];
        assert_eq!(encoded, expected);
    }

    #[test]
    fn can_encode_lists_of_str() {
        // encode ["cat", "dog"]
        let message = vec!["cat", "dog"];
        let encoded = {
            let mut buf = vec![];
            message.encode(&mut buf);
            buf
        };
        let expected: [u8; 9] = [0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g'];
        assert_eq!(encoded, expected);

        // encode empty list
        let message: Vec<&str> = vec![];
        let encoded = {
            let mut buf = vec![];
            message.encode(&mut buf);
            buf
        };
        let expected: [u8; 1] = [RLP_EMPTY_LIST];
        assert_eq!(encoded, expected);
    }

    #[test]
    fn can_encode_ip() {
        // encode an IPv4 address
        let message = "192.168.0.1";
        let ip: IpAddr = message.parse().unwrap();
        let encoded = {
            let mut buf = vec![];
            ip.encode(&mut buf);
            buf
        };
        let expected: [u8; 5] = [RLP_NULL + 4, 192, 168, 0, 1];
        assert_eq!(encoded, expected);

        // encode an IPv6 address
        let message = "2001:0000:130F:0000:0000:09C0:876A:130B";
        let ip: IpAddr = message.parse().unwrap();
        let encoded = {
            let mut buf = vec![];
            ip.encode(&mut buf);
            buf
        };
        let expected: [u8; 17] = [
            0x90, 0x20, 0x01, 0x00, 0x00, 0x13, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x09, 0xc0, 0x87,
            0x6a, 0x13, 0x0b,
        ];
        assert_eq!(encoded, expected);
    }

    #[test]
    fn can_encode_addresses() {
        let address = Address::from(hex!("ef2d6d194084c2de36e0dabfce45d046b37d1106"));
        let encoded = {
            let mut buf = vec![];
            address.encode(&mut buf);
            buf
        };
        let expected = hex!("94ef2d6d194084c2de36e0dabfce45d046b37d1106");
        assert_eq!(encoded, expected);
    }

    #[test]
    fn can_encode_u256() {
        let mut encoded = Vec::new();
        U256::from(1).encode(&mut encoded);
        assert_eq!(encoded, vec![1]);

        let mut encoded = Vec::new();
        U256::from(128).encode(&mut encoded);
        assert_eq!(encoded, vec![0x80 + 1, 128]);

        let mut encoded = Vec::new();
        U256::max_value().encode(&mut encoded);
        let bytes = [0xff; 32];
        let mut expected: Vec<u8> = bytes.into();
        expected.insert(0, 0x80 + 32);
        assert_eq!(encoded, expected);
    }

    #[test]
    fn can_encode_tuple() {
        // TODO: check if works for tuples with total length greater than 55 bytes
        let tuple: (u8, u8) = (0x01, 0x02);
        let mut encoded = Vec::new();
        tuple.encode(&mut encoded);
        let expected = vec![0xc0 + 2, 0x01, 0x02];
        assert_eq!(encoded, expected);
    }
}
