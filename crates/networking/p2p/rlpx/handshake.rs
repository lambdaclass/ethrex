use std::net::SocketAddr;

use crate::{
    rlpx::{
        connection::{LocalState, RemoteState},
        error::RLPxError,
        utils::{ecdh_xchng, id2pubkey, kdf, log_debug, pubkey2id, sha256, sha256_hmac},
    },
    types::Node,
    P2PContext,
};
use aes::cipher::{KeyIvInit, StreamCipher};
use ethrex_core::{Signature, H128, H256, H512};
use ethrex_rlp::{
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
use sha3::{Digest, Keccak256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{connection::RLPxConnection, frame::RLPxCodec};

type Aes128Ctr64BE = ctr::Ctr64BE<aes::Aes128>;

// https://github.com/ethereum/go-ethereum/blob/master/p2p/peer.go#L44
pub const P2P_MAX_MESSAGE_SIZE: usize = 2048;

pub(crate) async fn as_receiver<S>(
    context: P2PContext,
    peer_addr: SocketAddr,
    mut stream: S,
) -> Result<RLPxConnection<S>, RLPxError>
where
    S: AsyncRead + AsyncWrite + std::marker::Unpin,
{
    let remote_state = receive_auth(&context.signer, &mut stream).await?;
    let local_state = send_ack(remote_state.node_id, &mut stream).await?;
    let hashed_nonces: [u8; 32] =
        Keccak256::digest([local_state.nonce.0, remote_state.nonce.0].concat()).into();
    let node = Node {
        ip: peer_addr.ip(),
        udp_port: peer_addr.port(),
        tcp_port: peer_addr.port(),
        node_id: remote_state.node_id,
    };
    let codec = RLPxCodec::new(&local_state, &remote_state, hashed_nonces);
    log_debug(&node, "Completed handshake!");
    Ok(RLPxConnection::new(
        context.signer,
        node,
        stream,
        codec,
        context.storage,
        context.broadcast,
    ))
}

pub(crate) async fn as_initiator<S>(
    context: P2PContext,
    node: Node,
    mut stream: S,
) -> Result<RLPxConnection<S>, RLPxError>
where
    S: AsyncRead + AsyncWrite + std::marker::Unpin,
{
    let local_state = send_auth(&context.signer, node.node_id, &mut stream).await?;
    let remote_state = receive_ack(&context.signer, node.node_id, &mut stream).await?;
    // Local node is initator
    // keccak256(nonce || initiator-nonce)
    let hashed_nonces: [u8; 32] =
        Keccak256::digest([remote_state.nonce.0, local_state.nonce.0].concat()).into();
    let codec = RLPxCodec::new(&local_state, &remote_state, hashed_nonces);
    log_debug(&node, "Completed handshake!");
    Ok(RLPxConnection::new(
        context.signer,
        node,
        stream,
        codec,
        context.storage,
        context.broadcast,
    ))
}

async fn send_auth<S: AsyncWrite + std::marker::Unpin>(
    signer: &SigningKey,
    remote_node_id: H512,
    stream: S,
) -> Result<LocalState, RLPxError> {
    let secret_key: SecretKey = signer.clone().into();
    let peer_pk = id2pubkey(remote_node_id).ok_or(RLPxError::InvalidPeerId())?;

    let local_nonce = H256::random_using(&mut rand::thread_rng());
    let local_ephemeral_key = SecretKey::random(&mut rand::thread_rng());

    let msg = encode_auth_message(&secret_key, local_nonce, &peer_pk, &local_ephemeral_key)?;
    send_handshake_msg(stream, &msg).await?;

    Ok(LocalState {
        nonce: local_nonce,
        ephemeral_key: local_ephemeral_key,
        init_message: msg,
    })
}

async fn send_ack<S: AsyncWrite + std::marker::Unpin>(
    remote_node_id: H512,
    stream: S,
) -> Result<LocalState, RLPxError> {
    let peer_pk = id2pubkey(remote_node_id).ok_or(RLPxError::InvalidPeerId())?;

    let local_nonce = H256::random_using(&mut rand::thread_rng());
    let local_ephemeral_key = SecretKey::random(&mut rand::thread_rng());

    let msg = encode_ack_message(&local_ephemeral_key, local_nonce, &peer_pk)?;
    send_handshake_msg(stream, &msg).await?;

    Ok(LocalState {
        nonce: local_nonce,
        ephemeral_key: local_ephemeral_key,
        init_message: msg,
    })
}

async fn receive_auth<S: AsyncRead + std::marker::Unpin>(
    signer: &SigningKey,
    stream: S,
) -> Result<RemoteState, RLPxError> {
    let secret_key: SecretKey = signer.clone().into();

    let msg_bytes = receive_handshake_msg(stream).await?;
    let size_data = &msg_bytes
        .get(..2)
        .ok_or(RLPxError::InvalidMessageLength())?;
    let msg = &msg_bytes
        .get(2..)
        .ok_or(RLPxError::InvalidMessageLength())?;
    let (auth, remote_ephemeral_key) = decode_auth_message(&secret_key, msg, size_data)?;

    Ok(RemoteState {
        node_id: auth.node_id,
        nonce: auth.nonce,
        ephemeral_key: remote_ephemeral_key,
        init_message: msg_bytes.to_owned(),
    })
}

async fn receive_ack<S: AsyncRead + std::marker::Unpin>(
    signer: &SigningKey,
    remote_node_id: H512,
    stream: S,
) -> Result<RemoteState, RLPxError> {
    let secret_key: SecretKey = signer.clone().into();
    let msg_bytes = receive_handshake_msg(stream).await?;
    let size_data = &msg_bytes
        .get(..2)
        .ok_or(RLPxError::InvalidMessageLength())?;
    let msg = &msg_bytes
        .get(2..)
        .ok_or(RLPxError::InvalidMessageLength())?;
    let ack = decode_ack_message(&secret_key, msg, size_data)?;
    let remote_ephemeral_key = ack
        .get_ephemeral_pubkey()
        .ok_or(RLPxError::NotFound("Remote ephemeral key".to_string()))?;

    Ok(RemoteState {
        node_id: remote_node_id,
        nonce: ack.nonce,
        ephemeral_key: remote_ephemeral_key,
        init_message: msg_bytes.to_owned(),
    })
}

async fn send_handshake_msg<S: AsyncWrite + std::marker::Unpin>(
    mut stream: S,
    msg: &[u8],
) -> Result<(), RLPxError> {
    stream.write_all(msg).await?;
    Ok(())
}

async fn receive_handshake_msg<S: AsyncRead + std::marker::Unpin>(
    mut stream: S,
) -> Result<Vec<u8>, RLPxError> {
    let mut buf = vec![0; 2];

    // Read the message's size
    stream.read_exact(&mut buf).await?;
    let ack_data = [buf[0], buf[1]];
    let msg_size = u16::from_be_bytes(ack_data) as usize;
    if msg_size > P2P_MAX_MESSAGE_SIZE {
        return Err(RLPxError::InvalidMessageLength());
    }
    buf.resize(msg_size + 2, 0);

    // Read the rest of the message
    // Guard unwrap
    if buf.len() < msg_size + 2 {
        return Err(RLPxError::CryptographyError(String::from("bad buf size")));
    }
    stream.read_exact(&mut buf[2..msg_size + 2]).await?;
    let ack_bytes = &buf[..msg_size + 2];
    Ok(ack_bytes.to_vec())
}

/// Encodes an Auth message, to start a handshake.
fn encode_auth_message(
    static_key: &SecretKey,
    local_nonce: H256,
    remote_static_pubkey: &PublicKey,
    local_ephemeral_key: &SecretKey,
) -> Result<Vec<u8>, RLPxError> {
    let node_id = pubkey2id(&static_key.public_key());

    // Derive a shared secret from the static keys.
    let static_shared_secret = ecdh_xchng(static_key, remote_static_pubkey);

    // Create the signature included in the message.
    let signature = sign_shared_secret(
        static_shared_secret.into(),
        local_nonce,
        local_ephemeral_key,
    )?;

    // Compose the auth message.
    let auth = AuthMessage::new(signature, node_id, local_nonce);

    // RLP-encode the message.
    let encoded_auth_msg = auth.encode_to_vec();

    encrypt_message(remote_static_pubkey, encoded_auth_msg)
}

/// Decodes an incomming Auth message, starting a handshake.
fn decode_auth_message(
    static_key: &SecretKey,
    msg: &[u8],
    auth_data: &[u8],
) -> Result<(AuthMessage, PublicKey), RLPxError> {
    let payload = decrypt_message(static_key, msg, auth_data)?;

    // RLP-decode the message.
    let (auth, _padding) = AuthMessage::decode_unfinished(&payload)?;

    // Derive a shared secret from the static keys.
    let peer_pk = id2pubkey(auth.node_id).ok_or(RLPxError::InvalidPeerId())?;
    let static_shared_secret = ecdh_xchng(static_key, &peer_pk);
    let remote_ephemeral_key =
        retrieve_remote_ephemeral_key(static_shared_secret.into(), auth.nonce, auth.signature)?;
    Ok((auth, remote_ephemeral_key))
}

/// Encodes an Ack message, to complete a handshake
fn encode_ack_message(
    local_ephemeral_key: &SecretKey,
    local_nonce: H256,
    remote_static_pubkey: &PublicKey,
) -> Result<Vec<u8>, RLPxError> {
    // Compose the ack message.
    let ack_msg = AckMessage::new(pubkey2id(&local_ephemeral_key.public_key()), local_nonce);

    // RLP-encode the message.
    let encoded_ack_msg = ack_msg.encode_to_vec();

    encrypt_message(remote_static_pubkey, encoded_ack_msg)
}

/// Decodes an Ack message, completing a handshake.
fn decode_ack_message(
    static_key: &SecretKey,
    msg: &[u8],
    auth_data: &[u8],
) -> Result<AckMessage, RLPxError> {
    let payload = decrypt_message(static_key, msg, auth_data)?;

    // RLP-decode the message.
    let (ack, _padding) = AckMessage::decode_unfinished(&payload)?;

    Ok(ack)
}

fn decrypt_message(
    static_key: &SecretKey,
    msg: &[u8],
    size_data: &[u8],
) -> Result<Vec<u8>, RLPxError> {
    // Split the message into its components. General layout is:
    // public-key (65) || iv (16) || ciphertext || mac (32)
    let (pk, rest) = msg.split_at(65);
    let (iv, rest) = rest.split_at(16);
    let (c, d) = rest.split_at(rest.len() - 32);

    // Derive the message shared secret.
    let shared_secret = ecdh_xchng(static_key, &PublicKey::from_sec1_bytes(pk)?);

    // Derive the AES and MAC keys from the message shared secret.
    let mut buf = [0; 32];
    kdf(&shared_secret, &mut buf);
    let aes_key = &buf[..16];
    let mac_key = sha256(&buf[16..]);

    // Verify the MAC.
    let expected_d = sha256_hmac(&mac_key, &[iv, c], size_data);
    if d != expected_d {
        return Err(RLPxError::HandshakeError(String::from("Invalid MAC")));
    }

    // Decrypt the message with the AES key.
    let mut stream_cipher = Aes128Ctr64BE::new_from_slices(aes_key, iv)?;
    let mut decoded = c.to_vec();
    stream_cipher.try_apply_keystream(&mut decoded)?;
    Ok(decoded)
}

fn encrypt_message(
    remote_static_pubkey: &PublicKey,
    mut encoded_msg: Vec<u8>,
) -> Result<Vec<u8>, RLPxError> {
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
    let auth_size: u16 = (encoded_msg.len() + ecies_overhead)
        .try_into()
        .map_err(|_| RLPxError::CryptographyError("Invalid message length".to_owned()))?;
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
    let mut aes_cipher = Aes128Ctr64BE::new_from_slices(aes_key, &iv.0)?;
    aes_cipher.try_apply_keystream(&mut encoded_msg)?;
    let encrypted_auth_msg = encoded_msg;

    // Use the MAC secret to compute the MAC.
    let r_public_key = message_secret_key.public_key().to_encoded_point(false);
    let mac_footer = sha256_hmac(&mac_key, &[&iv.0, &encrypted_auth_msg], &auth_size_bytes);

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

fn retrieve_remote_ephemeral_key(
    shared_secret: H256,
    remote_nonce: H256,
    signature: Signature,
) -> Result<PublicKey, RLPxError> {
    let signature_prehash = shared_secret ^ remote_nonce;
    let sign = ecdsa::Signature::from_slice(&signature.to_fixed_bytes()[..64])?;
    let rid = RecoveryId::from_byte(signature[64]).ok_or(RLPxError::InvalidRecoveryId())?;
    let ephemeral_key =
        VerifyingKey::recover_from_prehash(signature_prehash.as_bytes(), &sign, rid)?;
    Ok(ephemeral_key.into())
}

fn sign_shared_secret(
    shared_secret: H256,
    local_nonce: H256,
    local_ephemeral_key: &SecretKey,
) -> Result<Signature, RLPxError> {
    let signature_prehash = shared_secret ^ local_nonce;
    let (signature, rid) =
        SigningKey::from(local_ephemeral_key).sign_prehash_recoverable(&signature_prehash.0)?;
    let mut signature_bytes = [0; 65];
    signature_bytes[..64].copy_from_slice(signature.to_bytes().as_slice());
    signature_bytes[64] = rid.to_byte();
    Ok(signature_bytes.into())
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
        let decoder = Decoder::new(rlp)?;
        let (signature, decoder) = decoder.decode_field("signature")?;
        let (node_id, decoder) = decoder.decode_field("node_id")?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (version, decoder) = decoder.decode_field("version")?;

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
        let decoder = Decoder::new(rlp)?;
        let (ephemeral_pubkey, decoder) = decoder.decode_field("ephemeral_pubkey")?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (version, decoder) = decoder.decode_field("version")?;

        let rest = decoder.finish_unchecked();
        let this = Self {
            ephemeral_pubkey,
            nonce,
            version,
        };
        Ok((this, rest))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use ethrex_core::H256;
    use hex_literal::hex;
    use k256::SecretKey;

    use crate::rlpx::{handshake::decode_ack_message, utils::pubkey2id};

    #[test]
    fn test_ack_decoding() {
        // This is the Ack₂ message from EIP-8.
        // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-8.md
        let msg = hex!("01ea0451958701280a56482929d3b0757da8f7fbe5286784beead59d95089c217c9b917788989470b0e330cc6e4fb383c0340ed85fab836ec9fb8a49672712aeabbdfd1e837c1ff4cace34311cd7f4de05d59279e3524ab26ef753a0095637ac88f2b499b9914b5f64e143eae548a1066e14cd2f4bd7f814c4652f11b254f8a2d0191e2f5546fae6055694aed14d906df79ad3b407d94692694e259191cde171ad542fc588fa2b7333313d82a9f887332f1dfc36cea03f831cb9a23fea05b33deb999e85489e645f6aab1872475d488d7bd6c7c120caf28dbfc5d6833888155ed69d34dbdc39c1f299be1057810f34fbe754d021bfca14dc989753d61c413d261934e1a9c67ee060a25eefb54e81a4d14baff922180c395d3f998d70f46f6b58306f969627ae364497e73fc27f6d17ae45a413d322cb8814276be6ddd13b885b201b943213656cde498fa0e9ddc8e0b8f8a53824fbd82254f3e2c17e8eaea009c38b4aa0a3f306e8797db43c25d68e86f262e564086f59a2fc60511c42abfb3057c247a8a8fe4fb3ccbadde17514b7ac8000cdb6a912778426260c47f38919a91f25f4b5ffb455d6aaaf150f7e5529c100ce62d6d92826a71778d809bdf60232ae21ce8a437eca8223f45ac37f6487452ce626f549b3b5fdee26afd2072e4bc75833c2464c805246155289f4");
        let static_key_a = SecretKey::from_slice(&hex!(
            "49a7b37aa6f6645917e7b807e9d1c00d4fa71f18343b0d4122a4d2df64dd6fee"
        ))
        .unwrap();

        let expected_nonce_b =
            H256::from_str("559aead08264d5795d3909718cdd05abd49572e84fe55590eef31a88a08fdffd")
                .unwrap();
        let expected_ephemeral_key_b = pubkey2id(
            &SecretKey::from_slice(&hex!(
                "e238eb8e04fee6511ab04c6dd3c89ce097b11f25d584863ac2b6d5b35b1847e4"
            ))
            .unwrap()
            .public_key(),
        );

        let ack = decode_ack_message(&static_key_a, &msg[2..], &msg[..2]).unwrap();

        assert_eq!(ack.ephemeral_pubkey, expected_ephemeral_key_b);
        assert_eq!(ack.nonce, expected_nonce_b);
        assert_eq!(ack.version, 4u8);
    }
}
