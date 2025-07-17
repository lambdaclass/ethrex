use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use ethrex_common::H256;
use spawned_concurrency::tasks::GenServerHandle;
use spawned_rt::tasks::mpsc;
use tokio::sync::Mutex;
use tracing::info;

use crate::{
    rlpx::{self, connection::server::RLPxConnection, p2p::Capability},
    types::{Node, NodeRecord},
};

#[derive(Debug, Clone)]
pub struct Contact {
    pub node: Node,
    pub n_find_node_sent: u64,
    // This contact failed to respond our Ping.
    pub disposable: bool,
    // Set to true after we send a successful ENRResponse to it.
    pub knows_us: bool,
}

impl From<Node> for Contact {
    fn from(node: Node) -> Self {
        Self {
            node,
            n_find_node_sent: 0,
            disposable: false,
            knows_us: true,
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
    // pub last_ping: u64,
    // pub last_pong: u64,
    // pub last_ping_hash: Option<H256>,
    // pub is_proven: bool,
    // pub find_node_request: Option<FindNodeRequest>,
    // pub enr_request_hash: Option<H256>,
    // /// a ration to track the peers's ping responses
    // pub liveness: u16,
    // /// if a revalidation was sent to the peer, the bool marks if it has answered
    // pub revalidation: Option<bool>,
    // /// Starts as false when a node is added. Set to true when a connection becomes active. When a
    // /// connection fails, the peer record is removed, so no need to set it to false.
    // pub is_connected: bool,
    // /// Simple peer score: +1 for success, -1 for failure
    // pub score: i32,
}

impl PeerData {
    pub fn new(node: Node, record: Option<NodeRecord>, channels: PeerChannels) -> Self {
        Self {
            node,
            record,
            supported_capabilities: Vec::new(),
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

    pub async fn set_connected_peer(&mut self, node: Node, channels: PeerChannels) {
        info!("New peer connected");

        let new_peer_id = node.node_id();

        let new_peer = PeerData::new(node, None, channels);

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
