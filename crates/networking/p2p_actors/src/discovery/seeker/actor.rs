use crate::{
    discovery::{
        packet::Packet,
        router,
        seeker::ingress::{Mailbox, Message},
        utils::{neighbors, new_find_node},
    },
    types::{Node, NodeId},
};
use commonware_runtime::Spawner;
use libsecp256k1::SecretKey;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to relay message: {0}")]
    FailedToRelayMessage(String),
    #[error("Failed to send heart beat")]
    FailedToSendHeartBeat,
    #[error("Commonware runtime error: {0}")]
    CommonwareRuntimeError(#[from] commonware_runtime::Error),
}

pub struct Config {
    pub signer: SecretKey,
    pub node_id: NodeId,
    pub seek_interval: std::time::Duration,
    pub timeout_duration: std::time::Duration,
}

pub struct Actor {
    runtime: commonware_runtime::tokio::Context,

    mailbox: Mailbox,
    receiver: mpsc::Receiver<Message>,

    // TODO: This should be the mailbox of a separate process
    peers: Arc<Mutex<BTreeMap<SocketAddr, Node>>>,

    router_mailbox: router::Mailbox,

    signer: SecretKey,
    node_id: NodeId,

    seek_interval: std::time::Duration,
    timeout_duration: std::time::Duration,
}

impl Actor {
    pub fn new(
        runtime: commonware_runtime::tokio::Context,
        router_mailbox: router::Mailbox,
        peers: Arc<Mutex<BTreeMap<SocketAddr, Node>>>,
        cfg: Config,
    ) -> (Self, Mailbox) {
        let (sender, receiver) = mpsc::channel(1);
        let mailbox = Mailbox::new(sender);
        let actor = Self {
            runtime,
            mailbox: mailbox.clone(),
            receiver,
            peers,
            router_mailbox,
            signer: cfg.signer,
            node_id: cfg.node_id,
            seek_interval: cfg.seek_interval,
            timeout_duration: cfg.timeout_duration,
        };
        (actor, mailbox)
    }

    pub async fn run(mut self) -> Result<(), Error> {
        let main_loop_runtime = self.runtime.clone();
        let mut main_loop_handle = main_loop_runtime.spawn("seeker", async move {
            loop {
                let message = self.receiver.recv().await.unwrap();
                tracing::debug!("Received message: {message:?}");
                match message {
                    Message::Seek(target) => {
                        tracing::info!("Seeking for peers");

                        let target_neighbors = neighbors(target, self.peers.clone()).await;

                        let packet_data = new_find_node(self.node_id);
                        let packet = Packet::new(packet_data, self.node_id);
                        let content = packet.encode(&self.signer);

                        for neighbor in target_neighbors.iter() {
                            self.router_mailbox
                                .relay(neighbor.endpoint.clone().udp_socket_addr(), content.clone())
                                .await
                                .map_err(|err| Error::FailedToRelayMessage(err.to_string()))?;
                        }
                    }
                    Message::Terminate => {
                        tracing::info!("Shutting down actor");
                        return Ok(());
                    }
                }
            }
        });

        let heart_beat_runtime = self.runtime.clone();
        let mut heart_beat_handle = heart_beat_runtime.spawn("seeker-heart-beat", async move {
            loop {
                tokio::time::sleep(self.seek_interval).await;
                if let Err(err) = self.mailbox.seek(self.node_id).await {
                    tracing::error!("Failed to send heart beat: {err}");
                    return Err(Error::FailedToSendHeartBeat);
                }
            }
        });

        tracing::info!("Seeker actor started");

        let result = tokio::select! {
            main_loop_result = &mut main_loop_handle => {
                tracing::debug!("Main loop finished, stopping heart beat");
                heart_beat_handle.abort();
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, heart_beat_handle).await {
                    tracing::error!("Failed to stop heart beat");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                main_loop_result
            },

            heart_beat_result = &mut heart_beat_handle => {
                tracing::debug!("Heart beat finished, stopping main loop");
                main_loop_handle.abort();
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, main_loop_handle).await {
                    tracing::error!("Failed to stop main loop");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                heart_beat_result
            },
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
                tracing::error!(error = ?commonware_runtime_error, "Actor runtime failed");
                Err(Error::CommonwareRuntimeError(commonware_runtime_error))
            }
        }
    }
}
