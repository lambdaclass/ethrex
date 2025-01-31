use crate::peer::{
    handshake::{receive_ack, receive_auth, send_ack},
    ingress::{Mailbox, Message},
};
use ethrex_core::H512;
use k256::SecretKey;
use std::net::{IpAddr, TcpStream};
use tokio::sync::mpsc;

use super::handshake::send_auth;

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
        send_auth(&mut stream, &cfg.secret_key, peer_id)?;

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

    pub async fn run(mut self) -> Result<Error> {
        match self.status {
            ConnectionStatus::WaitingAuth => {
                let auth_msg = receive_auth(&mut self.stream, &self.config.secret_key)?;
                if let Err(e) = send_ack(&mut self.stream, &self.config.secret_key, auth_msg) {
                    tracing::error!("{e}");
                };
                self.status = ConnectionStatus::WaitingHello;
            }
            ConnectionStatus::WaitingAuthAck => {
                receive_ack(&mut self.stream, &self.config.secret_key)?;
                // Send Hello message
                self.status = ConnectionStatus::WaitingHello;
            }
            _ => todo!(),
        }

        tracing::info!("Peer started");

        // Here should be the main loop for RLPx capabilities msg exchange

        Err(Error::UnexpectedExit)
    }
}
