//! Bit-level helpers. Bits are `Vec<u8>` of 0/1 values, MSB-first,
//! matching the spec's readability-first representation.

use crate::error::BinaryTrieError;

/// Expand each byte into eight bits, most significant bit first.
///
/// Sized up front rather than collected from an iterator: `flat_map`
/// reports no size hint, so `collect` would grow the vector through
/// seven or eight reallocations and land at roughly twice the needed
/// capacity.
pub fn bytes_to_bits(data: &[u8]) -> Vec<u8> {
    let mut bits = vec![0u8; data.len() * 8];
    for (byte, chunk) in data.iter().zip(bits.chunks_exact_mut(8)) {
        for (offset, bit) in chunk.iter_mut().enumerate() {
            *bit = (byte >> (7 - offset)) & 1;
        }
    }
    bits
}

/// Inverse of [`encode_bit_prefix`]: read the bit count, unpack that
/// many bits, and report how many bytes were consumed.
///
/// Rejects padding bits that are not zero. The encoder never sets them,
/// so accepting them would give one node two valid encodings — and
/// therefore two hashes.
pub(super) fn decode_bit_prefix(data: &[u8]) -> Result<(Vec<u8>, usize), BinaryTrieError> {
    let count_bytes: [u8; 2] = data
        .get(..2)
        .ok_or(BinaryTrieError::MalformedNode("prefix length truncated"))?
        .try_into()
        .expect("slice of two bytes");
    let count = u16::from_be_bytes(count_bytes) as usize;

    let packed_len = count.div_ceil(8);
    let packed = data
        .get(2..2 + packed_len)
        .ok_or(BinaryTrieError::MalformedNode("prefix bits truncated"))?;

    let mut bits = vec![0u8; count];
    for (i, bit) in bits.iter_mut().enumerate() {
        *bit = (packed[i / 8] >> (7 - i % 8)) & 1;
    }
    if let Some(last) = packed.last()
        && !count.is_multiple_of(8)
        && last & (0xff >> (count % 8)) != 0
    {
        return Err(BinaryTrieError::MalformedNode("non-zero prefix padding"));
    }
    Ok((bits, 2 + packed_len))
}

/// Encode a branch prefix: a two-byte big-endian bit count followed by
/// the bits packed MSB-first, zero-padded to a byte boundary.
///
/// The explicit count keeps the encoding injective: without it, two
/// prefixes differing only in trailing zero bits would pack to the
/// same bytes and two different trees could share a root.
pub fn encode_bit_prefix(prefix: &[u8]) -> Vec<u8> {
    debug_assert!(prefix.iter().all(|b| *b <= 1), "prefix bits must be 0 or 1");
    assert!(
        prefix.len() < 1 << 16,
        "prefix bit count must fit in two bytes"
    );
    let mut out = vec![0u8; 2 + prefix.len().div_ceil(8)];
    out[..2].copy_from_slice(&(prefix.len() as u16).to_be_bytes());
    for (i, bit) in prefix.iter().enumerate() {
        out[2 + i / 8] |= bit << (7 - i % 8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_bits_msb_first() {
        assert_eq!(bytes_to_bits(&[0b1010_0001]), vec![1, 0, 1, 0, 0, 0, 0, 1]);
        assert_eq!(bytes_to_bits(&[]), Vec::<u8>::new());
        assert_eq!(bytes_to_bits(&[0x80, 0x01])[0], 1);
        assert_eq!(bytes_to_bits(&[0x80, 0x01])[15], 1);
    }

    #[test]
    fn encode_bit_prefix_empty() {
        assert_eq!(encode_bit_prefix(&[]), vec![0x00, 0x00]);
    }

    #[test]
    fn encode_bit_prefix_packs_msb_first_and_pads() {
        assert_eq!(encode_bit_prefix(&[1, 0, 1]), vec![0x00, 0x03, 0b1010_0000]);
        let nine = vec![1, 1, 1, 1, 1, 1, 1, 1, 1];
        assert_eq!(encode_bit_prefix(&nine), vec![0x00, 0x09, 0xff, 0x80]);
    }

    #[test]
    fn encode_bit_prefix_is_injective_on_trailing_zeros() {
        assert_ne!(encode_bit_prefix(&[1]), encode_bit_prefix(&[1, 0]));
    }
}
