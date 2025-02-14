use aes::cipher::{KeyIvInit, StreamCipher};
use ethrex_core::{H128, H256, H520};
use k256::{
    ecdsa::{self, RecoveryId, SigningKey, VerifyingKey},
    elliptic_curve::sec1::ToEncodedPoint,
    PublicKey, SecretKey,
};

pub enum Error {
    KDFError,
    InvalidLength,
    InvalidBytes,
    InvalidRecoveryId,
    CipherError,
}

pub type Result<T> = std::result::Result<T, Error>;

type Aes128Ctr64BE = ctr::Ctr64BE<aes::Aes128>;

pub fn ecdh_xchng(secret_key: &SecretKey, public_key: &PublicKey) -> [u8; 32] {
    k256::ecdh::diffie_hellman(secret_key.to_nonzero_scalar(), public_key.as_affine())
        .raw_secret_bytes()[..32]
        .try_into()
        .unwrap()
}

pub fn kdf(secret: &[u8], output: &mut [u8]) -> Result<()> {
    // We don't use the `other_info` field
    concat_kdf::derive_key_into::<k256::sha2::Sha256>(secret, &[], output)
        .map_err(|_| Error::KDFError)
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    use k256::sha2::Digest;
    k256::sha2::Sha256::digest(data).into()
}

pub fn sha256_hmac(key: &[u8], inputs: &[&[u8]], size_data: &[u8]) -> Result<[u8; 32]> {
    use hmac::Mac;
    use k256::sha2::Sha256;

    let mut hasher = hmac::Hmac::<Sha256>::new_from_slice(key).map_err(|_| Error::InvalidLength)?;
    for input in inputs {
        hasher.update(input);
    }
    hasher.update(size_data);
    Ok(hasher.finalize().into_bytes().into())
}

pub fn decrypt_message(static_key: &SecretKey, msg: &[u8], size_data: &[u8]) -> Result<Vec<u8>> {
    // Split the message into its components. General layout is:
    // public-key (65) || iv (16) || ciphertext || mac (32)
    let (pk, rest) = msg.split_at(65);
    let (iv, rest) = rest.split_at(16);
    let (c, d) = rest.split_at(rest.len() - 32);

    // Derive the message shared secret.
    let shared_secret = ecdh_xchng(
        static_key,
        &PublicKey::from_sec1_bytes(pk).map_err(|_| Error::InvalidBytes)?,
    );

    // Derive the AES and MAC keys from the message shared secret.
    let mut buf = [0; 32];
    kdf(&shared_secret, &mut buf)?;
    let aes_key = &buf[..16];
    let mac_key = sha256(&buf[16..]);

    // Verify the MAC.
    let expected_d = sha256_hmac(&mac_key, &[iv, c], size_data)?;

    if d != expected_d {
        return Err(Error::InvalidBytes);
    }

    // Decrypt the message with the AES key.
    let mut stream_cipher =
        Aes128Ctr64BE::new_from_slices(aes_key, iv).map_err(|_| Error::InvalidLength)?;
    let mut decoded = c.to_vec();
    stream_cipher
        .try_apply_keystream(&mut decoded)
        .map_err(|_| Error::CipherError)?;

    Ok(decoded)
}

pub fn encrypt_message(
    remote_static_pubkey: &PublicKey,
    mut encoded_msg: Vec<u8>,
) -> Result<Vec<u8>> {
    const SIGNATURE_SIZE: usize = 65;
    const IV_SIZE: usize = 16;
    const MAC_FOOTER_SIZE: usize = 32;

    let mut rng = rand::thread_rng();

    // Precompute the size of the message. This is needed for computing the MAC.
    let ecies_overhead = SIGNATURE_SIZE + IV_SIZE + MAC_FOOTER_SIZE;
    let auth_size: u16 = (encoded_msg.len() + ecies_overhead)
        .try_into()
        .map_err(|_| Error::InvalidLength)?;
    let auth_size_bytes = auth_size.to_be_bytes();

    // Generate a keypair just for this message.
    let message_secret_key = SecretKey::random(&mut rng);

    // Derive a shared secret for this message.
    let message_secret = ecdh_xchng(&message_secret_key, remote_static_pubkey);

    // Derive the AES and MAC keys from the message secret.
    let mut secret_keys = [0; 32];
    kdf(&message_secret, &mut secret_keys)?;
    let aes_key = &secret_keys[..16];
    let mac_key = sha256(&secret_keys[16..]);

    // Use the AES secret to encrypt the auth message.
    let iv = H128::random_using(&mut rng);
    let mut aes_cipher =
        Aes128Ctr64BE::new_from_slices(aes_key, &iv.0).map_err(|_| Error::InvalidLength)?;
    aes_cipher
        .try_apply_keystream(&mut encoded_msg)
        .map_err(|_| Error::CipherError)?;
    let encrypted_auth_msg = encoded_msg;

    // Use the MAC secret to compute the MAC.
    let r_public_key = message_secret_key.public_key().to_encoded_point(false);
    let mac_footer = sha256_hmac(&mac_key, &[&iv.0, &encrypted_auth_msg], &auth_size_bytes)?;

    // Return the message
    Ok([
        &auth_size_bytes,
        r_public_key.as_bytes(),
        &iv.0,
        &encrypted_auth_msg,
        &mac_footer,
    ]
    .concat())
}

pub fn retrieve_remote_ephemeral_key(
    shared_secret: H256,
    remote_nonce: H256,
    signature: H520,
) -> Result<PublicKey> {
    let signature_prehash = shared_secret ^ remote_nonce;
    let sign = ecdsa::Signature::from_slice(&signature[..64]).map_err(|_| Error::InvalidBytes)?;
    let rid = RecoveryId::from_byte(signature[64]).ok_or(Error::InvalidRecoveryId)?;
    let ephemeral_key =
        VerifyingKey::recover_from_prehash(signature_prehash.as_bytes(), &sign, rid)
            .map_err(|_| Error::InvalidBytes)?;

    Ok(ephemeral_key.into())
}

pub fn sign_shared_secret(
    shared_secret: H256,
    local_nonce: H256,
    local_ephemeral_key: &SecretKey,
) -> Result<H520> {
    let signature_prehash = shared_secret ^ local_nonce;
    let (signature, rid) = SigningKey::from(local_ephemeral_key)
        .sign_prehash_recoverable(&signature_prehash.0)
        .map_err(|_| Error::CipherError)?;
    let mut signature_bytes = [0; 65];
    signature_bytes[..64].copy_from_slice(signature.to_bytes().as_slice());
    signature_bytes[64] = rid.to_byte();
    Ok(signature_bytes.into())
}
