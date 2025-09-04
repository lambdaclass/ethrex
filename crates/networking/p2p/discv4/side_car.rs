use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use keccak_hash::H256;
use rand::rngs::OsRng;
use secp256k1::SecretKey;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_after, send_interval},
};
use tokio::{net::UdpSocket, sync::Mutex};
use tracing::{debug, error, info};

use crate::{
    discv4::messages::{FindNodeMessage, Message, PingMessage},
    kademlia::{Contact, Kademlia},
    metrics::METRICS,
    types::{Endpoint, Node, NodeRecord},
    utils::{get_msg_expiration_from_seconds, public_key_from_signing_key},
};

#[derive(Debug, thiserror::Error)]
pub enum DiscoverySideCarError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Failed to send message")]
    MessageSendFailure(std::io::Error),
    #[error("Only partial message was sent")]
    PartialMessageSent,
}

#[derive(Debug, Clone)]
pub struct DiscoverySideCar {
    local_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SecretKey,
    udp_socket: Arc<UdpSocket>,

    /// Interval between revalidation checks.
    revalidation_check_interval: Duration,
    /// Interval between revalidations.
    revalidation_interval: Duration,

    /// The initial interval between peer lookups, until the number of peers reaches
    /// [target_peers](DiscoverySideCarState::target_peers), or the number of
    /// contacts reaches [target_contacts](DiscoverySideCarState::target_contacts).
    initial_lookup_interval: Duration,
    lookup_interval: Duration,

    prune_interval: Duration,

    /// The target number of RLPx connections to reach.
    target_peers: u64,
    /// The target number of contacts to maintain in the Kademlia table.
    target_contacts: u64,

    kademlia: Kademlia,
}

impl DiscoverySideCar {
    pub fn new(
        local_node: Node,
        local_node_record: Arc<Mutex<NodeRecord>>,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
    ) -> Self {
        Self {
            local_node,
            local_node_record,
            signer,
            udp_socket,
            kademlia,

            revalidation_check_interval: Duration::from_secs(12 * 60 * 60), // 12 hours
            revalidation_interval: Duration::from_secs(12 * 60 * 60),       // 12 hours

            initial_lookup_interval: Duration::from_secs(5),
            lookup_interval: Duration::from_secs(5 * 60), // 5 minutes

            prune_interval: Duration::from_secs(5),

            target_peers: 100,
            target_contacts: 100_000,
        }
    }

    async fn ping(&self, node: &Node) -> Result<H256, DiscoverySideCarError> {
        let mut buf = Vec::new();

        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let from = Endpoint {
            ip: self.local_node.ip,
            udp_port: self.local_node.udp_port,
            tcp_port: self.local_node.tcp_port,
        };

        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let enr_seq = self.local_node_record.lock().await.seq;

        let ping = Message::Ping(PingMessage::new(from, to, expiration).with_enr_seq(enr_seq));

        ping.encode_with_header(&mut buf, &self.signer);

        let ping_hash: [u8; 32] = buf[..32]
            .try_into()
            .expect("first 32 bytes are the message hash");

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, SocketAddr::new(node.ip.to_canonical(), node.udp_port))
            .await
            .map_err(DiscoverySideCarError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoverySideCarError::PartialMessageSent);
        }

        debug!(sent = "Ping", to = %format!("{:#x}", node.public_key));

        Ok(H256::from(ping_hash))
    }

    async fn send_find_node(&self, node: &Node) -> Result<(), DiscoverySideCarError> {
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let random_priv_key = SecretKey::new(&mut OsRng);
        let random_pub_key = public_key_from_signing_key(&random_priv_key);

        let msg = Message::FindNode(FindNodeMessage::new(random_pub_key, expiration));

        let mut buf = Vec::new();
        msg.encode_with_header(&mut buf, &self.signer);

        // Nodes that use ipv6 currently are only ipv4 masked addresses, so we can convert it to an ipv4 address.
        // If in the future we have real ipv6 nodes, we will need to handle them differently.
        let bytes_sent = self
            .udp_socket
            .send_to(&buf, SocketAddr::new(node.ip.to_canonical(), node.udp_port))
            .await
            .map_err(DiscoverySideCarError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoverySideCarError::PartialMessageSent);
        }

        debug!(sent = "FindNode", to = %format!("{:#x}", node.public_key));

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Revalidate,
    Lookup,
    Prune,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

impl DiscoverySideCar {
    pub async fn spawn(
        local_node: Node,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
    ) -> Result<(), DiscoverySideCarError> {
        info!("Starting Discovery Side Car");

        let local_node_record = Arc::new(Mutex::new(
            NodeRecord::from_node(&local_node, 1, &signer)
                .expect("Failed to create local node record"),
        ));

        let state =
            DiscoverySideCar::new(local_node, local_node_record, signer, udp_socket, kademlia);

        let mut server = state.clone().start();

        send_interval(
            state.revalidation_check_interval,
            server.clone(),
            InMessage::Revalidate,
        );

        send_interval(state.prune_interval, server.clone(), InMessage::Prune);

        let _ = server.cast(InMessage::Lookup).await;

        Ok(())
    }

    async fn revalidate(&self) {
        for contact in self.kademlia.table.lock().await.values_mut() {
            if contact.disposable || !self.is_validation_needed(contact) {
                continue;
            }

            match self.ping(&contact.node).await {
                Ok(ping_hash) => {
                    METRICS.record_ping_sent().await;
                    contact.validation_timestamp = Some(Instant::now());
                    contact.ping_hash = Some(ping_hash);
                }
                Err(err) => {
                    error!(sent = "Ping", to = %format!("{:#x}", contact.node.public_key), err = ?err);

                    contact.disposable = true;

                    METRICS.record_new_discarded_node().await;
                }
            }
        }
    }

    async fn lookup(&self) {
        for contact in self.kademlia.table.lock().await.values_mut() {
            if contact.n_find_node_sent == 20 || contact.disposable {
                continue;
            }

            if let Err(err) = self.send_find_node(&contact.node).await {
                error!(sent = "FindNode", to = %format!("{:#x}", contact.node.public_key), err = ?err);
                contact.disposable = true;
                METRICS.record_new_discarded_node().await;
            }

            contact.n_find_node_sent += 1;
        }
    }

    async fn prune(&self) {
        let mut contacts = self.kademlia.table.lock().await;
        let mut discarded_contacts = self.kademlia.discarded_contacts.lock().await;

        let disposable_contacts = contacts
            .iter()
            .filter_map(|(c_id, c)| c.disposable.then_some(*c_id))
            .collect::<Vec<_>>();

        for contact_to_discard_id in disposable_contacts {
            contacts.remove(&contact_to_discard_id);
            discarded_contacts.insert(contact_to_discard_id);
        }
    }

    fn is_validation_needed(&self, contact: &Contact) -> bool {
        let sent_ping_ttl = Duration::from_secs(30);

        let validation_is_stale = !contact.was_validated()
            || contact
                .validation_timestamp
                .map(|ts| Instant::now().saturating_duration_since(ts) > self.revalidation_interval)
                .unwrap_or(false);

        let sent_ping_is_stale = contact
            .validation_timestamp
            .map(|ts| Instant::now().saturating_duration_since(ts) > sent_ping_ttl)
            .unwrap_or(false);

        validation_is_stale || sent_ping_is_stale
    }

    async fn get_lookup_interval(&self) -> Duration {
        let number_of_contacts = self.kademlia.table.lock().await.len() as u64;
        let number_of_peers = self.kademlia.peers.lock().await.len() as u64;
        if number_of_peers < self.target_peers && number_of_contacts < self.target_contacts {
            self.initial_lookup_interval
        } else {
            info!("Reached target number of peers or contacts. Using longer lookup interval.");
            self.lookup_interval
        }
    }
}

impl GenServer for DiscoverySideCar {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = DiscoverySideCarError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::Revalidate => {
                debug!(received = "Revalidate");

                self.revalidate().await;

                CastResponse::NoReply
            }
            Self::CastMsg::Lookup => {
                debug!(received = "Lookup");

                self.lookup().await;

                let interval = self.get_lookup_interval().await;
                send_after(interval, handle.clone(), Self::CastMsg::Lookup);

                CastResponse::NoReply
            }
            Self::CastMsg::Prune => {
                debug!(received = "Prune");

                self.prune().await;

                CastResponse::NoReply
            }
        }
    }
}
