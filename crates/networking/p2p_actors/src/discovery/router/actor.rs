use crate::{
    discovery::{
        self,
        packet::{Packet, DEFAULT_UDP_PAYLOAD_BUF},
        router::ingress::{Mailbox, Message},
    },
    types::Endpoint,
};
use commonware_runtime::Spawner;
use std::{net::SocketAddr, sync::Arc};
use tokio::{net::UdpSocket, sync::mpsc};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to bind socket {0}, reason: {1}")]
    FailedToBindSocket(SocketAddr, String),
    #[error("Failed to relay message: {0}")]
    FailedToRelayMessage(String),
    #[error("Receiver task failed")]
    ReceiverTaskFailed,
    #[error("Listener task failed")]
    ListenerTaskFailed,
    #[error("Task finished unexpectedly")]
    TaskFinishedUnexpectedly,
    #[error("Discovery server not attached")]
    DiscoveryServerNotAttached,
    #[error("Commonware runtime error: {0}")]
    CommonwareRuntimeError(#[from] commonware_runtime::Error),
}

pub struct Config {
    pub endpoint: Endpoint,
    pub timeout_duration: std::time::Duration,
}

pub struct Actor {
    runtime: commonware_runtime::tokio::Context,

    mailbox: Mailbox,
    receiver: mpsc::Receiver<Message>,

    server_mailbox: Option<discovery::server::Mailbox>,

    endpoint: Endpoint,

    timeout_duration: std::time::Duration,
}

impl Actor {
    pub fn new(
        runtime: commonware_runtime::tokio::Context,
        cfg: Config,
    ) -> Result<(Self, Mailbox), Error> {
        let (sender, receiver) = mpsc::channel(32);
        let mailbox = Mailbox::new(sender);
        let actor = Self {
            runtime,
            mailbox: mailbox.clone(),
            receiver,
            server_mailbox: None,
            endpoint: cfg.endpoint,
            timeout_duration: cfg.timeout_duration,
        };
        Ok((actor, mailbox))
    }

    pub fn register_discovery_server(&mut self, mailbox: discovery::server::Mailbox) {
        self.server_mailbox = Some(mailbox);
    }

    pub async fn run(mut self) -> Result<(), Error> {
        let udp_socket_address = self.endpoint.clone().udp_socket_addr();

        let conn = match UdpSocket::bind(udp_socket_address).await {
            Ok(conn) => Arc::new(conn),
            Err(err) => {
                return Err(Error::FailedToBindSocket(
                    self.endpoint.clone().udp_socket_addr(),
                    err.to_string(),
                ))
            }
        };

        let udp_listener_runtime = self.runtime.clone();
        let udp_listener_conn = conn.clone();
        let mut udp_listener_handle = udp_listener_runtime.spawn("udp_listener", async move {
            tracing::info!(listening_on = ?udp_socket_address, "UDP listener started");

            let mut buf = DEFAULT_UDP_PAYLOAD_BUF;
            loop {
                let Some(discovery_server_mailbox) = self.server_mailbox.as_ref() else {
                    tracing::error!("Discovery server mailbox not registered");
                    return Err(Error::DiscoveryServerNotAttached);
                };

                let incoming_message = udp_listener_conn.recv_from(&mut buf).await;
                let (msg_size, from) = match incoming_message {
                    Ok((msg_size, from)) => (msg_size, from),
                    Err(err) => {
                        tracing::error!(error = ?err, "Failed to receive message");
                        continue;
                    }
                };
                let Some(encoded_packet) = buf.get(..msg_size) else {
                    tracing::error!("Received empty message");
                    continue;
                };
                let packet = match Packet::decode(encoded_packet) {
                    Ok(packet) => packet,
                    Err(err) => {
                        tracing::error!(error = ?err, "Failed to decode packet");
                        continue;
                    }
                };

                tracing::info!("Received {packet} from {from}");

                if let Err(err) = discovery_server_mailbox.serve(packet, from).await {
                    tracing::error!(error = ?err, "Failed to relay message to server");
                    continue;
                }
            }
        });

        let mailbox_handler_runtime = self.runtime.clone();
        let mailbox_handler_connection = conn.clone();
        let mut mailbox_handler_handle = mailbox_handler_runtime.spawn("router_mailbox_handler", async move {
            tracing::info!(sending_from = ?udp_socket_address, "Mailbox handler started");

            loop {
                let message = self.receiver.recv().await.unwrap();
                match message {
                    Message::SendViaUDP(recipient, content) => {
                        tracing::debug!(to = ?recipient, "Relaying message");
                        if let Err(err) = mailbox_handler_connection.send_to(&content, recipient).await
                        {
                            tracing::error!(error = ?err, to = ?recipient, "Failed to relay message");
                        }
                    },
                    Message::SendViaTCP(_recipient) => todo!(),
                    Message::Terminate => {
                        tracing::info!("Shutting down actor");
                        return Ok(())
                    },
                }
            }
        });

        let result = tokio::select! {
            mailbox_handler_handle_result = &mut mailbox_handler_handle => {
                tracing::debug!("Mailbox handler finished, stopping listener");
                udp_listener_handle.abort();
                if let Err(err) = tokio::time::timeout(self.timeout_duration, udp_listener_handle).await {
                    tracing::error!(error = ?err, "Failed to stop listener");
                }
                mailbox_handler_handle_result
            }

            listener_handle_result = &mut udp_listener_handle => {
                tracing::debug!("Listener finished, stopping mailbox handler");
                mailbox_handler_handle.abort();
                if let Err(err) = tokio::time::timeout(self.timeout_duration, mailbox_handler_handle).await {
                    tracing::error!(error = ?err, "Failed to stop mailbox handler");
                }
                listener_handle_result
            }
        };

        match result {
            Ok(Ok(())) => {
                tracing::info!("Actor shutdown");
                Ok(())
            }
            Ok(Err(actor_error)) => {
                tracing::error!(error = ?actor_error, "Actor failed");
                Err(actor_error)
            }
            Err(commonware_runtime_error) => {
                tracing::error!(error = ?commonware_runtime_error, "Commonware runtime error");
                Err(Error::CommonwareRuntimeError(commonware_runtime_error))
            }
        }
    }
}
