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

const MAX_SCORE: i64 = 50;
const MIN_SCORE: i64 = -50;
/// Score assigned to peers who are acting maliciously (e.g., returning a node with wrong hash)
const MIN_SCORE_CRITICAL: i64 = MIN_SCORE * 3;

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
    /// This tracks if a peer is being used by a task
    /// So we can't use it yet
    in_use: bool,
    /// This tracks the score of a peer
    score: i64,
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
            in_use: false,
            score: Default::default(),
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

    //// Score management functions ////

    pub async fn get_score(&self, peer_id: &H256) -> i64 {
        self.get_score_opt(peer_id).await.unwrap_or(0)
    }

    async fn get_score_opt(&self, peer_id: &H256) -> Option<i64> {
        self.peers
            .lock()
            .await
            .get(peer_id)
            .map(|peer_data| peer_data.score)
    }

    pub async fn record_success(&self, peer_id: H256) {
        self.peers
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.score = (peer_data.score + 1).min(MAX_SCORE));
    }

    pub async fn record_failure(&self, peer_id: H256) {
        self.peers
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.score = (peer_data.score - 1).max(MIN_SCORE));
    }

    pub async fn record_critical_failure(&self, peer_id: H256) {
        self.peers
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.score = MIN_SCORE_CRITICAL);
    }

    pub async fn mark_in_use(&self, peer_id: H256) {
        self.peers
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.in_use = true);
    }

    pub async fn free_peer(&self, peer_id: H256) {
        self.peers
            .lock()
            .await
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.in_use = false);
    }

    pub async fn free_peers(&self) -> u64 {
        let mut free_count = 0;
        self.peers
            .lock()
            .await
            .iter_mut()
            .for_each(|(_, peer_data)| {
                if peer_data.in_use {
                    free_count += 1;
                }
                peer_data.in_use = false;
            });
        free_count
    }

    /// Returns the peer with the highest score and its peer channel.
    pub async fn get_peer_channel_with_highest_score(
        &self,
        capabilities: &[Capability],
    ) -> Option<(H256, PeerChannels)> {
        let peer_table = self.peers.lock().await;
        peer_table
            .iter()
            // We filter only to those peers which are useful to us
            .filter_map(|(id, peer_data)| {
                // If the peer is already in use right now, we skip it
                if peer_data.in_use {
                    return None;
                }

                // if the peer doesn't have all the capabilities we need, we skip it
                if !capabilities
                    .iter()
                    .all(|cap| peer_data.supported_capabilities.contains(cap))
                {
                    return None;
                }

                // if the peer doesn't have the channel open, we skip it.
                let peer_channel = peer_data.channels.clone()?;

                // We return the id, the score and the channel to connect with.
                Some((*id, peer_data.score, peer_channel))
            })
            .max_by_key(|(_, score, _)| *score)
            .map(|(k, _, v)| (k, v))
    }

    /// Returns the peer with the highest score and its peer channel, and marks it as used, if found.
    pub async fn get_peer_channel_with_highest_score_and_mark_as_used(
        &self,
        capabilities: &[Capability],
    ) -> Option<(H256, PeerChannels)> {
        let (peer_id, peer_channel) = self
            .get_peer_channel_with_highest_score(capabilities)
            .await?;

        self.mark_in_use(peer_id).await;

        Some((peer_id, peer_channel))
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
