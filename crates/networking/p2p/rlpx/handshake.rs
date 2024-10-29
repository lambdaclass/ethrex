use crate::rlpx::utils::{ecdh_xchng, id2pubkey, kdf, pubkey2id, sha256, sha256_hmac};
use aes::cipher::{KeyIvInit, StreamCipher};
use ethereum_rust_core::{Signature, H128, H256, H512};
use ethereum_rust_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use k256::{
    ecdsa::{self, RecoveryId, SigningKey, VerifyingKey},
    elliptic_curve::sec1::ToEncodedPoint,
    PublicKey, SecretKey,
};
use rand::Rng;

type Aes128Ctr64BE = ctr::Ctr64BE<aes::Aes128>;

/// Encodes an Auth message, to start a handshake.
pub fn encode_auth_message(
    static_key: &SecretKey,
    local_nonce: H256,
    remote_static_pubkey: &PublicKey,
    local_ephemeral_key: &SecretKey,
) -> Vec<u8> {
    let node_id = pubkey2id(&static_key.public_key());

    // Derive a shared secret from the static keys.
    let static_shared_secret = ecdh_xchng(static_key, remote_static_pubkey);

    // Create the signature included in the message.
    let signature = sign_shared_secret(
        static_shared_secret.into(),
        local_nonce,
        local_ephemeral_key,
    );

    // Compose the auth message.
    let auth = AuthMessage::new(signature, node_id, local_nonce);

    // RLP-encode the message.
    let encoded_auth_msg = auth.encode_to_vec();

    encrypt_message(remote_static_pubkey, encoded_auth_msg)
}

/// Decodes an incomming Auth message, starting a handshake.
pub(crate) fn decode_auth_message(
    static_key: &SecretKey,
    msg: &[u8],
    auth_data: [u8; 2],
) -> (AuthMessage, PublicKey) {
    let payload = decrypt_message(static_key, msg, auth_data);

    // RLP-decode the message.
    let (auth, _padding) = AuthMessage::decode_unfinished(&payload).unwrap();

    // Derive a shared secret from the static keys.
    let peer_pk = id2pubkey(auth.node_id).unwrap();
    let static_shared_secret = ecdh_xchng(static_key, &peer_pk);
    let remote_ephemeral_key =
        retrieve_remote_ephemeral_key(static_shared_secret.into(), auth.nonce, auth.signature);
    (auth, remote_ephemeral_key)
}

/// Encodes an Ack message, to complete a handshake
pub fn encode_ack_message(
    local_ephemeral_key: &SecretKey,
    local_nonce: H256,
    remote_static_pubkey: &PublicKey,
) -> Vec<u8> {
    // Compose the ack message.
    let ack_msg = AckMessage::new(pubkey2id(&local_ephemeral_key.public_key()), local_nonce);

    // RLP-encode the message.
    let encoded_ack_msg = ack_msg.encode_to_vec();

    encrypt_message(remote_static_pubkey, encoded_ack_msg)
}

/// Decodes an Ack message, completing a handshake.
pub(crate) fn decode_ack_message(
    static_key: &SecretKey,
    msg: &[u8],
    auth_data: [u8; 2],
) -> AckMessage {
    let payload = decrypt_message(static_key, msg, auth_data);

    // RLP-decode the message.
    let (ack, _padding) = AckMessage::decode_unfinished(&payload).unwrap();

    ack
}

fn decrypt_message(static_key: &SecretKey, msg: &[u8], auth_data: [u8; 2]) -> Vec<u8> {
    // Split the message into its components. General layout is:
    // public-key (65) || iv (16) || ciphertext || mac (32)
    let (pk, rest) = msg.split_at(65);
    let (iv, rest) = rest.split_at(16);
    let (c, d) = rest.split_at(rest.len() - 32);

    // Derive the message shared secret.
    let shared_secret = ecdh_xchng(static_key, &PublicKey::from_sec1_bytes(pk).unwrap());

    // Derive the AES and MAC keys from the message shared secret.
    let mut buf = [0; 32];
    kdf(&shared_secret, &mut buf);
    let aes_key = &buf[..16];
    let mac_key = sha256(&buf[16..]);

    // Verify the MAC.
    let expected_d = sha256_hmac(&mac_key, &[iv, c], &auth_data);
    assert_eq!(d, expected_d);

    // Decrypt the message with the AES key.
    let mut stream_cipher = Aes128Ctr64BE::new_from_slices(aes_key, iv).unwrap();
    let mut decoded = c.to_vec();
    stream_cipher.try_apply_keystream(&mut decoded).unwrap();
    decoded
}

pub fn encrypt_message(remote_static_pubkey: &PublicKey, mut encoded_msg: Vec<u8>) -> Vec<u8> {
    const SIGNATURE_SIZE: usize = 65;
    const IV_SIZE: usize = 16;
    const MAC_FOOTER_SIZE: usize = 32;

    let mut rng = rand::thread_rng();

    // Pad with random amount of data. the amount needs to be at least 100 bytes to make
    // the message distinguishable from pre-EIP-8 handshakes.
    let padding_length = rng.gen_range(100..=300);
    encoded_msg.resize(encoded_msg.len() + padding_length, 0);

    // Precompute the size of the message. This is needed for computing the MAC.
    let ecies_overhead = SIGNATURE_SIZE + IV_SIZE + MAC_FOOTER_SIZE;
    let auth_size: u16 = (encoded_msg.len() + ecies_overhead).try_into().unwrap();
    let auth_size_bytes = auth_size.to_be_bytes();

    // Generate a keypair just for this message.
    let message_secret_key = SecretKey::random(&mut rng);

    // Derive a shared secret for this message.
    let message_secret = ecdh_xchng(&message_secret_key, remote_static_pubkey);

    // Derive the AES and MAC keys from the message secret.
    let mut secret_keys = [0; 32];
    kdf(&message_secret, &mut secret_keys);
    let aes_key = &secret_keys[..16];
    let mac_key = sha256(&secret_keys[16..]);

    // Use the AES secret to encrypt the auth message.
    let iv = H128::random_using(&mut rng);
    let mut aes_cipher = Aes128Ctr64BE::new_from_slices(aes_key, &iv.0).unwrap();
    aes_cipher.try_apply_keystream(&mut encoded_msg).unwrap();
    let encrypted_auth_msg = encoded_msg;

    // Use the MAC secret to compute the MAC.
    let r_public_key = message_secret_key.public_key().to_encoded_point(false);
    let mac_footer = sha256_hmac(&mac_key, &[&iv.0, &encrypted_auth_msg], &auth_size_bytes);

    // Return the message
    [
        &auth_size_bytes,
        r_public_key.as_bytes(),
        &iv.0,
        &encrypted_auth_msg,
        &mac_footer,
    ]
    .concat()
}

fn retrieve_remote_ephemeral_key(
    shared_secret: H256,
    remote_nonce: H256,
    signature: Signature,
) -> PublicKey {
    let signature_prehash = shared_secret ^ remote_nonce;
    let sign = ecdsa::Signature::from_slice(&signature.to_fixed_bytes()[..64]).unwrap();
    let rid = RecoveryId::from_byte(signature[64]).unwrap();
    let ephemeral_key =
        VerifyingKey::recover_from_prehash(signature_prehash.as_bytes(), &sign, rid).unwrap();
    ephemeral_key.into()
}

fn sign_shared_secret(
    shared_secret: H256,
    local_nonce: H256,
    local_ephemeral_key: &SecretKey,
) -> Signature {
    let signature_prehash = shared_secret ^ local_nonce;
    let (signature, rid) = SigningKey::from(local_ephemeral_key)
        .sign_prehash_recoverable(&signature_prehash.0)
        .unwrap();
    let mut signature_bytes = [0; 65];
    signature_bytes[..64].copy_from_slice(signature.to_bytes().as_slice());
    signature_bytes[64] = rid.to_byte();
    signature_bytes.into()
}

#[derive(Debug)]
pub(crate) struct AuthMessage {
    /// The signature of the message.
    /// The signed data is `static-shared-secret ^ initiator-nonce`.
    pub signature: Signature,
    /// The node ID of the initiator.
    pub node_id: H512,
    /// The nonce generated by the initiator.
    pub nonce: H256,
    /// The version of RLPx used by the sender.
    /// The current version is 5.
    pub version: u8,
}

impl AuthMessage {
    pub fn new(signature: Signature, node_id: H512, nonce: H256) -> Self {
        Self {
            signature,
            node_id,
            nonce,
            version: 5,
        }
    }
}

impl RLPEncode for AuthMessage {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.signature)
            .encode_field(&self.node_id)
            .encode_field(&self.nonce)
            .encode_field(&self.version)
            .finish()
    }
}

impl RLPDecode for AuthMessage {
    // NOTE: discards any extra data in the list after the known fields.
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp).unwrap();
        let (signature, decoder) = decoder.decode_field("signature").unwrap();
        let (node_id, decoder) = decoder.decode_field("node_id").unwrap();
        let (nonce, decoder) = decoder.decode_field("nonce").unwrap();
        let (version, decoder) = decoder.decode_field("version").unwrap();

        let rest = decoder.finish_unchecked();
        let this = Self {
            signature,
            node_id,
            nonce,
            version,
        };
        Ok((this, rest))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AckMessage {
    /// The recipient's ephemeral public key.
    pub ephemeral_pubkey: H512,
    /// The nonce generated by the recipient.
    pub nonce: H256,
    /// The version of RLPx used by the recipient.
    /// The current version is 5.
    pub version: u8,
}

impl AckMessage {
    pub fn new(ephemeral_pubkey: H512, nonce: H256) -> Self {
        Self {
            ephemeral_pubkey,
            nonce,
            version: 5,
        }
    }

    pub fn get_ephemeral_pubkey(&self) -> Option<PublicKey> {
        id2pubkey(self.ephemeral_pubkey)
    }
}

impl RLPEncode for AckMessage {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.ephemeral_pubkey)
            .encode_field(&self.nonce)
            .encode_field(&self.version)
            .finish()
    }
}

impl RLPDecode for AckMessage {
    // NOTE: discards any extra data in the list after the known fields.
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp).unwrap();
        let (ephemeral_pubkey, decoder) = decoder.decode_field("ephemeral_pubkey").unwrap();
        let (nonce, decoder) = decoder.decode_field("nonce").unwrap();
        let (version, decoder) = decoder.decode_field("version").unwrap();

        let rest = decoder.finish_unchecked();
        let this = Self {
            ephemeral_pubkey,
            nonce,
            version,
        };
        Ok((this, rest))
    }
}
