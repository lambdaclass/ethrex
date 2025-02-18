use std::{
    io::{Read, Write},
    net::TcpStream,
};

use ethrex_core::{H256, H512, H520};
use ethrex_rlp::encode::RLPEncode;
use k256::{PublicKey, SecretKey};

use super::{
    constants::RLPX_PROTOCOL_VERSION,
    crypto::{
        decrypt_message, ecdh_xchng, encrypt_message, retrieve_remote_ephemeral_key,
        sign_shared_secret,
    },
    packet::{Auth, AuthAck, Packet},
    utils::{id2pubkey, pubkey2id},
    Error,
};

type Result<T> = std::result::Result<T, Error>;

fn auth_signature(
    local_secret_key: &SecretKey,
    remote_public_key: &PublicKey,
    nonce: H256,
) -> Result<H520> {
    let static_shared_secret = ecdh_xchng(local_secret_key, remote_public_key);

    let local_ephemeral_key = SecretKey::random(&mut rand::thread_rng());

    sign_shared_secret(static_shared_secret.into(), nonce, &local_ephemeral_key)
        .map_err(|_| Error::CryptographyError)
}

pub fn send_auth(
    stream: &mut TcpStream,
    local_secret_key: &SecretKey,
    remote_id: H512,
) -> Result<()> {
    let mut auth_msg = Auth {
        signature: H520::zero(),
        initiator_pubkey: pubkey2id(&local_secret_key.public_key()),
        nonce: H256::random(),
        version: RLPX_PROTOCOL_VERSION,
    };

    let remote_pubkey =
        id2pubkey(remote_id).ok_or(Error::InvalidMessage("Invalid peer ID".to_string()))?;

    auth_msg.signature = auth_signature(local_secret_key, &remote_pubkey, auth_msg.nonce)?;

    let auth_body = auth_msg.encode_to_vec();

    let enc_auth_body =
        encrypt_message(&remote_pubkey, auth_body).map_err(|_| Error::CryptographyError)?;

    stream
        .write_all(&enc_auth_body)
        .map_err(|err| Error::ConnectionError(err.to_string()))?;

    Ok(())
}

pub fn receive_auth(stream: &mut TcpStream, local_secret_key: &SecretKey) -> Result<Auth> {
    let mut size_buf = [0u8; 2];
    if let Err(e) = stream.read_exact(&mut size_buf) {
        tracing::error!(error = ?e, "Failed to read size of message");
        return Err(Error::ReadError);
    }

    let auth_size = u16::from_be_bytes(size_buf);
    let mut enc_auth_body = vec![0u8; auth_size as usize];
    if stream.read_exact(&mut enc_auth_body).is_err() {
        tracing::error!("Failed to read message");
        return Err(Error::ReadError);
    }

    let auth_body = decrypt_message(local_secret_key, &enc_auth_body, &size_buf)
        .map_err(|_| Error::CryptographyError)?;

    let (auth_msg, _) = Auth::try_rlp_decode(auth_body.as_slice())
        .map_err(|_| Error::InvalidMessage("Failed to decode auth message".to_string()))?;

    return Ok(auth_msg);
}

pub fn send_ack(
    stream: &mut TcpStream,
    local_secret_key: &SecretKey,
    auth_msg: Auth,
) -> Result<()> {
    let static_shared_secret = ecdh_xchng(
        local_secret_key,
        &id2pubkey(auth_msg.initiator_pubkey)
            .ok_or(Error::InvalidMessage("Invalid public key".to_string()))?,
    );
    // TODO: We'll need to save this
    let remote_ephemeral_pubkey = retrieve_remote_ephemeral_key(
        static_shared_secret.into(),
        auth_msg.nonce,
        auth_msg.signature,
    )
    .map_err(|_| Error::CryptographyError)?;

    let local_ephemeral_key = SecretKey::random(&mut rand::thread_rng());

    let auth_ack_msg = AuthAck {
        recipient_ephemeral_pubk: pubkey2id(&local_ephemeral_key.public_key()),
        recipient_nonce: H256::random(),
        version: RLPX_PROTOCOL_VERSION,
    };

    let auth_ack_body = auth_ack_msg.encode_to_vec();
    let enc_auth_ack_body = encrypt_message(
        &id2pubkey(auth_msg.initiator_pubkey).ok_or(Error::InvalidMessage(
            "Invalid initiator pubkey".to_string(),
        ))?,
        auth_ack_body,
    )
    .map_err(|_| Error::CryptographyError)?;

    stream
        .write_all(&enc_auth_ack_body)
        .map_err(|err| Error::ConnectionError(err.to_string()))?;

    Ok(())
}

pub fn receive_ack(stream: &mut TcpStream, local_secret_key: &SecretKey) -> Result<()> {
    let mut size_buf = [0u8; 2];
    if let Err(e) = stream.read_exact(&mut size_buf) {
        tracing::error!(error = ?e, "Failed to read size of message");
        return Err(Error::ReadError);
    }
    let ack_size = u16::from_be_bytes(size_buf);

    let mut enc_ack_body = vec![0u8; ack_size as usize];
    if stream.read(&mut enc_ack_body).is_err() {
        tracing::error!("Failed to read message");
        return Err(Error::ReadError);
    }

    let ack_body = decrypt_message(local_secret_key, &enc_ack_body, &size_buf)
        .map_err(|_| Error::CryptographyError)?;

    // TODO: We'll need to save this
    let (ack_msg, _) = AuthAck::try_rlp_decode(ack_body.as_slice())
        .map_err(|_| Error::InvalidMessage("Failed to decode ACK message".to_string()))
        .unwrap();

    Ok(())
}
