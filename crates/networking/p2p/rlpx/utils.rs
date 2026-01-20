use ethrex_common::H512;
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use secp256k1::ecdh::shared_secret_point;
use secp256k1::{PublicKey, SecretKey};
use sha2::{Digest, Sha256};
use snap::raw::{Decoder as SnappyDecoder, Encoder as SnappyEncoder, max_compress_len};
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
    let compressed_size = snappy_encoder.compress(&encoded_data, &mut msg_data)?;

    msg_data.truncate(compressed_size);
    Ok(msg_data)
}

pub fn snappy_decompress(msg_data: &[u8]) -> Result<Vec<u8>, RLPDecodeError> {
    let mut snappy_decoder = SnappyDecoder::new();
    Ok(snappy_decoder.decompress_vec(msg_data)?)
}
