use crate::peer::{
    crypto::{ecdh_xchng, retrieve_remote_ephemeral_key},
    ingress::{Mailbox, Message},
    utils::id2pubkey,
};
use ethrex_core::{H256, H512, H520};
use ethrex_rlp::encode::RLPEncode;
use k256::{PublicKey, SecretKey};
use std::{
    io::{Read, Write},
    net::{IpAddr, TcpStream},
};
use tokio::sync::mpsc;

use super::{
    constants::RLPX_PROTOCOL_VERSION,
    crypto::{decrypt_message, encrypt_message, sign_shared_secret},
    ingress::{Auth, AuthAck, PacketData},
    utils::pubkey2id,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Couldn't receive data from peer")]
    ReadError,
    #[error("Cryptography error")]
    CryptographyError,
    #[error("Invalid message: {0}")]
    InvalidMessage(String),
    #[error("Connection error with peer: {0}")]
    ConnectionError(String),
    #[error("Unexpected exit")]
    UnexpectedExit,
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct Config {
    secret_key: SecretKey,
}

impl Config {
    pub fn new(secret_key: SecretKey) -> Self {
        Self { secret_key }
    }
}

/// A -> B (Establish TCP connection)
/// A -> Auth -> B
/// B -> ACK -> A
/// The following to messages are asynchronous
/// B -> Hello -> A
/// A -> Hello -> B
#[derive(Debug)]
pub enum ConnectionStatus {
    /// A connection was opened by a peer and we are waiting for an Auth message
    WaitingAuth,
    /// We are the initiator and are waiting for an ACK message from the receiver
    WaitingAuthAck,
    /// We are waiting for the Hello message from the other peer,
    WaitingHello,
    /// Handshake is complete
    Ready,
}

pub struct Actor {
    stream: TcpStream,
    config: Config,
    mailbox: Mailbox,
    mailbox_handler: mpsc::Receiver<Message>,
    status: ConnectionStatus,
}

impl Actor {
    pub fn new_as_initiator(
        cfg: Config,
        peer_id: H512,
        peer_ip: IpAddr,
        peer_port: u16,
    ) -> Result<(Self, Mailbox)> {
        let (sender, receiver) = mpsc::channel(32);
        let mailbox = Mailbox::new(sender);

        let mut stream = TcpStream::connect((peer_ip, peer_port))
            .map_err(|err| Error::ConnectionError(err.to_string()))?;
        Self::send_auth(&mut stream, cfg.clone(), peer_id)?;

        let actor = Self {
            stream,
            config: cfg,
            mailbox: mailbox.clone(),
            mailbox_handler: receiver,
            status: ConnectionStatus::WaitingAuthAck,
        };

        Ok((actor, mailbox))
    }

    pub fn new_as_recipient(stream: TcpStream, cfg: Config) -> Result<(Self, Mailbox)> {
        let (sender, receiver) = mpsc::channel(32);
        let mailbox = Mailbox::new(sender);

        let actor = Self {
            stream,
            config: cfg,
            mailbox: mailbox.clone(),
            mailbox_handler: receiver,
            status: ConnectionStatus::WaitingAuth,
        };

        Ok((actor, mailbox))
    }

    fn auth_signature(
        secret_key: &SecretKey,
        peer_pubkey: &PublicKey,
        nonce: H256,
    ) -> Result<H520> {
        let static_shared_secret = ecdh_xchng(secret_key, peer_pubkey);

        let local_ephemeral_key = SecretKey::random(&mut rand::thread_rng());

        sign_shared_secret(static_shared_secret.into(), nonce, &local_ephemeral_key)
            .map_err(|_| Error::CryptographyError)
    }

    fn send_auth(stream: &mut TcpStream, config: Config, peer_id: H512) -> Result<()> {
        let mut auth_msg = Auth {
            signature: H520::zero(),
            initiator_pubkey: pubkey2id(&config.secret_key.public_key()),
            nonce: H256::random(),
            version: RLPX_PROTOCOL_VERSION,
        };

        let remote_pubkey =
            id2pubkey(peer_id).ok_or(Error::InvalidMessage("Invalid peer ID".to_string()))?;

        auth_msg.signature =
            Self::auth_signature(&config.secret_key, &remote_pubkey, auth_msg.nonce)?;

        let auth_body = PacketData::Auth(auth_msg.clone()).encode_to_vec();

        let enc_auth_body =
            encrypt_message(&remote_pubkey, auth_body).map_err(|_| Error::CryptographyError)?;

        stream
            .write_all(&enc_auth_body)
            .map_err(|err| Error::ConnectionError(err.to_string()))?;

        Ok(())
    }

    fn wait_auth(&mut self) -> Result<Auth> {
        let mut size_buf = [0u8; 2];
        if let Err(e) = self.stream.read_exact(&mut size_buf) {
            tracing::error!(error = ?e, "Failed to read size of message");
            return Err(Error::ReadError);
        }

        let auth_size = u16::from_be_bytes(size_buf);
        let mut enc_auth_body = vec![0u8; auth_size as usize];
        if self.stream.read_exact(&mut enc_auth_body).is_err() {
            tracing::error!("Failed to read message");
            return Err(Error::ReadError);
        }

        let auth_body = decrypt_message(&self.config.secret_key, &enc_auth_body, &size_buf)
            .map_err(|_| Error::CryptographyError)?;

        let auth_msg = match PacketData::decode(auth_body.as_slice()) {
            Ok(PacketData::Auth(auth_msg)) => auth_msg,
            _ => return Err(Error::InvalidMessage(hex::encode(auth_body))),
        };

        return Ok(auth_msg);
    }

    pub fn send_auth_ack(&mut self, auth_msg: Auth) -> Result<()> {
        let static_shared_secret = ecdh_xchng(
            &self.config.secret_key,
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

        let auth_ack_msg = PacketData::AuthAck(AuthAck {
            recipient_ephemeral_pubk: pubkey2id(&local_ephemeral_key.public_key()),
            recipient_nonce: H256::random(),
            ack_vsn: RLPX_PROTOCOL_VERSION,
        });

        let auth_ack_body = auth_ack_msg.encode_to_vec();
        let enc_auth_ack_body = encrypt_message(
            &id2pubkey(auth_msg.initiator_pubkey).ok_or(Error::InvalidMessage(
                "Invalid initiator pubkey".to_string(),
            ))?,
            auth_ack_body,
        )
        .map_err(|_| Error::CryptographyError)?;

        self.stream
            .write_all(&enc_auth_ack_body)
            .map_err(|err| Error::ConnectionError(err.to_string()))?;

        self.status = ConnectionStatus::WaitingHello;

        Ok(())
    }

    pub async fn run(mut self) -> Result<Error> {
        match self.status {
            ConnectionStatus::WaitingAuth => {
                let auth_msg = self.wait_auth()?;
                if let Err(e) = self.send_auth_ack(auth_msg) {
                    tracing::error!("{e}");
                };
            }
            _ => todo!(),
        }

        tracing::info!("Peer started");

        // Here should be the main loop for RLPx capabilities msg exchange

        Err(Error::UnexpectedExit)
    }
}
