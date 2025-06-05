use crate::types::Node;
use ethrex_common::{H256, H512};
use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use k256::{
    elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint},
    EncodedPoint, PublicKey, SecretKey,
};
use sha3::{Digest, Keccak256};
use snap::raw::{max_compress_len, Decoder as SnappyDecoder, Encoder as SnappyEncoder};
use tracing::{debug, error, warn};

pub fn sha256(data: &[u8]) -> [u8; 32] {
    use k256::sha2::Digest;
    k256::sha2::Sha256::digest(data).into()
}

pub fn sha256_hmac(
    key: &[u8],
    inputs: &[&[u8]],
    size_data: &[u8],
) -> Result<[u8; 32], RLPDecodeError> {
    use hmac::Mac;
    use k256::sha2::Sha256;

    let mut hasher =
        hmac::Hmac::<Sha256>::new_from_slice(key).map_err(|_| RLPDecodeError::InvalidLength)?;
    for input in inputs {
        hasher.update(input);
    }
    hasher.update(size_data);
    Ok(hasher.finalize().into_bytes().into())
}

pub fn ecdh_xchng(
    secret_key: &SecretKey,
    public_key: &PublicKey,
) -> Result<[u8; 32], RLPDecodeError> {
    k256::ecdh::diffie_hellman(secret_key.to_nonzero_scalar(), public_key.as_affine())
        .raw_secret_bytes()[..32]
        .try_into()
        .map_err(|_| RLPDecodeError::InvalidLength)
}

pub fn kdf(secret: &[u8], output: &mut [u8]) -> Result<(), RLPDecodeError> {
    // We don't use the `other_info` field
    concat_kdf::derive_key_into::<k256::sha2::Sha256>(secret, &[], output)
        .map_err(|_| RLPDecodeError::InvalidLength)?;
    Ok(())
}

/// Cpmputes the node_id from a public key (aka computes the Keccak256 hash of the given public key)
pub fn node_id(public_key: &H512) -> H256 {
    H256(Keccak256::new_with_prefix(public_key).finalize().into())
}

/// Decompresses the received public key
pub fn decompress_pubkey(pk: &PublicKey) -> H512 {
    let encoded = pk.to_encoded_point(false);
    let bytes = encoded.as_bytes();
    debug_assert_eq!(bytes[0], 4);
    H512::from_slice(&bytes[1..])
}

/// Compresses the received public key
/// The received value is the uncompressed public key of a node, with the first byte omitted (0x04).
pub fn compress_pubkey(pk: H512) -> Option<PublicKey> {
    let point = EncodedPoint::from_untagged_bytes(&pk.0.into());
    PublicKey::from_encoded_point(&point).into_option()
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

pub(crate) fn log_peer_debug(node: &Node, text: &str) {
    debug!("[{0}]: {1}", node, text)
}

pub(crate) fn log_peer_error(node: &Node, text: &str) {
    error!("[{0}]: {1}", node, text)
}
pub(crate) fn log_peer_warn(node: &Node, text: &str) {
    warn!("[{0}]: {1}", node, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecdh_xchng_smoke_test() {
        use rand::rngs::OsRng;

        let a_sk = SecretKey::random(&mut OsRng);
        let b_sk = SecretKey::random(&mut OsRng);

        let a_sk_b_pk = ecdh_xchng(&a_sk, &b_sk.public_key());
        let b_sk_a_pk = ecdh_xchng(&b_sk, &a_sk.public_key());

        // The shared secrets should be the same.
        // The operation done is:
        //   a_sk * b_pk = a * (b * G) = b * (a * G) = b_sk * a_pk
        assert_eq!(a_sk_b_pk, b_sk_a_pk);
    }

    #[test]
    fn compress_pubkey_decompress_pubkey_smoke_test() {
        use rand::rngs::OsRng;

        let sk = SecretKey::random(&mut OsRng);
        let pk = sk.public_key();
        let id = decompress_pubkey(&pk);
        let _pk2 = compress_pubkey(id).unwrap();
    }
}
