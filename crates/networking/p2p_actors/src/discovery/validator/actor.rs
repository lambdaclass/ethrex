use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use crate::{
    discovery::{
        packet::Packet,
        router,
        utils::new_ping,
        validator::ingress::{Mailbox, Message},
    },
    types::{Endpoint, Node, NodeId},
};
use commonware_runtime::Spawner;
use libsecp256k1::SecretKey;
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

#[derive(Debug, Clone)]
pub struct Config {
    pub endpoint: Endpoint,
    pub signer: SecretKey,
    pub node_id: NodeId,
    pub revalidation_interval: std::time::Duration,
    pub timeout_duration: std::time::Duration,
}

pub struct Actor {
    runtime: commonware_runtime::tokio::Context,

    mailbox: Mailbox,
    receiver: mpsc::Receiver<Message>,

    peers: Arc<Mutex<BTreeMap<SocketAddr, Node>>>,

    router_mailbox: router::Mailbox,

    signer: SecretKey,
    node_id: NodeId,
    endpoint: Endpoint,

    revalidation_interval: std::time::Duration,
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
            endpoint: cfg.endpoint,
            revalidation_interval: cfg.revalidation_interval,
            timeout_duration: cfg.timeout_duration,
        };
        (actor, mailbox)
    }

    pub async fn run(mut self) -> Result<(), Error> {
        let main_loop_runtime = self.runtime.clone();
        let mut main_loop_handle = main_loop_runtime.spawn("seeker", async move {
            loop {
                let message = self.receiver.recv().await.unwrap();
                tracing::info!("Received message: {message:?}");
                match message {
                    Message::Validate => {
                        let peers = self.peers.lock().await;
                        for (peer_address, peer) in peers.iter() {
                            let packet_data = new_ping(
                                self.endpoint.clone(),
                                &peer.endpoint.clone().udp_socket_addr(),
                            );
                            let packet = Packet::new(packet_data, self.node_id);
                            let content = packet.encode(&self.signer);

                            if let Err(err) =
                                self.router_mailbox.relay(*peer_address, content).await
                            {
                                tracing::error!(error = ?err, "Failed to relay message");
                                return Err(Error::FailedToRelayMessage(err.to_string()));
                            }
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
                tokio::time::sleep(self.revalidation_interval).await;
                if let Err(err) = self.mailbox.validate().await {
                    tracing::error!(error = ?err, "Failed to send heart beat");
                    return Err(Error::FailedToSendHeartBeat);
                }
            }
        });

        tracing::info!("Validator actor started");

        let result = tokio::select! {
            main_loop_result = &mut main_loop_handle => {
                tracing::debug!("Main loop finished, stopping heart beat");
                heart_beat_handle.abort();
                if let Err(err) = tokio::time::timeout(self.timeout_duration, heart_beat_handle).await {
                    tracing::error!(error = ?err, "Failed to stop heart beat");
                }
                main_loop_result
            },

            heart_beat_result = &mut heart_beat_handle => {
                tracing::debug!("Heart beat finished, stopping main loop");
                main_loop_handle.abort();
                if let Err(err) = tokio::time::timeout(self.timeout_duration, main_loop_handle).await {
                    tracing::error!(error = ?err, "Failed to stop main loop");
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
