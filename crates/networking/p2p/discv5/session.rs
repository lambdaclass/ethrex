use ethrex_common::H256;
use hkdf::Hkdf;
use secp256k1::{
    Message as SecpMessage, PublicKey, SECP256K1, SecretKey, ecdh::shared_secret_point,
    ecdsa::Signature,
};
use sha2::{Digest, Sha256};

/// A discv5 session
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub outbound_key: [u8; 16],
    pub inbound_key: [u8; 16],
}

/// Builds the challenge-data from a WHOAREYOU packet
pub fn build_challenge_data(masking_iv: &[u8], static_header: &[u8], authdata: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(masking_iv.len() + static_header.len() + authdata.len());
    data.extend_from_slice(masking_iv);
    data.extend_from_slice(static_header);
    data.extend_from_slice(authdata);
    data
}

/// Derives session keys from the handshake.
/// - `secret_key`: The secret key for ECDH (ephemeral for initiator, static for recipient)
/// - `public_key`: The public key for ECDH (dest static for initiator, ephemeral for recipient)
/// - `node_id_a`: The initiator's node ID
/// - `node_id_b`: The recipient's node ID
/// - `challenge_data`: The challenge data from WHOAREYOU
/// - `is_initiator`: True if we are the initiator (node A), false if recipient (node B)
pub fn derive_session_keys(
    secret_key: &SecretKey,
    public_key: &PublicKey,
    node_id_a: &H256,
    node_id_b: &H256,
    challenge_data: &[u8],
    is_initiator: bool,
) -> Session {
    let shared_secret = compressed_shared_secret(public_key, secret_key);
    let hkdf = Hkdf::<Sha256>::new(Some(challenge_data), &shared_secret);

    let mut kdf_info = b"discovery v5 key agreement".to_vec();
    kdf_info.extend_from_slice(node_id_a.as_bytes());
    kdf_info.extend_from_slice(node_id_b.as_bytes());

    let mut key_data = [0u8; 32];
    hkdf.expand(&kdf_info, &mut key_data)
        .expect("key_data is 32 bytes long, it can never fail");

    // First 16 bytes are initiator's outbound key, second 16 are recipient's outbound key
    let mut initiator_key = [0u8; 16];
    let mut recipient_key = [0u8; 16];
    initiator_key.copy_from_slice(&key_data[..16]);
    recipient_key.copy_from_slice(&key_data[16..]);

    let (outbound_key, inbound_key) = if is_initiator {
        (initiator_key, recipient_key)
    } else {
        (recipient_key, initiator_key)
    };

    Session {
        outbound_key,
        inbound_key,
    }
}

/// Signs the id-signature input used in the handshake
pub fn create_id_signature(
    static_key: &SecretKey,
    challenge_data: &[u8],
    ephemeral_pubkey: &[u8],
    node_id_b: &H256,
) -> Signature {
    /*
    *  id-signature-text  = "discovery v5 identity proof"
       id-signature-input = id-signature-text || challenge-data || ephemeral-pubkey || node-id-B
       id-signature       = id_sign(sha256(id-signature-input))
    */
    let mut id_signature_input = b"discovery v5 identity proof".to_vec();
    id_signature_input.extend_from_slice(challenge_data);
    id_signature_input.extend_from_slice(ephemeral_pubkey);
    id_signature_input.extend_from_slice(node_id_b.as_bytes());

    let digest = Sha256::digest(&id_signature_input);
    let message = SecpMessage::from_digest_slice(&digest).expect("32 byte digest");
    SECP256K1.sign_ecdsa(&message, static_key)
}

/// Verifies the id-signature from the handshake
pub fn verify_id_signature(
    src_pubkey: &PublicKey,
    challenge_data: &[u8],
    ephemeral_pubkey: &[u8],
    node_id_b: &H256,
    signature: &Signature,
) -> bool {
    let mut id_signature_input = b"discovery v5 identity proof".to_vec();
    id_signature_input.extend_from_slice(challenge_data);
    id_signature_input.extend_from_slice(ephemeral_pubkey);
    id_signature_input.extend_from_slice(node_id_b.as_bytes());

    let digest = Sha256::digest(&id_signature_input);
    let Ok(message) = SecpMessage::from_digest_slice(&digest) else {
        return false;
    };
    SECP256K1
        .verify_ecdsa(&message, signature, src_pubkey)
        .is_ok()
}

/// Creates a secret through elliptic-curve Diffie-Hellman key agreement
///
/// ecdh(pubkey, privkey) from the spec
///
/// https://github.com/ethereum/devp2p/blob/master/discv5/discv5-theory.md#identity-specific-cryptography-in-the-handshake
fn compressed_shared_secret(dest_pubkey: &PublicKey, ephemeral_key: &SecretKey) -> [u8; 33] {
    let xy_point = shared_secret_point(dest_pubkey, ephemeral_key);
    let mut compressed = [0u8; 33];
    let y = &xy_point[32..];
    compressed[0] = if y[31] & 1 == 0 { 0x02 } else { 0x03 };
    compressed[1..].copy_from_slice(&xy_point[..32]);
    compressed
}
