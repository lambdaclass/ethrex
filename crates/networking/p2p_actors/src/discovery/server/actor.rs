use crate::{
    discovery::{
        packet::{Packet, PacketData},
        router, seeker,
        server::ingress::{Mailbox, Message},
        utils::{neighbors, new_neighbors, new_pong},
    },
    types::{Node, NodeId},
};
use libsecp256k1::SecretKey;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to relay message: {0}")]
    FailedToRelayMessage(String),
    #[error("Commonware runtime error: {0}")]
    CommonwareRuntimeError(#[from] commonware_runtime::Error),
}

pub struct Config {
    pub signer: SecretKey,
    pub node_id: NodeId,
}

pub struct Actor {
    mailbox: Mailbox,
    receiver: mpsc::Receiver<Message>,

    router_mailbox: router::Mailbox,
    seeker_mailbox: seeker::Mailbox,

    signer: SecretKey,
    node_id: NodeId,

    peers: Arc<Mutex<BTreeMap<SocketAddr, Node>>>,
}

impl Actor {
    pub fn new(
        router_mailbox: router::Mailbox,
        seeker_mailbox: seeker::Mailbox,
        kademlia: Arc<Mutex<BTreeMap<SocketAddr, Node>>>,
        cfg: Config,
    ) -> (Self, Mailbox) {
        let (sender, receiver) = mpsc::channel(32);
        let mailbox = Mailbox::new(sender);
        let actor = Self {
            mailbox: mailbox.clone(),
            receiver,
            router_mailbox,
            seeker_mailbox,
            signer: cfg.signer,
            node_id: cfg.node_id,
            peers: kademlia,
        };
        (actor, mailbox)
    }

    pub async fn run(mut self) -> Result<(), Error> {
        tracing::info!("Discovery server actor started");
        loop {
            let message = self.receiver.recv().await.unwrap();
            tracing::info!("Received message: {message:?}");
            match message {
                Message::Ping(packet) => {
                    let ping_hash = packet.hash(&self.signer);

                    let PacketData::Ping { from, .. } = packet.data else {
                        continue;
                    };

                    let pong_packet_data = new_pong(from.clone(), ping_hash);
                    let pong_packet = Packet::new(pong_packet_data, self.node_id);

                    if let Err(err) = self
                        .router_mailbox
                        .relay(from.udp_socket_addr(), pong_packet.encode(&self.signer))
                        .await
                    {
                        return Err(Error::FailedToRelayMessage(err.to_string()));
                    }
                }
                Message::Pong(_packet) => {}
                Message::FindNode(packet, from) => {
                    let PacketData::FindNode { target, .. } = packet.data else {
                        continue;
                    };

                    let neighbors = neighbors(target, self.peers.clone()).await;

                    let packet_data = new_neighbors(neighbors);
                    let packet = Packet::new(packet_data, self.node_id);

                    let content = packet.encode(&self.signer);

                    if let Err(err) = self.router_mailbox.relay(from, content).await {
                        return Err(Error::FailedToRelayMessage(err.to_string()));
                    }
                }
                Message::Neighbors(packet) => {
                    let PacketData::Neighbors { nodes, .. } = packet.data else {
                        continue;
                    };

                    let mut table = self.peers.lock().await;

                    for node in nodes {
                        table.insert(node.endpoint.clone().udp_socket_addr(), node.clone());
                        // TODO: Spawn a RLPx connection to the new peer.

                        self.seeker_mailbox.seek(node.id).await.unwrap();
                    }
                }
                Message::ENRRequest(_packet) => todo!(),
                Message::ENRResponse(_packet) => todo!(),
                Message::Terminate => {
                    tracing::info!("Shutting down actor");
                    return Ok(());
                }
            }
        }
    }
}
