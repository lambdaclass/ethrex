use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use super::{
    codec::RLPxCodec,
    server::{Initiator, Receiver},
};
use crate::{
    rlpx::{
        connection::server::{Established, InnerState},
        error::RLPxError,
        l2::l2_connection::L2ConnState,
        utils::{
            compress_pubkey, decompress_pubkey, ecdh_xchng, kdf, log_peer_debug, sha256,
            sha256_hmac,
        },
    },
    types::Node,
};
use aes::cipher::{KeyIvInit, StreamCipher};
use ethrex_common::{H128, H256, H512, Signature};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use futures::{StreamExt, stream::SplitStream};
use rand::Rng;
use secp256k1::{
    PublicKey, SecretKey,
    ecdsa::{RecoverableSignature, RecoveryId},
};
use sha3::{Digest, Keccak256};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpSocket, TcpStream},
    sync::Mutex,
};
use tokio_util::codec::Framed;

type Aes128Ctr64BE = ctr::Ctr64BE<aes::Aes128>;

// https://github.com/ethereum/go-ethereum/blob/master/p2p/peer.go#L44
pub const P2P_MAX_MESSAGE_SIZE: usize = 2048;

pub(crate) struct RemoteState {
    pub(crate) public_key: H512,
    pub(crate) nonce: H256,
    pub(crate) ephemeral_key: PublicKey,
    pub(crate) init_message: Vec<u8>,
}

pub(crate) struct LocalState {
    pub(crate) nonce: H256,
    pub(crate) ephemeral_key: SecretKey,
    pub(crate) init_message: Vec<u8>,
}

pub(crate) async fn perform(
    state: InnerState,
) -> Result<(Established, SplitStream<Framed<TcpStream, RLPxCodec>>), RLPxError> {
    let (context, node, framed, inbound) = match state {
        InnerState::Initiator(Initiator { context, node, .. }) => {
            let addr = SocketAddr::new(node.ip, node.tcp_port);
            let mut stream = match tcp_stream(addr).await {
                Ok(result) => result,
                Err(error) => {
                    log_peer_debug(&node, &format!("Error creating tcp connection {error}"));
                    // context.table.lock().await.replace_peer(node.node_id());
                    return Err(error)?;
                }
            };
            let local_state = send_auth(&context.signer, node.public_key, &mut stream).await?;
            let remote_state = receive_ack(&context.signer, node.public_key, &mut stream).await?;
            // Local node is initator
            // keccak256(nonce || initiator-nonce)
            let hashed_nonces: [u8; 32] =
                Keccak256::digest([remote_state.nonce.0, local_state.nonce.0].concat()).into();
            let codec = RLPxCodec::new(&local_state, &remote_state, hashed_nonces)?;
            log_peer_debug(&node, "Completed handshake as initiator");
            (context, node, Framed::new(stream, codec), false)
        }
        InnerState::Receiver(Receiver {
            context,
            peer_addr,
            stream,
        }) => {
            let Some(mut stream) = Arc::into_inner(stream) else {
                return Err(RLPxError::StateError("Cannot use the stream".to_string()));
            };
            let remote_state = receive_auth(&context.signer, &mut stream).await?;
            let local_state = send_ack(remote_state.public_key, &mut stream).await?;
            // Remote node is initiator
            // keccak256(nonce || initiator-nonce)
            let hashed_nonces: [u8; 32] =
                Keccak256::digest([local_state.nonce.0, remote_state.nonce.0].concat()).into();
            let codec = RLPxCodec::new(&local_state, &remote_state, hashed_nonces)?;
            let node = Node::new(
                peer_addr.ip(),
                peer_addr.port(),
                peer_addr.port(),
                remote_state.public_key,
            );
            log_peer_debug(&node, "Completed handshake as receiver");
            (context, node, Framed::new(stream, codec), true)
        }
        InnerState::Established(_) => {
            return Err(RLPxError::StateError("Already established".to_string()));
        }
        // Shouldn't perform a Handshake on an already failed connection.
        // Put it here to complete the match arms
        InnerState::HandshakeFailed => {
            return Err(RLPxError::StateError("Handshake Failed".to_string()));
        }
    };
    let (sink, stream) = framed.split();
    Ok((
        Established {
            signer: context.signer,
            sink: Arc::new(Mutex::new(sink)),
            node: node.clone(),
            storage: context.storage.clone(),
            blockchain: context.blockchain.clone(),
            capabilities: vec![],
            negotiated_eth_capability: None,
            negotiated_snap_capability: None,
            last_block_range_update_block: 0,
            requested_pooled_txs: HashMap::new(),
            client_version: context.client_version.clone(),
            connection_broadcast_send: context.broadcast.clone(),
            table: context.table.clone(),
            backend_channel: None,
            _inbound: inbound,
            l2_state: context
                .based_context
                .map_or_else(|| L2ConnState::Unsupported, L2ConnState::Disconnected),
        },
        stream,
    ))
}

async fn tcp_stream(addr: SocketAddr) -> Result<TcpStream, std::io::Error> {
    match addr {
        SocketAddr::V4(_) => TcpSocket::new_v4()?.connect(addr).await,
        SocketAddr::V6(_) => TcpSocket::new_v6()?.connect(addr).await,
    }
}

async fn send_auth<S: AsyncWrite + std::marker::Unpin>(
    signer: &SecretKey,
    remote_public_key: H512,
    mut stream: S,
) -> Result<LocalState, RLPxError> {
    let peer_pk = compress_pubkey(remote_public_key).ok_or(RLPxError::InvalidPeerId())?;

    let local_nonce = H256::random_using(&mut rand::thread_rng());
    let local_ephemeral_key = SecretKey::new(&mut rand::thread_rng());

    let msg = encode_auth_message(signer, local_nonce, &peer_pk, &local_ephemeral_key)?;
    stream.write_all(&msg).await?;

    Ok(LocalState {
        nonce: local_nonce,
        ephemeral_key: local_ephemeral_key,
        init_message: msg,
    })
}

async fn send_ack<S: AsyncWrite + std::marker::Unpin>(
    remote_public_key: H512,
    mut stream: S,
) -> Result<LocalState, RLPxError> {
    let peer_pk = compress_pubkey(remote_public_key).ok_or(RLPxError::InvalidPeerId())?;

    let local_nonce = H256::random_using(&mut rand::thread_rng());
    let local_ephemeral_key = SecretKey::new(&mut rand::thread_rng());

    let msg = encode_ack_message(&local_ephemeral_key, local_nonce, &peer_pk)?;
    stream.write_all(&msg).await?;

    Ok(LocalState {
        nonce: local_nonce,
        ephemeral_key: local_ephemeral_key,
        init_message: msg,
    })
}

async fn receive_auth<S: AsyncRead + std::marker::Unpin>(
    signer: &SecretKey,
    stream: S,
) -> Result<RemoteState, RLPxError> {
    let msg_bytes = receive_handshake_msg(stream).await?;
    let size_data = &msg_bytes
        .get(..2)
        .ok_or(RLPxError::InvalidMessageLength())?;
    let msg = &msg_bytes
        .get(2..)
        .ok_or(RLPxError::InvalidMessageLength())?;
    let (auth, remote_ephemeral_key) = decode_auth_message(signer, msg, size_data)?;

    Ok(RemoteState {
        public_key: auth.public_key,
        nonce: auth.nonce,
        ephemeral_key: remote_ephemeral_key,
        init_message: msg_bytes.to_owned(),
    })
}

async fn receive_ack<S: AsyncRead + std::marker::Unpin>(
    signer: &SecretKey,
    remote_public_key: H512,
    stream: S,
) -> Result<RemoteState, RLPxError> {
    let msg_bytes = receive_handshake_msg(stream).await?;
    let size_data = &msg_bytes
        .get(..2)
        .ok_or(RLPxError::InvalidMessageLength())?;
    let msg = &msg_bytes
        .get(2..)
        .ok_or(RLPxError::InvalidMessageLength())?;
    let ack = decode_ack_message(signer, msg, size_data)?;
    let remote_ephemeral_key = ack
        .get_ephemeral_pubkey()
        .ok_or(RLPxError::NotFound("Remote ephemeral key".to_string()))?;

    Ok(RemoteState {
        public_key: remote_public_key,
        nonce: ack.nonce,
        ephemeral_key: remote_ephemeral_key,
        init_message: msg_bytes.to_owned(),
    })
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
    let public_key = decompress_pubkey(&static_key.public_key(secp256k1::SECP256K1));

    // Derive a shared secret from the static keys.
    let static_shared_secret = ecdh_xchng(static_key, remote_static_pubkey).map_err(|error| {
        RLPxError::CryptographyError(format!("Invalid generated static shared secret: {error}"))
    })?;

    // Create the signature included in the message.
    let signature = sign_shared_secret(
        static_shared_secret.into(),
        local_nonce,
        local_ephemeral_key,
    )?;

    // Compose the auth message.
    let auth = AuthMessage::new(signature, public_key, local_nonce);

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
    let peer_pk = compress_pubkey(auth.public_key).ok_or(RLPxError::InvalidPeerId())?;
    let static_shared_secret = ecdh_xchng(static_key, &peer_pk).map_err(|error| {
        RLPxError::CryptographyError(format!("Invalid generated static shared secret: {error}"))
    })?;
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
    let ack_msg = AckMessage::new(
        decompress_pubkey(&local_ephemeral_key.public_key(secp256k1::SECP256K1)),
        local_nonce,
    );

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
    let shared_secret = ecdh_xchng(static_key, &PublicKey::from_slice(pk)?).map_err(|error| {
        RLPxError::CryptographyError(format!("Invalid generated shared secret: {error}"))
    })?;
    // Derive the AES and MAC keys from the message shared secret.
    let mut buf = [0; 32];
    kdf(&shared_secret, &mut buf).map_err(|error| {
        RLPxError::CryptographyError(format!("Couldn't get keys from shared secret: {error}"))
    })?;
    let aes_key = &buf[..16];
    let mac_key = sha256(&buf[16..]);

    // Verify the MAC.
    let expected_d = sha256_hmac(&mac_key, &[iv, c], size_data)
        .map_err(|error| RLPxError::CryptographyError(error.to_string()))?;
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
    const SIGNATURE_SIZE: u16 = 65;
    const IV_SIZE: u16 = 16;
    const MAC_FOOTER_SIZE: u16 = 32;

    let mut rng = rand::thread_rng();

    // Pad with random amount of data. the amount needs to be at least 100 bytes to make
    // the message distinguishable from pre-EIP-8 handshakes.
    let padding_length = rng.gen_range(100..=300);
    encoded_msg.resize(encoded_msg.len() + padding_length, 0);

    // Precompute the size of the message. This is needed for computing the MAC.
    let ecies_overhead = SIGNATURE_SIZE + IV_SIZE + MAC_FOOTER_SIZE;
    let encoded_msg_len: u16 = encoded_msg
        .len()
        .try_into()
        .map_err(|_| RLPxError::CryptographyError("Invalid message length".to_owned()))?;
    let auth_size = ecies_overhead + encoded_msg_len;
    let auth_size_bytes = auth_size.to_be_bytes();

    // Generate a keypair just for this message.
    let message_secret_key = SecretKey::new(&mut rng);

    // Derive a shared secret for this message.
    let message_secret =
        ecdh_xchng(&message_secret_key, remote_static_pubkey).map_err(|error| {
            RLPxError::CryptographyError(format!("Invalid generated message secret:  {error}"))
        })?;

    // Derive the AES and MAC keys from the message secret.
    let mut secret_keys = [0; 32];
    kdf(&message_secret, &mut secret_keys)
        .map_err(|error| RLPxError::CryptographyError(error.to_string()))?;
    let aes_key = &secret_keys[..16];
    let mac_key = sha256(&secret_keys[16..]);

    // Use the AES secret to encrypt the auth message.
    let iv = H128::random_using(&mut rng);
    let mut aes_cipher = Aes128Ctr64BE::new_from_slices(aes_key, &iv.0)?;
    aes_cipher.try_apply_keystream(&mut encoded_msg)?;
    let encrypted_auth_msg = encoded_msg;

    // Use the MAC secret to compute the MAC.
    let r_public_key = message_secret_key
        .public_key(secp256k1::SECP256K1)
        .serialize_uncompressed();
    let mac_footer = sha256_hmac(&mac_key, &[&iv.0, &encrypted_auth_msg], &auth_size_bytes)
        .map_err(|error| RLPxError::CryptographyError(error.to_string()))?;

    // Return the message
    let mut final_msg = Vec::new();
    final_msg.extend_from_slice(&auth_size_bytes);
    final_msg.extend_from_slice(&r_public_key);
    final_msg.extend_from_slice(&iv.0);
    final_msg.extend_from_slice(&encrypted_auth_msg);
    final_msg.extend_from_slice(&mac_footer);
    Ok(final_msg)
}

fn retrieve_remote_ephemeral_key(
    shared_secret: H256,
    remote_nonce: H256,
    signature: Signature,
) -> Result<PublicKey, RLPxError> {
    let signature_prehash = shared_secret ^ remote_nonce;
    let msg = secp256k1::Message::from_digest_slice(signature_prehash.as_bytes())?;
    let rid = RecoveryId::from_i32(signature[64].into())?;
    let sig = RecoverableSignature::from_compact(&signature[0..64], rid)?;
    Ok(secp256k1::SECP256K1.recover_ecdsa(&msg, &sig)?)
}

fn sign_shared_secret(
    shared_secret: H256,
    local_nonce: H256,
    local_ephemeral_key: &SecretKey,
) -> Result<Signature, RLPxError> {
    let signature_prehash = shared_secret ^ local_nonce;
    let msg = secp256k1::Message::from_digest_slice(signature_prehash.as_bytes())?;
    let sig = secp256k1::SECP256K1.sign_ecdsa_recoverable(&msg, local_ephemeral_key);
    let (rid, signature) = sig.serialize_compact();
    let mut signature_bytes = [0; 65];
    signature_bytes[..64].copy_from_slice(&signature);
    signature_bytes[64] = rid
        .to_i32()
        .try_into()
        .map_err(|_| RLPxError::CryptographyError("Invalid recovery id".into()))?;
    Ok(signature_bytes.into())
}

#[derive(Debug)]
pub(crate) struct AuthMessage {
    /// The signature of the message.
    /// The signed data is `static-shared-secret ^ initiator-nonce`.
    pub signature: Signature,
    /// The uncompressed node public key of the initiator.
    pub public_key: H512,
    /// The nonce generated by the initiator.
    pub nonce: H256,
    /// The version of RLPx used by the sender.
    /// The current version is 5.
    pub version: u8,
}

impl AuthMessage {
    pub fn new(signature: Signature, public_key: H512, nonce: H256) -> Self {
        Self {
            signature,
            public_key,
            nonce,
            version: 5,
        }
    }
}

impl RLPEncode for AuthMessage {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.signature)
            .encode_field(&self.public_key)
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
        let (public_key, decoder) = decoder.decode_field("public_key")?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (version, decoder) = decoder.decode_field("version")?;

        let rest = decoder.finish_unchecked();
        let this = Self {
            signature,
            public_key,
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
        compress_pubkey(self.ephemeral_pubkey)
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

    use ethrex_common::H256;
    use hex_literal::hex;
    use secp256k1::SecretKey;

    use crate::rlpx::{connection::handshake::decode_ack_message, utils::decompress_pubkey};

    #[test]
    fn test_ack_decoding() {
        // This is the Ack₂ message from EIP-8.
        // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-8.md
        let msg = hex!(
            "01ea0451958701280a56482929d3b0757da8f7fbe5286784beead59d95089c217c9b917788989470b0e330cc6e4fb383c0340ed85fab836ec9fb8a49672712aeabbdfd1e837c1ff4cace34311cd7f4de05d59279e3524ab26ef753a0095637ac88f2b499b9914b5f64e143eae548a1066e14cd2f4bd7f814c4652f11b254f8a2d0191e2f5546fae6055694aed14d906df79ad3b407d94692694e259191cde171ad542fc588fa2b7333313d82a9f887332f1dfc36cea03f831cb9a23fea05b33deb999e85489e645f6aab1872475d488d7bd6c7c120caf28dbfc5d6833888155ed69d34dbdc39c1f299be1057810f34fbe754d021bfca14dc989753d61c413d261934e1a9c67ee060a25eefb54e81a4d14baff922180c395d3f998d70f46f6b58306f969627ae364497e73fc27f6d17ae45a413d322cb8814276be6ddd13b885b201b943213656cde498fa0e9ddc8e0b8f8a53824fbd82254f3e2c17e8eaea009c38b4aa0a3f306e8797db43c25d68e86f262e564086f59a2fc60511c42abfb3057c247a8a8fe4fb3ccbadde17514b7ac8000cdb6a912778426260c47f38919a91f25f4b5ffb455d6aaaf150f7e5529c100ce62d6d92826a71778d809bdf60232ae21ce8a437eca8223f45ac37f6487452ce626f549b3b5fdee26afd2072e4bc75833c2464c805246155289f4"
        );
        let static_key_a = SecretKey::from_slice(&hex!(
            "49a7b37aa6f6645917e7b807e9d1c00d4fa71f18343b0d4122a4d2df64dd6fee"
        ))
        .unwrap();

        let expected_nonce_b =
            H256::from_str("559aead08264d5795d3909718cdd05abd49572e84fe55590eef31a88a08fdffd")
                .unwrap();
        let expected_ephemeral_key_b = decompress_pubkey(
            &SecretKey::from_slice(&hex!(
                "e238eb8e04fee6511ab04c6dd3c89ce097b11f25d584863ac2b6d5b35b1847e4"
            ))
            .unwrap()
            .public_key(secp256k1::SECP256K1),
        );

        let ack = decode_ack_message(&static_key_a, &msg[2..], &msg[..2]).unwrap();

        assert_eq!(ack.ephemeral_pubkey, expected_ephemeral_key_b);
        assert_eq!(ack.nonce, expected_nonce_b);
        assert_eq!(ack.version, 4u8);
    }
}
