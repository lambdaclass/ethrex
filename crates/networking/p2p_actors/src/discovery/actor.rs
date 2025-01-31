use crate::{
    discovery::{
        ingress::{self, Mailbox, Message},
        packet::{Packet, PacketData, DEFAULT_UDP_PAYLOAD_BUF},
        utils::{neighbors, new_find_node, new_neighbors, new_ping, new_pong},
    },
    types::{Endpoint, Node, NodeId},
};
use commonware_runtime::Spawner;
use libsecp256k1::SecretKey;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use tokio::{
    net::UdpSocket,
    sync::{mpsc, Mutex},
};

#[allow(clippy::large_enum_variant)]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to bind socket {0}, reason: {1}")]
    FailedToBindSocket(SocketAddr, String),
    #[error("Failed to relay message: {0}")]
    FailedToRelayMessage(ingress::Error),
    #[error("Failed to replay message: {0}")]
    FailedToReplayMessage(std::io::Error),
    #[error("Failed to send revalidate message: {0}")]
    FailedToRevalidate(String),
    #[error("Failed to send lookup message: {0}")]
    FailedToLookup(String),
    #[error("Failed to send heart beat")]
    FailedToSendHeartBeat,
    #[error("Timeout")]
    Timeout,
    #[error("Commonware runtime error: {0}")]
    CommonwareRuntimeError(#[from] commonware_runtime::Error),
}

pub struct Config {
    pub endpoint: Endpoint,
    pub signer: SecretKey,
    pub node_id: NodeId,
    pub seek_interval: std::time::Duration,
    pub revalidation_interval: std::time::Duration,
    pub timeout_duration: std::time::Duration,
}

pub struct Actor {
    runtime: commonware_runtime::tokio::Context,

    mailbox: Mailbox,
    receiver: mpsc::Receiver<Message>,

    // TODO: This should be the mailbox of a separate process
    peers: Arc<Mutex<BTreeMap<SocketAddr, PeerData>>>,

    endpoint: Endpoint,
    signer: SecretKey,
    node_id: NodeId,

    lookup_interval: std::time::Duration,
    revalidation_interval: std::time::Duration,
    timeout_duration: std::time::Duration,
}

impl Actor {
    pub fn new(
        runtime: commonware_runtime::tokio::Context,
        peers: Arc<Mutex<BTreeMap<SocketAddr, PeerData>>>,
        cfg: Config,
    ) -> (Self, Mailbox) {
        let (sender, receiver) = mpsc::channel(1);
        let mailbox = Mailbox::new(sender);
        let actor = Self {
            runtime,
            mailbox: mailbox.clone(),
            receiver,
            peers,
            endpoint: cfg.endpoint,
            signer: cfg.signer,
            node_id: cfg.node_id,
            lookup_interval: cfg.seek_interval,
            revalidation_interval: cfg.revalidation_interval,
            timeout_duration: cfg.timeout_duration,
        };
        (actor, mailbox)
    }

    async fn send_after(
        label: String,
        runtime: commonware_runtime::tokio::Context,
        mailbox: Mailbox,
        message: Message,
        after: std::time::Duration,
    ) -> commonware_runtime::Handle<Result<(), Error>> {
        tracing::info!(task = ?label, "starting task");
        let label_clone = label.clone();
        runtime.spawn(&label, async move {
            let mut interval = tokio::time::interval(after);
            loop {
                interval.tick().await;
                tracing::info!(task = ?label_clone, "sending message");
                match message {
                    Message::Serve(_, _) => todo!(),
                    Message::Lookup(public_key) => mailbox
                        .lookup(public_key)
                        .await
                        .map_err(|err| Error::FailedToLookup(err.to_string()))?,
                    Message::Revalidate => mailbox
                        .revalidate()
                        .await
                        .map_err(|err| Error::FailedToRevalidate(err.to_string()))?,
                    Message::Terminate => todo!(),
                }
            }
        })
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

        let main_loop_conn = conn.clone();
        let main_loop_runtime = self.runtime.clone();
        let main_loop_mailbox = self.mailbox.clone();
        let mut main_loop_handle = main_loop_runtime.spawn("discovery", async move {
            tracing::info!("main loop started");
            loop {
                let message = self.receiver.recv().await.unwrap();
                match message {
                    Message::Serve(packet, from) => {
                        let packet_hash = packet.hash(&self.signer);
                        match packet.data {
                            PacketData::Ping {
                                from: from_endpoint,
                                ..
                            } => {
                                let mut table = self.peers.lock().await;
                                match table.entry(from) {
                                    Entry::Vacant(entry) => {
                                        entry.insert(PeerData::new_known(from_endpoint.clone()));
                                    }
                                    Entry::Occupied(mut entry) => {
                                        let peer_data = entry.get_mut();
                                        peer_data.last_ping_hash = Some(packet_hash);
                                        peer_data.last_ping = Some(current_unix_time());
                                    }
                                }
                                let ping_hash = packet_hash;
                                let pong_packet_data = new_pong(from_endpoint.clone(), ping_hash);
                                let pong_packet = Packet::new(pong_packet_data, self.node_id);
                                let content = pong_packet.encode(&self.signer);
                                main_loop_conn
                                    .send_to(&content, from_endpoint.udp_socket_addr())
                                    .await
                                    .map_err(Error::FailedToReplayMessage)?;
                                tracing::info!(packet = ?pong_packet, "replied to ping");
                            }
                            PacketData::Pong { ping_hash, .. } => {
                                let mut table = self.peers.lock().await;
                                match table.entry(from) {
                                    Entry::Vacant(_entry) => {
                                        tracing::debug!("received pong from unknown peer");
                                        continue;
                                    }
                                    Entry::Occupied(mut entry) => {
                                        let peer_data = entry.get_mut();
                                        if peer_data.last_ping_hash != Some(ping_hash) {
                                            tracing::warn!("received invalid pong");
                                            continue;
                                        }
                                        tracing::debug!("pong sender is {}", peer_data.state);
                                        match peer_data.state {
                                            NodeState::Known { .. } => {
                                                tracing::warn!(
                                                    "received pong from non-pinged known peer"
                                                );
                                            }
                                            NodeState::Pinged => {
                                                tracing::debug!("updating peer to proven");
                                                peer_data.state = NodeState::Proven {
                                                    last_pong: current_unix_time(),
                                                }
                            }
                                            NodeState::Proven { .. } => {
                                                tracing::debug!("updating peer last pong");
                                                peer_data.state = NodeState::Proven {
                                                    last_pong: current_unix_time(),
                                                }
                                            }
                                            NodeState::Connected { .. } => {
                                                tracing::debug!("updating peer last pong");
                                                peer_data.state = NodeState::Connected {
                                                    last_pong: current_unix_time(),
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            PacketData::FindNode { target, .. } => {
                                let neighbors = neighbors(target, self.peers.clone()).await;
                                let packet_data = new_neighbors(neighbors);
                                let packet = Packet::new(packet_data, self.node_id);
                                let content = packet.encode(&self.signer);
                                main_loop_conn
                                    .send_to(&content, from)
                                    .await
                                    .map_err(Error::FailedToReplayMessage)?;
                            }
                            PacketData::Neighbors { nodes, .. } => {
                                let mut table = self.peers.lock().await;
                                for node in nodes {
                                    match table.entry(node.endpoint.clone().udp_socket_addr()) {
                                        Entry::Vacant(entry) => {
                                            entry.insert(PeerData::new_known(node.endpoint));
                                        }
                                        Entry::Occupied(mut _entry) => {
                                            // TODO: What should we do here?
                                            continue;
                                        }
                                    }
                                    main_loop_mailbox.lookup(node.id).await.unwrap();
                                }
                            }
                            PacketData::ENRRequest { .. } => todo!(),
                            PacketData::ENRResponse { .. } => todo!(),
                        }
                    }
                    Message::Lookup(target) => {
                        let target_neighbors = neighbors(target, self.peers.clone()).await;
                        let packet_data = new_find_node(self.node_id);
                        let packet = Packet::new(packet_data, self.node_id);
                        let content = packet.encode(&self.signer);

                        for neighbor in target_neighbors.iter() {
                            main_loop_conn
                                .send_to(&content, neighbor.endpoint.clone().udp_socket_addr())
                                .await
                                .map_err(|err| Error::FailedToLookup(err.to_string()))?;
                        }
                    }
                    Message::Revalidate => {
                        let mut peers = self.peers.lock().await;
                        for (peer_address, peer_data) in peers.iter_mut() {
                            if matches!(peer_data.state, NodeState::Known)
                                || peer_data.last_ping.is_none()
                                || peer_data.last_ping.is_some_and(is_last_ping_expired)
                            {
                                let packet_data = new_ping(
                                    self.endpoint.clone(),
                                    &peer_data.endpoint.clone().udp_socket_addr(),
                                );
                                let packet = Packet::new(packet_data, self.node_id);
                                let content = packet.encode(&self.signer);
                                if matches!(peer_data.state, NodeState::Known) {
                                    peer_data.state = NodeState::Pinged;
                                    peer_data.last_ping = Some(current_unix_time());
                                    peer_data.last_ping_hash = Some(packet.hash(&self.signer));
                                }
                                main_loop_conn
                                    .send_to(&content, *peer_address)
                                    .await
                                    .map_err(|err| Error::FailedToRevalidate(err.to_string()))?;
                            }
                        }
                    }
                    Message::Terminate => {
                        tracing::info!("shutting down");
                        return Ok(());
                    }
                }
            }
        });

        let listener_conn = conn.clone();
        let listener_runtime = self.runtime.clone();
        let listener_mailbox = self.mailbox.clone();
        let mut listener_handle = listener_runtime.spawn("listener", async move {
            tracing::info!("listener started");
            let mut buf = DEFAULT_UDP_PAYLOAD_BUF;
            loop {
                let incoming_message = listener_conn.recv_from(&mut buf).await;
                let (msg_size, from) = match incoming_message {
                    Ok((msg_size, from)) => (msg_size, from),
                    Err(err) => {
                        tracing::error!(error = ?err, "failed to receive message");
                        continue;
                    }
                };
                let Some(encoded_packet) = buf.get(..msg_size) else {
                    tracing::error!("received empty message");
                    continue;
                };
                let packet = match Packet::decode(encoded_packet) {
                    Ok(packet) => packet,
                    Err(err) => {
                        tracing::error!(error = ?err, "failed to decode packet");
                        continue;
                    }
                };

                tracing::info!(packet = ?packet, from = ?from, "received packet");

                listener_mailbox
                    .serve(packet, from)
                    .await
                    .map_err(Error::FailedToRelayMessage)?;
            }
        });

        let mut revalidation_handle = Self::send_after(
            "revalidation".to_string(),
            self.runtime.clone(),
            self.mailbox.clone(),
            Message::Revalidate,
            self.revalidation_interval,
        )
        .await;

        let mut lookup_handle = Self::send_after(
            "lookup".to_string(),
            self.runtime.clone(),
            self.mailbox.clone(),
            Message::Lookup(self.node_id),
            self.lookup_interval,
        )
        .await;

        tracing::info!("Discovery started");

        let result = tokio::select! {
            lookup_handle_result = &mut lookup_handle => {
                tracing::debug!("Lookup task finished, stopping discovery");
                // We abort these because we do not have channels to send message them to terminate.
                revalidation_handle.abort();
                listener_handle.abort();
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, revalidation_handle).await {
                    tracing::error!("Failed to stop heart beat");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, listener_handle).await {
                    tracing::error!("Failed to stop listener");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, self.mailbox.terminate()).await {
                    tracing::error!("Failed to stop listener");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                lookup_handle_result
            },

            revalidation_handle_result = &mut revalidation_handle => {
                tracing::debug!("Revalidation task finished, stopping discovery");
                // We abort these because we do not have channels to send message them to terminate.
                lookup_handle.abort();
                listener_handle.abort();
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, lookup_handle).await {
                    tracing::error!("Failed to stop lookup");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, listener_handle).await {
                    tracing::error!("Failed to stop listener");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, self.mailbox.terminate()).await {
                    tracing::error!("Failed to stop listener");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                revalidation_handle_result
            },

            listener_handle_result = &mut listener_handle => {
                tracing::debug!("Listener task finished, stopping discovery");
                // We abort these because we do not have channels to send message them to terminate.
                lookup_handle.abort();
                revalidation_handle.abort();
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, lookup_handle).await {
                    tracing::error!("Failed to stop lookup");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, revalidation_handle).await {
                    tracing::error!("Failed to stop heart beat");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, self.mailbox.terminate()).await {
                    tracing::error!("Failed to stop listener");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                listener_handle_result
            },

            main_loop_handle_result = &mut main_loop_handle => {
                tracing::debug!("Main loop task finished, stopping discovery");
                // We abort these because we do not have channels to send message them to terminate.
                lookup_handle.abort();
                revalidation_handle.abort();
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, lookup_handle).await {
                    tracing::error!("Failed to stop lookup");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, revalidation_handle).await {
                    tracing::error!("Failed to stop heart beat");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                if let Err(_err) = tokio::time::timeout(self.timeout_duration, self.mailbox.terminate()).await {
                    tracing::error!("Failed to stop listener");
                    return Err(Error::CommonwareRuntimeError(commonware_runtime::Error::Timeout));
                }
                main_loop_handle_result
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
