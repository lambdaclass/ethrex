use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Instant,
};

use ethrex_common::H256;
use spawned_concurrency::tasks::GenServerHandle;
use spawned_rt::tasks::mpsc;
use tokio::sync::Mutex;
use tracing::debug;

use crate::{
    rlpx::{self, connection::server::RLPxConnection, p2p::Capability},
    types::{Node, NodeRecord},
};

#[derive(Debug, Clone)]
pub struct Contact {
    pub node: Node,
    /// The timestamp when the contact was last sent a ping.
    /// If None, the contact has never been pinged.
    pub validation_timestamp: Option<Instant>,
    /// The hash of the last unacknowledged ping sent to this contact, or
    /// None if no ping was sent yet or it was already acknowledged.
    pub ping_hash: Option<H256>,

    pub n_find_node_sent: u64,
    // This contact failed to respond our Ping.
    pub disposable: bool,
    // Set to true after we send a successful ENRResponse to it.
    pub knows_us: bool,
    // This is a known-bad peer (on another network, no matching capabilities, etc)
    pub unwanted: bool,
}

impl Contact {
    pub fn was_validated(&self) -> bool {
        self.validation_timestamp.is_some() && !self.has_pending_ping()
    }

    pub fn has_pending_ping(&self) -> bool {
        self.ping_hash.is_some()
    }

    pub fn record_sent_ping(&mut self, ping_hash: H256) {
        self.validation_timestamp = Some(Instant::now());
        self.ping_hash = Some(ping_hash);
    }
}

impl From<Node> for Contact {
    fn from(node: Node) -> Self {
        Self {
            node,
            validation_timestamp: None,
            ping_hash: None,
            n_find_node_sent: 0,
            disposable: false,
            knows_us: true,
            unwanted: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeerData {
    pub node: Node,
    pub record: Option<NodeRecord>,
    pub supported_capabilities: Vec<Capability>,
    /// Set to true if the connection is inbound (aka the connection was started by the peer and not by this node)
    /// It is only valid as long as is_connected is true
    pub is_connection_inbound: bool,
    /// communication channels between the peer data and its active connection
    pub channels: Option<PeerChannels>,
}

impl PeerData {
    pub fn new(
        node: Node,
        record: Option<NodeRecord>,
        channels: PeerChannels,
        capabilities: Vec<Capability>,
    ) -> Self {
        Self {
            node,
            record,
            supported_capabilities: capabilities,
            is_connection_inbound: false,
            channels: Some(channels),
        }
    }
}

#[derive(Debug, Clone)]
/// Holds the respective sender and receiver ends of the communication channels between the peer data and its active connection
pub struct PeerChannels {
    pub connection: GenServerHandle<RLPxConnection>,
    pub receiver: Arc<Mutex<mpsc::Receiver<rlpx::Message>>>,
}

impl PeerChannels {
    /// Sets up the communication channels for the peer
    /// Returns the channel endpoints to send to the active connection's listen loop
    pub(crate) fn create(
        connection: GenServerHandle<RLPxConnection>,
    ) -> (Self, mpsc::Sender<rlpx::Message>) {
        let (connection_sender, receiver) = mpsc::channel::<rlpx::Message>();
        (
            Self {
                connection,
                receiver: Arc::new(Mutex::new(receiver)),
            },
            connection_sender,
        )
    }
}

#[derive(Debug, Clone)]
pub struct Kademlia {
    pub table: Arc<Mutex<BTreeMap<H256, Contact>>>,
    pub peers: Arc<Mutex<BTreeMap<H256, PeerData>>>,
    pub already_tried_peers: Arc<Mutex<HashSet<H256>>>,
    pub discarded_contacts: Arc<Mutex<HashSet<H256>>>,
    pub discovered_mainnet_peers: Arc<Mutex<HashSet<H256>>>,
}

impl Kademlia {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set_connected_peer(
        &mut self,
        node: Node,
        channels: PeerChannels,
        capabilities: Vec<Capability>,
    ) {
        debug!("New peer connected");

        let new_peer_id = node.node_id();

        let new_peer = PeerData::new(node, None, channels, capabilities);

        self.peers.lock().await.insert(new_peer_id, new_peer);
    }

    pub async fn get_peer_channels(
        &self,
        _capabilities: &[Capability],
    ) -> Vec<(H256, PeerChannels)> {
        self.peers
            .lock()
            .await
            .iter()
            .filter_map(|(peer_id, peer_data)| {
                peer_data
                    .channels
                    .clone()
                    .map(|peer_channels| (*peer_id, peer_channels))
            })
            .collect()
    }

    pub async fn get_peer_channels_with_capabilities(
        &self,
        _capabilities: &[Capability],
    ) -> Vec<(H256, PeerChannels, Vec<Capability>)> {
        self.peers
            .lock()
            .await
            .iter()
            .filter_map(|(peer_id, peer_data)| {
                peer_data.channels.clone().map(|peer_channels| {
                    (
                        *peer_id,
                        peer_channels,
                        peer_data.supported_capabilities.clone(),
                    )
                })
            })
            .collect()
    }

    pub async fn get_peer_channel(&self, peer_id: H256) -> Option<PeerChannels> {
        let peers = self.peers.lock().await;
        let peer_data = peers.get(&peer_id)?;
        peer_data.channels.clone()
    }
}

impl Default for Kademlia {
    fn default() -> Self {
        Self {
            table: Arc::new(Mutex::new(BTreeMap::new())),
            peers: Arc::new(Mutex::new(BTreeMap::new())),
            already_tried_peers: Arc::new(Mutex::new(HashSet::new())),
            discarded_contacts: Arc::new(Mutex::new(HashSet::new())),
            discovered_mainnet_peers: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}
