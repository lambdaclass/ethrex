use ethrex_common::H512;
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use secp256k1::ecdh::shared_secret_point;
use secp256k1::{PublicKey, SecretKey};
use sha2::{Digest, Sha256};
use snap::raw::{
    Decoder as SnappyDecoder, Encoder as SnappyEncoder, decompress_len, max_compress_len,
};
use std::array::TryFromSliceError;

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}
use crate::rlpx::error::CryptographyError;

pub fn sha256_hmac(
    key: &[u8],
    inputs: &[&[u8]],
    size_data: &[u8],
) -> Result<[u8; 32], CryptographyError> {
    use hmac::Mac;
    use sha2::Sha256;

    let mut hasher = hmac::Hmac::<Sha256>::new_from_slice(key)
        .map_err(|error| CryptographyError::InvalidKey(error.to_string()))?;
    for input in inputs {
        hasher.update(input);
    }
    hasher.update(size_data);
    Ok(hasher.finalize().into_bytes().into())
}

pub fn ecdh_xchng(
    secret_key: &SecretKey,
    public_key: &PublicKey,
) -> Result<[u8; 32], CryptographyError> {
    let point = shared_secret_point(public_key, secret_key);
    point[..32].try_into().map_err(|error: TryFromSliceError| {
        CryptographyError::InvalidGeneratedSecret(error.to_string())
    })
}

pub fn kdf(secret: &[u8], output: &mut [u8]) -> Result<(), CryptographyError> {
    // We don't use the `other_info` field
    concat_kdf::derive_key_into::<sha2::Sha256>(secret, &[], output)
        .map_err(|error| CryptographyError::CouldNotGetKeyFromSecret(error.to_string()))
}

/// Decompresses the received public key
pub fn decompress_pubkey(pk: &PublicKey) -> H512 {
    let bytes = pk.serialize_uncompressed();
    debug_assert_eq!(bytes[0], 4);
    H512::from_slice(&bytes[1..])
}

/// Compresses the received public key
/// The received value is the uncompressed public key of a node, with the first byte omitted (0x04).
pub fn compress_pubkey(pk: H512) -> Option<PublicKey> {
    let mut full_pk = [0u8; 65];
    full_pk[0] = 0x04;
    full_pk[1..].copy_from_slice(&pk.0);
    PublicKey::from_slice(&full_pk).ok()
}

pub fn snappy_compress(encoded_data: Vec<u8>) -> Result<Vec<u8>, RLPEncodeError> {
    let mut snappy_encoder = SnappyEncoder::new();
    let mut msg_data = vec![0; max_compress_len(encoded_data.len()) + 1];
    let compressed_size = snappy_encoder
        .compress(&encoded_data, &mut msg_data)
        .map_err(|e| RLPEncodeError::InvalidCompression(e.to_string()))?;

    msg_data.truncate(compressed_size);
    Ok(msg_data)
}

/// Maximum decompressed size accepted for a snappy-compressed RLPx message. Matches the
/// compressed-frame cap (`MAX_MESSAGE_SIZE` = `0xFFFFFF`, ~16 MiB, in `connection/codec.rs`)
/// and go-ethereum, which likewise bounds `snappy.DecodedLen` at maxUint24 before allocating.
const MAX_SNAPPY_DECOMPRESSED_LEN: usize = 0xFF_FFFF;

pub fn snappy_decompress(msg_data: &[u8]) -> Result<Vec<u8>, RLPDecodeError> {
    snappy_decompress_bounded(msg_data, MAX_SNAPPY_DECOMPRESSED_LEN)
}

/// Like [`snappy_decompress`] but rejects a declared decompressed length above `max_len` before
/// allocating, for messages with a tighter natural bound than the global frame cap. `max_len` is
/// clamped to `MAX_SNAPPY_DECOMPRESSED_LEN` so it can never exceed the global limit.
///
/// RLPx uses *raw* (block) snappy, which is one-shot: the block header declares the full
/// decompressed length and `decompress_vec` produces the whole buffer in a single allocation.
pub fn snappy_decompress_bounded(
    msg_data: &[u8],
    max_len: usize,
) -> Result<Vec<u8>, RLPDecodeError> {
    let max_len = max_len.min(MAX_SNAPPY_DECOMPRESSED_LEN);
    // The declared length is authoritative for raw snappy: `decompress_vec` allocates exactly
    // this many bytes *before* validating the body, and a body that doesn't decompress to it
    // fails — so a peer can't declare small then deliver large. Reject an over-large declared
    // length up front, otherwise a tiny frame forces a giant allocation (only the compressed
    // frame is capped elsewhere, not the decoded size).
    let declared_len =
        decompress_len(msg_data).map_err(|e| RLPDecodeError::InvalidCompression(e.to_string()))?;
    if declared_len > max_len {
        return Err(RLPDecodeError::InvalidCompression(format!(
            "decompressed length {declared_len} exceeds maximum {max_len}"
        )));
    }
    let mut snappy_decoder = SnappyDecoder::new();
    snappy_decoder
        .decompress_vec(msg_data)
        .map_err(|e| RLPDecodeError::InvalidCompression(e.to_string()))
}

/// The first `len` bytes a raw snappy block decompresses to (all of it, if shorter), without
/// decompressing the rest. Lets a caller read a message's leading fields, such as a response's
/// request id, before deciding whether the whole message is worth decompressing.
///
/// Raw snappy is a varint of the decompressed length followed by elements, each a literal run
/// or a copy of earlier output. Elements are decoded in order until `len` bytes are out.
pub fn snappy_decompress_prefix(data: &[u8], len: usize) -> Result<Vec<u8>, RLPDecodeError> {
    fn malformed(what: &str) -> RLPDecodeError {
        RLPDecodeError::InvalidCompression(format!("snappy block: {what}"))
    }
    fn le(bytes: &[u8]) -> usize {
        bytes
            .iter()
            .rev()
            .fold(0, |value, byte| (value << 8) | usize::from(*byte))
    }

    // Decompressed length: a varint of at most five bytes.
    let mut total: usize = 0;
    let mut pos = 0;
    loop {
        let byte = *data.get(pos).ok_or_else(|| malformed("truncated length"))?;
        total |= usize::from(byte & 0x7f) << (7 * pos);
        pos += 1;
        if byte & 0x80 == 0 {
            break;
        }
        if pos == 5 {
            return Err(malformed("length varint too long"));
        }
    }
    if total > MAX_SNAPPY_DECOMPRESSED_LEN {
        return Err(malformed("decompressed length exceeds the maximum"));
    }

    let want = len.min(total);
    let mut out = Vec::with_capacity(want);
    while out.len() < want {
        let tag = *data
            .get(pos)
            .ok_or_else(|| malformed("truncated element"))?;
        pos += 1;
        // Bytes the element holds after the tag, and its length and copy offset.
        let (extra, run, offset) = match tag & 3 {
            0 => {
                let short = usize::from(tag >> 2);
                let len_bytes = short.saturating_sub(59);
                let len_field = data
                    .get(pos..pos + len_bytes)
                    .ok_or_else(|| malformed("truncated literal length"))?;
                let run = if len_bytes == 0 { short } else { le(len_field) } + 1;
                pos += len_bytes;
                let literal = data
                    .get(pos..pos + run)
                    .ok_or_else(|| malformed("truncated literal"))?;
                // A literal can run for a whole 64 KiB block; take only what is wanted.
                out.extend_from_slice(&literal[..run.min(want - out.len())]);
                pos += run;
                continue;
            }
            1 => {
                let low = *data.get(pos).ok_or_else(|| malformed("truncated copy"))?;
                (
                    1,
                    4 + usize::from((tag >> 2) & 7),
                    (usize::from(tag >> 5) << 8) | usize::from(low),
                )
            }
            2 => {
                let field = data
                    .get(pos..pos + 2)
                    .ok_or_else(|| malformed("truncated copy"))?;
                (2, 1 + usize::from(tag >> 2), le(field))
            }
            _ => {
                let field = data
                    .get(pos..pos + 4)
                    .ok_or_else(|| malformed("truncated copy"))?;
                (4, 1 + usize::from(tag >> 2), le(field))
            }
        };
        pos += extra;
        if offset == 0 || offset > out.len() {
            return Err(malformed("copy offset outside the output"));
        }
        // Copies may overlap their own output, so go byte by byte.
        for _ in 0..run.min(want - out.len()) {
            out.push(out[out.len() - offset]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snappy stream begins with a varint of the decompressed length. A peer can declare a
    /// huge length (up to ~4 GiB) in a tiny frame, and `decompress_vec` allocates that buffer
    /// before validating the body. `snappy_decompress` must reject an over-large declared
    /// length *before* allocating, so a small compressed frame can't force a giant allocation.
    #[test]
    fn snappy_decompress_rejects_oversized_declared_length() {
        // LEB128 varint of 100 MiB — far above the 16 MiB (0xFFFFFF) frame cap.
        let mut frame = Vec::new();
        let mut declared = 100u64 * 1024 * 1024;
        while declared >= 0x80 {
            frame.push((declared as u8) | 0x80);
            declared >>= 7;
        }
        frame.push(declared as u8);
        frame.push(0x00); // minimal (invalid) body

        let err =
            snappy_decompress(&frame).expect_err("oversized declared length must be rejected");
        let msg = format!("{err}").to_lowercase();
        assert!(
            msg.contains("exceed"),
            "expected a declared-length cap rejection, got: {msg}"
        );
    }

    /// A normal round-trip still works after the cap is added.
    #[test]
    fn snappy_roundtrip_below_cap() {
        let data = b"the quick brown fox jumps over the lazy dog".repeat(10);
        let compressed = snappy_compress(data.clone()).expect("compress");
        let out = snappy_decompress(&compressed).expect("decompress");
        assert_eq!(out, data);
    }

    /// `snappy_decompress_bounded` rejects a declared length above a caller-supplied `max_len`
    /// even when it is well under the global frame cap — the per-message bound used for
    /// `PooledTransactions`.
    #[test]
    fn snappy_decompress_bounded_rejects_above_max_len() {
        // Declare 8 MiB (under the 16 MiB global cap) but bound at 4 MiB.
        let mut frame = Vec::new();
        let mut declared = 8u64 * 1024 * 1024;
        while declared >= 0x80 {
            frame.push((declared as u8) | 0x80);
            declared >>= 7;
        }
        frame.push(declared as u8);
        frame.push(0x00); // minimal (invalid) body

        let err = snappy_decompress_bounded(&frame, 4 * 1024 * 1024)
            .expect_err("declared length above max_len must be rejected");
        assert!(
            format!("{err}").to_lowercase().contains("exceed"),
            "expected a declared-length cap rejection, got: {err}"
        );
    }

    /// A round-trip whose payload fits under the tighter bound still decodes.
    #[test]
    fn snappy_decompress_bounded_allows_below_max_len() {
        let data = b"pooled transactions payload".repeat(100);
        let compressed = snappy_compress(data.clone()).expect("compress");
        let out = snappy_decompress_bounded(&compressed, 4 * 1024 * 1024).expect("decompress");
        assert_eq!(out, data);
    }

    /// The prefix SHALL be exactly the start of the full decompression, for literals, short
    /// and overlapping copies, and lengths past the end.
    #[test]
    fn snappy_prefix_matches_full_decompression() {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut inputs: Vec<Vec<u8>> = vec![Vec::new(), vec![7], vec![0; 100_000]];
        for size in [1usize, 17, 64, 1000, 70_000] {
            // Random bytes (literals), a short repeating pattern (overlapping copies) and a
            // mix of both (copies with long offsets).
            inputs.push((0..size).map(|_| next() as u8).collect());
            inputs.push((0..size).map(|i| (i % 5) as u8).collect());
            inputs.push(
                (0..size)
                    .map(|i| {
                        if i % 97 < 60 {
                            (i % 7) as u8
                        } else {
                            next() as u8
                        }
                    })
                    .collect(),
            );
        }
        for input in inputs {
            let compressed = snappy_compress(input.clone()).unwrap();
            let full = snappy_decompress(&compressed).unwrap();
            assert_eq!(full, input);
            for len in [0, 1, 2, 18, 63, 64, 65, 4096, input.len(), input.len() + 10] {
                let expected = &full[..len.min(full.len())];
                assert_eq!(
                    snappy_decompress_prefix(&compressed, len).unwrap(),
                    expected,
                    "prefix of {len} bytes of a {}-byte input",
                    input.len()
                );
            }
        }
    }
}
