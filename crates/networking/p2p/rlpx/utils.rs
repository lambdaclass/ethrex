use ethereum_rust_core::H512;
use ethereum_rust_rlp::error::{RLPDecodeError, RLPEncodeError};
use k256::{
    elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint},
    EncodedPoint, PublicKey, SecretKey,
};
use snap::raw::{max_compress_len, Decoder as SnappyDecoder, Encoder as SnappyEncoder};

pub fn sha256(data: &[u8]) -> [u8; 32] {
    use k256::sha2::Digest;
    k256::sha2::Sha256::digest(data).into()
}

pub fn sha256_hmac(key: &[u8], inputs: &[&[u8]], size_data: &[u8]) -> [u8; 32] {
    use hmac::Mac;
    use k256::sha2::Sha256;

    let mut hasher = hmac::Hmac::<Sha256>::new_from_slice(key).unwrap();
    for input in inputs {
        hasher.update(input);
    }
    hasher.update(size_data);
    hasher.finalize().into_bytes().into()
}

pub fn ecdh_xchng(secret_key: &SecretKey, public_key: &PublicKey) -> [u8; 32] {
    k256::ecdh::diffie_hellman(secret_key.to_nonzero_scalar(), public_key.as_affine())
        .raw_secret_bytes()[..32]
        .try_into()
        .unwrap()
}

pub fn kdf(secret: &[u8], output: &mut [u8]) {
    // We don't use the `other_info` field
    concat_kdf::derive_key_into::<k256::sha2::Sha256>(secret, &[], output).unwrap();
}

/// Computes recipient id from public key.
pub fn pubkey2id(pk: &PublicKey) -> H512 {
    let encoded = pk.to_encoded_point(false);
    let bytes = encoded.as_bytes();
    debug_assert_eq!(bytes[0], 4);
    H512::from_slice(&bytes[1..])
}

/// Computes public key from recipient id.
/// The node ID is the uncompressed public key of a node, with the first byte omitted (0x04).
pub fn id2pubkey(id: H512) -> Option<PublicKey> {
    let point = EncodedPoint::from_untagged_bytes(&id.0.into());
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
    fn id2pubkey_pubkey2id_smoke_test() {
        use rand::rngs::OsRng;

        let sk = SecretKey::random(&mut OsRng);
        let pk = sk.public_key();
        let id = pubkey2id(&pk);
        let _pk2 = id2pubkey(id).unwrap();
    }
}
