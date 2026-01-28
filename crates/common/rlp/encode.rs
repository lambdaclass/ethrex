use bytes::{BufMut, Bytes};
use ethereum_types::U256;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::constants::RLP_NULL;

/// Function for encoding a value to RLP.
/// For encoding the value into a buffer directly, use [`RLPEncode::encode`].
pub fn encode<T: RLPEncode>(value: T) -> Vec<u8> {
    let mut buf = Vec::new();
    value.encode(&mut buf);
    buf
}

/// Calculates the encoded length of the given integer bit width (ilog2 value) and lsb
#[inline(always)]
const fn impl_length_integers(bits: u32, lsb: u8) -> usize {
    // bits is the ilog2 0 based result, +8 accounts for the first byte boundary
    let sig_len = (bits + 8) >> 3;
    let is_multibyte_mask = ((sig_len > 1) as usize) | ((lsb > 0x7f) as usize);
    1 + sig_len as usize * is_multibyte_mask
}

/// Computes the length needed for a given payload length
#[inline]
pub const fn list_length(payload_len: usize) -> usize {
    if payload_len < 56 {
        // short prefix
        1 + payload_len
    } else {
        // encode payload_len as big endian without leading zeros
        let be_len = payload_len.ilog2() / 8 + 1;
        // prefix + payload_len encoding size + payload bytes
        1 + be_len as usize + payload_len
    }
}

/// Computes the length needed for a given byte-string and first byte
#[inline]
pub const fn bytes_length(bytes_len: usize, first_byte: u8) -> usize {
    if bytes_len == 1 && first_byte <= 0x7f {
        return 1;
    }

    if bytes_len < 56 {
        return 1 + bytes_len; // prefix (0x80 + len) + payload
    }

    // long (>=56 bytes)
    let be_len = bytes_len.ilog2() / 8 + 1;
    1 + be_len as usize + bytes_len // prefix + len(len) + payload
}

pub trait RLPEncode {
    fn encode(&self, buf: &mut dyn BufMut);

    fn length(&self) -> usize;

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

#[inline]
fn impl_encode<const N: usize>(value_be: [u8; N], buf: &mut dyn BufMut) {
    // count leading zeros
    let mut i = 0;
    while i < N && value_be[i] == 0 {
        i += 1;
    }

    // 0, also known as null or the empty string is 0x80
    if i == N {
        buf.put_u8(RLP_NULL);
        return;
    }

    let first = value_be[i];

    // for a single byte whose value is in the [0x00, 0x7f] range, that byte is its own RLP encoding.
    if i == N - 1 && first <= 0x7f {
        buf.put_u8(first);
        return;
    }

    // if a string is 0-55 bytes long, the RLP encoding consists of a
    // single byte with value RLP_NULL (0x80) plus the length of the string followed by the string.
    let len = N - i;
    buf.put_u8(RLP_NULL + len as u8);
    buf.put_slice(&value_be[i..]);
}

impl RLPEncode for u8 {
    fn encode(&self, buf: &mut dyn BufMut) {
        impl_encode(self.to_be_bytes(), buf);
    }

    #[inline]
    fn length(&self) -> usize {
        impl_length_integers(self.checked_ilog2().unwrap_or(0), *self)
    }
}

impl RLPEncode for u16 {
    fn encode(&self, buf: &mut dyn BufMut) {
        impl_encode(self.to_be_bytes(), buf);
    }
    #[inline]
    fn length(&self) -> usize {
        impl_length_integers(self.checked_ilog2().unwrap_or(0), (self & 0xff) as u8)
    }
}

impl RLPEncode for u32 {
    fn encode(&self, buf: &mut dyn BufMut) {
        impl_encode(self.to_be_bytes(), buf);
    }

    #[inline]
    fn length(&self) -> usize {
        impl_length_integers(self.checked_ilog2().unwrap_or(0), (self & 0xff) as u8)
    }
}

impl RLPEncode for u64 {
    fn encode(&self, buf: &mut dyn BufMut) {
        impl_encode(self.to_be_bytes(), buf);
    }

    #[inline]
    fn length(&self) -> usize {
        impl_length_integers(self.checked_ilog2().unwrap_or(0), (self & 0xff) as u8)
    }
}

impl RLPEncode for usize {
    fn encode(&self, buf: &mut dyn BufMut) {
        impl_encode(self.to_be_bytes(), buf);
    }

    #[inline]
    fn length(&self) -> usize {
        impl_length_integers(self.checked_ilog2().unwrap_or(0), (self & 0xff) as u8)
    }
}

impl RLPEncode for u128 {
    fn encode(&self, buf: &mut dyn BufMut) {
        impl_encode(self.to_be_bytes(), buf);
    }

    #[inline]
    fn length(&self) -> usize {
        impl_length_integers(self.checked_ilog2().unwrap_or(0), (self & 0xff) as u8)
    }
}

impl RLPEncode for () {
    fn encode(&self, buf: &mut dyn BufMut) {
        buf.put_u8(RLP_NULL);
    }
    #[inline]
    fn length(&self) -> usize {
        1
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
                let bytes = len.to_be_bytes();
                let start = bytes.iter().position(|&x| x != 0).unwrap();
                let len = bytes.len() - start;
                buf.put_u8(0xb7 + len as u8);
                buf.put_slice(&bytes[start..]);
            }
            buf.put_slice(self);
        }
    }

    #[inline]
    fn length(&self) -> usize {
        if self.is_empty() {
            return 1;
        }
        bytes_length(self.len(), self[0])
    }
}

impl<const N: usize> RLPEncode for [u8; N] {
    #[inline]
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_ref().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        if N == 1 && self[0] <= 0x7f {
            return 1;
        }

        if N < 56 {
            return 1 + N;
        }

        // long case
        let be_len = if N == 0 {
            1
        } else {
            (N.ilog2() as usize / 8) + 1
        };

        1 + be_len + N
    }
}

impl RLPEncode for str {
    #[inline]
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for &str {
    #[inline]
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for String {
    #[inline]
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for U256 {
    fn encode(&self, buf: &mut dyn BufMut) {
        let leading_zeros_in_bytes: usize = (self.leading_zeros() / 8) as usize;
        let bytes = self.to_big_endian();
        bytes[leading_zeros_in_bytes..].encode(buf)
    }

    fn length(&self) -> usize {
        let ilog = self.bits().saturating_sub(1);
        impl_length_integers(ilog as u32, (self.low_u32() & 0xff) as u8)
    }
}

impl<T: RLPEncode> RLPEncode for Vec<T> {
    #[inline(always)]
    fn encode(&self, buf: &mut dyn BufMut) {
        if self.is_empty() {
            buf.put_u8(0xc0);
        } else {
            let payload_len: usize = self.iter().map(|item| item.length()).sum();

            encode_length(payload_len, buf);

            for item in self {
                item.encode(buf);
            }
        }
    }

    #[inline]
    fn length(&self) -> usize {
        if self.is_empty() {
            // 0xc0 (1 byte)
            return 1;
        }

        let mut payload_len = 0usize;
        for item in self {
            payload_len += item.length();
        }

        list_length(payload_len)
    }
}

#[inline]
pub fn encode_length(total_len: usize, buf: &mut dyn BufMut) {
    if total_len < 56 {
        buf.put_u8(0xc0 + total_len as u8);
    } else {
        let bytes = total_len.to_be_bytes();
        let start = bytes.iter().position(|&x| x != 0).unwrap();
        let len = bytes.len() - start;
        buf.put_u8(0xf7 + len as u8);
        buf.put_slice(&bytes[start..]);
    }
}

impl<S: RLPEncode, T: RLPEncode> RLPEncode for (S, T) {
    fn encode(&self, buf: &mut dyn BufMut) {
        super::structs::Encoder::new(buf)
            .encode_field(&self.0)
            .encode_field(&self.1)
            .finish();
    }

    #[inline]
    fn length(&self) -> usize {
        let payload_len = self.0.length() + self.1.length();
        list_length(payload_len)
    }
}

impl<S: RLPEncode, T: RLPEncode, U: RLPEncode> RLPEncode for (S, T, U) {
    fn encode(&self, buf: &mut dyn BufMut) {
        super::structs::Encoder::new(buf)
            .encode_field(&self.0)
            .encode_field(&self.1)
            .encode_field(&self.2)
            .finish();
    }

    #[inline]
    fn length(&self) -> usize {
        let payload_len = self.0.length() + self.1.length() + self.2.length();
        list_length(payload_len)
    }
}

impl<S: RLPEncode, T: RLPEncode, U: RLPEncode, V: RLPEncode> RLPEncode for (S, T, U, V) {
    fn encode(&self, buf: &mut dyn BufMut) {
        super::structs::Encoder::new(buf)
            .encode_field(&self.0)
            .encode_field(&self.1)
            .encode_field(&self.2)
            .encode_field(&self.3)
            .finish();
    }

    #[inline]
    fn length(&self) -> usize {
        let payload_len = self.0.length() + self.1.length() + self.2.length() + self.3.length();
        list_length(payload_len)
    }
}

impl<S: RLPEncode, T: RLPEncode, U: RLPEncode, V: RLPEncode, W: RLPEncode> RLPEncode
    for (S, T, U, V, W)
{
    fn encode(&self, buf: &mut dyn BufMut) {
        super::structs::Encoder::new(buf)
            .encode_field(&self.0)
            .encode_field(&self.1)
            .encode_field(&self.2)
            .encode_field(&self.3)
            .encode_field(&self.4)
            .finish();
    }

    #[inline]
    fn length(&self) -> usize {
        let payload_len =
            self.0.length() + self.1.length() + self.2.length() + self.3.length() + self.4.length();
        list_length(payload_len)
    }
}

impl RLPEncode for Ipv4Addr {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.octets().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(&self.octets())
    }
}

impl RLPEncode for Ipv6Addr {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.octets().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(&self.octets())
    }
}

impl RLPEncode for IpAddr {
    fn encode(&self, buf: &mut dyn BufMut) {
        match self {
            IpAddr::V4(ip) => ip.encode(buf),
            IpAddr::V6(ip) => ip.encode(buf),
        }
    }

    #[inline]
    fn length(&self) -> usize {
        match self {
            IpAddr::V4(ipv4_addr) => RLPEncode::length(&ipv4_addr.octets()),
            IpAddr::V6(ipv6_addr) => RLPEncode::length(&ipv6_addr.octets()),
        }
    }
}

impl RLPEncode for Bytes {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_ref().encode(buf)
    }

    fn length(&self) -> usize {
        self.as_ref().length()
    }
}

// encoding for Ethereum types

impl RLPEncode for ethereum_types::H32 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for ethereum_types::H64 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for ethereum_types::H128 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for ethereum_types::Address {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for ethereum_types::H256 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for ethereum_types::H264 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for ethereum_types::H512 {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for ethereum_types::Signature {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.as_bytes().encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(self.as_bytes())
    }
}

impl RLPEncode for ethereum_types::Bloom {
    fn encode(&self, buf: &mut dyn BufMut) {
        self.0.encode(buf)
    }

    #[inline]
    fn length(&self) -> usize {
        RLPEncode::length(&self.0)
    }
}

pub trait PayloadRLPEncode {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut);
    fn encode_payload_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_payload(&mut buf);
        buf
    }
}
