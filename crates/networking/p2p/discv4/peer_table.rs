use crate::{
    discv4::server::MAX_NODES_IN_NEIGHBORS_PACKET,
    metrics::METRICS,
    rlpx::{self, connection::server::RLPxConnection, p2p::Capability},
    types::{Node, NodeRecord},
};
use ethrex_common::{H256, U256};
use rand::seq::SliceRandom;
use spawned_concurrency::{
    error::GenServerError,
    tasks::{CallResponse, CastResponse, GenServer, GenServerHandle},
};
use spawned_rt::tasks::mpsc;
use std::{
    collections::{BTreeMap, HashSet, btree_map::Entry},
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{debug, info};

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

    pub fn record_ping_sent(&mut self, ping_hash: H256) {
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
        channels: Option<PeerChannels>,
        capabilities: Vec<Capability>,
    ) -> Self {
        Self {
            node,
            record,
            supported_capabilities: capabilities,
            is_connection_inbound: false,
            channels,
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
#[derive(Clone, Debug)]
pub struct PeerTableHandle(GenServerHandle<PeerTable>);

impl PeerTableHandle {
    /// We received a list of Nodes to contact. No conection has been established yet.
    pub async fn new_contacts(
        &mut self,
        nodes: Vec<Node>,
        local_node_id: H256,
    ) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::NewContacts {
                nodes,
                local_node_id,
            })
            .await?;
        Ok(())
    }

    /// We have established a connection with the remote peer.
    pub async fn new_connected_peer(
        &mut self,
        node: Node,
        channels: PeerChannels,
        capabilities: Vec<Capability>,
    ) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::NewConnectedPeer {
                node,
                channels,
                capabilities,
            })
            .await?;
        Ok(())
    }

    /// Remove from list of connected peers.
    pub async fn remove_peer(&mut self, node_id: H256) -> Result<(), PeerTableError> {
        self.0.cast(CastMessage::RemovePeer { node_id }).await?;
        Ok(())
    }

    /// Mark node as not wanted
    pub async fn set_unwanted(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::SetUnwanted { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Mark peer as in use
    pub async fn mark_in_use(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::MarkInUse { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Remove "in use" mark for peer
    pub async fn free_peer(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::FreePeer { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Record a successful connection, used to score peers
    pub async fn record_success(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::RecordSuccess { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Record a failed connection, used to score peers
    pub async fn record_failure(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::RecordFailure { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Record a critical failure for connection, used to score peers
    pub async fn record_critical_failure(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::RecordCriticalFailure { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Record ping sent, store the ping hash for later check
    pub async fn record_ping_sent(
        &mut self,
        node_id: &H256,
        hash: H256,
    ) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::RecordPingSent {
                node_id: *node_id,
                hash,
            })
            .await?;
        Ok(())
    }

    /// Record a pong received. Check previously saved hash and reset it if it matches
    pub async fn record_pong_received(
        &mut self,
        node_id: &H256,
        ping_hash: H256,
    ) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::RecordPongReceived {
                node_id: *node_id,
                ping_hash,
            })
            .await?;
        Ok(())
    }

    /// Set peer as disposable
    pub async fn set_disposable(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::SetDisposable { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Increment FindNode message counter for peer
    pub async fn increment_find_node_sent(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::IncrementFindNodeSent { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Set flag for peer that tells that it knows us
    pub async fn knows_us(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.0
            .cast(CastMessage::KnowsUs { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Remove from list of contacts the ones marked as disposable
    pub async fn prune(&mut self) -> Result<(), PeerTableError> {
        self.0.cast(CastMessage::Prune).await?;
        Ok(())
    }

    /// Return the amount of connected peers
    pub async fn peer_count(&mut self) -> Result<usize, PeerTableError> {
        match self.0.call(CallMessage::PeerCount).await? {
            OutMessage::PeerCount(peer_count) => Ok(peer_count),
            _ => unreachable!(),
        }
    }

    /// Return the amount of connected peers that matches any of the given capabilities
    pub async fn peer_count_by_capabilities(
        &mut self,
        capabilities: &[Capability],
    ) -> Result<usize, PeerTableError> {
        match self
            .0
            .call(CallMessage::PeerCountByCapabilities {
                capabilities: capabilities.to_vec(),
            })
            .await?
        {
            OutMessage::PeerCount(peer_count) => Ok(peer_count),
            _ => unreachable!(),
        }
    }

    /// Remove the "in use" mark for all peers
    pub async fn free_peers(&mut self) -> Result<usize, PeerTableError> {
        match self.0.call(CallMessage::FreePeers).await? {
            OutMessage::PeerCount(result) => Ok(result),
            _ => unreachable!(),
        }
    }

    /// Check if target number of contacts and connected peers is reached
    pub async fn target_reached(
        &mut self,
        target_contacts: usize,
        target_peers: usize,
    ) -> Result<bool, PeerTableError> {
        match self
            .0
            .call(CallMessage::TargetReached {
                target_contacts,
                target_peers,
            })
            .await?
        {
            OutMessage::TargetReached(result) => Ok(result),
            _ => unreachable!(),
        }
    }

    /// Get all contacts available to initiate a connection
    pub async fn get_contacts_to_initiate(
        &mut self,
        amount: usize,
    ) -> Result<Vec<Contact>, PeerTableError> {
        match self
            .0
            .call(CallMessage::GetContactsToInitiate(amount))
            .await?
        {
            OutMessage::Contacts(contacts) => Ok(contacts),
            _ => unreachable!(),
        }
    }

    /// Get all contacts available for lookup
    pub async fn get_contacts_for_lookup(
        &mut self,
        amount: usize,
    ) -> Result<Vec<Contact>, PeerTableError> {
        match self
            .0
            .call(CallMessage::GetContactsForLookup(amount))
            .await?
        {
            OutMessage::Contacts(contacts) => Ok(contacts),
            _ => unreachable!(),
        }
    }

    /// Get all contacts available to revalidate
    pub async fn get_contacts_to_revalidate(
        &mut self,
        revalidation_interval: Duration,
    ) -> Result<Vec<Contact>, PeerTableError> {
        match self
            .0
            .call(CallMessage::GetContactsToRevalidate(revalidation_interval))
            .await?
        {
            OutMessage::Contacts(contacts) => Ok(contacts),
            _ => unreachable!(),
        }
    }

    /// Returns the peer with the highest score and its peer channel.
    pub async fn get_best_peer(
        &mut self,
        capabilities: &[Capability],
    ) -> Result<Option<(H256, PeerChannels)>, PeerTableError> {
        match self
            .0
            .call(CallMessage::GetBestPeer {
                capabilities: capabilities.to_vec(),
            })
            .await?
        {
            OutMessage::FoundPeer {
                node_id,
                peer_channels,
            } => Ok(Some((node_id, peer_channels))),
            OutMessage::NotFound => Ok(None),
            _ => unreachable!(),
        }
    }

    /// Returns the peer with the highest score and its peer channel, and marks it as used, if found.
    pub async fn use_best_peer(
        &mut self,
        capabilities: &[Capability],
    ) -> Result<Option<(H256, PeerChannels)>, PeerTableError> {
        match self
            .0
            .call(CallMessage::UseBestPeer {
                capabilities: capabilities.to_vec(),
            })
            .await?
        {
            OutMessage::FoundPeer {
                node_id,
                peer_channels,
            } => Ok(Some((node_id, peer_channels))),
            OutMessage::NotFound => Ok(None),
            _ => unreachable!(),
        }
    }

    /// Get peer score
    pub async fn get_score(&mut self, node_id: &H256) -> Result<i64, PeerTableError> {
        match self
            .0
            .call(CallMessage::GetScore { node_id: *node_id })
            .await?
        {
            OutMessage::PeerScore(score) => Ok(score),
            _ => unreachable!(),
        }
    }

    /// Get list of connected peers
    pub async fn get_connected_nodes(&mut self) -> Result<Vec<Node>, PeerTableError> {
        match self.0.call(CallMessage::GetConnectedNodes).await? {
            OutMessage::Nodes(nodes) => Ok(nodes),
            _ => unreachable!(),
        }
    }

    /// Get list of connected peers with their capabilities
    pub async fn get_peers_with_capabilities(
        &mut self,
    ) -> Result<Vec<(H256, PeerChannels, Vec<Capability>)>, PeerTableError> {
        match self.0.call(CallMessage::GetPeersWithCapabilities).await? {
            OutMessage::PeersWithCapabilities(peers_with_capabilities) => {
                Ok(peers_with_capabilities)
            }
            _ => unreachable!(),
        }
    }

    /// Get peer channels for communication
    pub async fn get_peer_channels(
        &mut self,
        capabilities: &[Capability],
    ) -> Result<Vec<(H256, PeerChannels)>, PeerTableError> {
        match self
            .0
            .call(CallMessage::GetPeerChannels {
                capabilities: capabilities.to_vec(),
            })
            .await?
        {
            OutMessage::PeerChannels(peer_channels) => Ok(peer_channels),
            _ => unreachable!(),
        }
    }

    /// Insert new peer if it is new. Returns a boolean telling if it was new or not.
    pub async fn insert_if_new(&mut self, node: &Node) -> Result<bool, PeerTableError> {
        match self
            .0
            .call(CallMessage::InsertIfNew { node: node.clone() })
            .await?
        {
            OutMessage::IsNew(is_new) => Ok(is_new),
            _ => unreachable!(),
        }
    }

    /// Validate a contact
    pub async fn validate_contact(
        &mut self,
        node_id: &H256,
        sender_ip: IpAddr,
    ) -> Result<OutMessage, PeerTableError> {
        self.0
            .call(CallMessage::ValidateContact {
                node_id: *node_id,
                sender_ip,
            })
            .await
            .map_err(PeerTableError::InternalError)
    }

    /// Get closest nodes according to kademlia's distance
    pub async fn get_closest_nodes(&mut self, node_id: &H256) -> Result<Vec<Node>, PeerTableError> {
        match self
            .0
            .call(CallMessage::GetClosestNodes { node_id: *node_id })
            .await?
        {
            OutMessage::Nodes(nodes) => Ok(nodes),
            _ => unreachable!(),
        }
    }

    /// Get metadata associated to peer
    pub async fn get_peers_data(&mut self) -> Result<Vec<PeerData>, PeerTableError> {
        match self.0.call(CallMessage::GetPeersData).await? {
            OutMessage::PeersData(peers_data) => Ok(peers_data),
            _ => unreachable!(),
        }
    }

    /// Retrieve a random peer.
    pub async fn get_random_peer(
        &mut self,
        capabilities: &[Capability],
    ) -> Result<Option<(H256, PeerChannels)>, PeerTableError> {
        match self
            .0
            .call(CallMessage::GetRandomPeer {
                capabilities: capabilities.to_vec(),
            })
            .await?
        {
            OutMessage::FoundPeer {
                node_id,
                peer_channels,
            } => Ok(Some((node_id, peer_channels))),
            OutMessage::NotFound => Ok(None),
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Default)]
pub struct PeerTable {
    contacts: BTreeMap<H256, Contact>,
    peers: BTreeMap<H256, PeerData>,
    already_tried_peers: HashSet<H256>,
    discarded_contacts: HashSet<H256>,
}

impl PeerTable {
    pub fn spawn() -> PeerTableHandle {
        PeerTableHandle(Self::default().start())
    }

    // Internal functions //

    fn get_best_peer(&self, capabilities: &[Capability]) -> Option<(H256, PeerChannels)> {
        self.peers
            .iter()
            // We filter only to those peers which are useful to us
            .filter_map(|(id, peer_data)| {
                // If the peer is already in use right now, we skip it
                if peer_data.in_use {
                    return None;
                }

                // if the peer doesn't have any of the capabilities we need, we skip it
                if !capabilities
                    .iter()
                    .any(|cap| peer_data.supported_capabilities.contains(cap))
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
    fn use_best_peer(&mut self, capabilities: &[Capability]) -> Option<(H256, PeerChannels)> {
        let (peer_id, peer_channel) = self.get_best_peer(capabilities)?;

        self.peers
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.in_use = true);

        Some((peer_id, peer_channel))
    }

    fn prune(&mut self) {
        let disposable_contacts = self
            .contacts
            .iter()
            .filter_map(|(c_id, c)| c.disposable.then_some(*c_id))
            .collect::<Vec<_>>();

        for contact_to_discard_id in disposable_contacts {
            self.contacts.remove(&contact_to_discard_id);
            self.discarded_contacts.insert(contact_to_discard_id);
        }
    }

    fn get_contacts_to_initiate(&mut self, max_amount: usize) -> Vec<Contact> {
        let mut contacts = Vec::new();
        let mut tried_connections = 0;

        for contact in self.contacts.values() {
            let node_id = contact.node.node_id();
            if !self.peers.contains_key(&node_id)
                && !self.already_tried_peers.contains(&node_id)
                && contact.knows_us
                && !contact.unwanted
            {
                self.already_tried_peers.insert(node_id);

                contacts.push(contact.clone());

                tried_connections += 1;
                if tried_connections >= max_amount {
                    break;
                }
            }
        }

        if tried_connections < max_amount {
            info!("Resetting list of tried peers.");
            self.already_tried_peers.clear();
        }

        contacts
    }

    fn get_contacts_for_lookup(&mut self, max_amount: usize) -> Vec<Contact> {
        self.contacts
            .values()
            .take(max_amount)
            .filter(|c| !c.disposable)
            .cloned()
            .collect()
    }

    fn get_contacts_to_revalidate(&mut self, revalidation_interval: Duration) -> Vec<Contact> {
        self.contacts
            .values()
            .filter(|c| Self::is_validation_needed(c, revalidation_interval))
            .cloned()
            .collect()
    }

    fn validate_contact(&mut self, node_id: H256, sender_ip: IpAddr) -> OutMessage {
        let Some(contact) = self.contacts.get(&node_id) else {
            return OutMessage::UnknownContact;
        };
        if !contact.was_validated() {
            return OutMessage::InvalidContact;
        }

        // Check that the IP address from which we receive the request matches the one we have stored to prevent amplification attacks
        // This prevents an attack vector where the discovery protocol could be used to amplify traffic in a DDOS attack.
        // A malicious actor would send a findnode request with the IP address and UDP port of the target as the source address.
        // The recipient of the findnode packet would then send a neighbors packet (which is a much bigger packet than findnode) to the victim.
        if sender_ip != contact.node.ip {
            return OutMessage::IpMismatch;
        }
        OutMessage::ValidContact(contact.clone())
    }

    fn get_closest_nodes(&mut self, node_id: H256) -> Vec<Node> {
        let mut nodes: Vec<(Node, usize)> = vec![];

        for (contact_id, contact) in &self.contacts {
            let distance = Self::distance(&node_id, contact_id);
            if nodes.len() < MAX_NODES_IN_NEIGHBORS_PACKET {
                nodes.push((contact.node.clone(), distance));
            } else {
                for (i, (_, dis)) in &mut nodes.iter().enumerate() {
                    if distance < *dis {
                        nodes[i] = (contact.node.clone(), distance);
                        break;
                    }
                }
            }
        }
        nodes.into_iter().map(|(node, _distance)| node).collect()
    }

    async fn new_contacts(&mut self, nodes: Vec<Node>, local_node_id: H256) {
        for node in nodes {
            let node_id = node.node_id();
            if let Entry::Vacant(vacant_entry) = self.contacts.entry(node_id) {
                if !self.discarded_contacts.contains(&node_id) && node_id != local_node_id {
                    vacant_entry.insert(Contact::from(node));
                    METRICS.record_new_discovery().await;
                }
            };
        }
    }

    fn peer_count_by_capabilities(&mut self, capabilities: Vec<Capability>) -> usize {
        self.peers
            .iter()
            .filter_map(|(node_id, peer_data)| {
                // if the peer doesn't have any of the capabilities we need, we skip it
                if !capabilities
                    .iter()
                    .any(|cap| peer_data.supported_capabilities.contains(cap))
                {
                    None
                } else {
                    Some(*node_id)
                }
            })
            .collect::<Vec<_>>()
            .len()
    }

    fn get_peer_channels(&mut self, capabilities: Vec<Capability>) -> Vec<(H256, PeerChannels)> {
        self.peers
            .iter()
            .filter_map(|(peer_id, peer_data)| {
                // if the peer doesn't have any of the capabilities we need, we skip it
                if !capabilities
                    .iter()
                    .any(|cap| peer_data.supported_capabilities.contains(cap))
                {
                    return None;
                }
                peer_data
                    .channels
                    .clone()
                    .map(|peer_channels| (*peer_id, peer_channels))
            })
            .collect()
    }

    fn get_random_peer(&mut self, capabilities: Vec<Capability>) -> Option<(H256, PeerChannels)> {
        let mut peers: Vec<(H256, PeerChannels)> = self
            .peers
            .iter()
            .filter_map(|(node_id, peer_data)| {
                // if the peer doesn't have any of the capabilities we need, we skip it
                if !capabilities
                    .iter()
                    .any(|cap| peer_data.supported_capabilities.contains(cap))
                {
                    return None;
                }
                peer_data
                    .channels
                    .clone()
                    .map(|peer_channels| (*node_id, peer_channels))
            })
            .collect();
        peers.shuffle(&mut rand::rngs::OsRng);
        peers.first().cloned()
    }

    fn distance(node_id_1: &H256, node_id_2: &H256) -> usize {
        let xor = node_id_1 ^ node_id_2;
        let distance = U256::from_big_endian(xor.as_bytes());
        distance.bits().saturating_sub(1)
    }

    fn is_validation_needed(contact: &Contact, revalidation_interval: Duration) -> bool {
        let sent_ping_ttl = Duration::from_secs(30);

        let validation_is_stale = !contact.was_validated()
            || contact
                .validation_timestamp
                .map(|ts| Instant::now().saturating_duration_since(ts) > revalidation_interval)
                .unwrap_or(false);

        let sent_ping_is_stale = contact
            .validation_timestamp
            .map(|ts| Instant::now().saturating_duration_since(ts) > sent_ping_ttl)
            .unwrap_or(false);

        !contact.disposable || validation_is_stale || sent_ping_is_stale
    }
}

#[derive(Clone, Debug)]
pub enum CastMessage {
    NewContacts {
        nodes: Vec<Node>,
        local_node_id: H256,
    },
    NewConnectedPeer {
        node: Node,
        channels: PeerChannels,
        capabilities: Vec<Capability>,
    },
    RemovePeer {
        node_id: H256,
    },
    SetUnwanted {
        node_id: H256,
    },
    MarkInUse {
        node_id: H256,
    },
    FreePeer {
        node_id: H256,
    },
    RecordSuccess {
        node_id: H256,
    },
    RecordFailure {
        node_id: H256,
    },
    RecordCriticalFailure {
        node_id: H256,
    },
    RecordPingSent {
        node_id: H256,
        hash: H256,
    },
    RecordPongReceived {
        node_id: H256,
        ping_hash: H256,
    },
    SetDisposable {
        node_id: H256,
    },
    IncrementFindNodeSent {
        node_id: H256,
    },
    KnowsUs {
        node_id: H256,
    },
    Prune,
}

#[derive(Clone, Debug)]
pub enum CallMessage {
    PeerCount,
    PeerCountByCapabilities {
        capabilities: Vec<Capability>,
    },
    FreePeers,
    TargetReached {
        target_contacts: usize,
        target_peers: usize,
    },
    GetContactsToInitiate(usize),
    GetContactsForLookup(usize),
    GetContactsToRevalidate(Duration),
    GetBestPeer {
        capabilities: Vec<Capability>,
    },
    UseBestPeer {
        capabilities: Vec<Capability>,
    },
    GetScore {
        node_id: H256,
    },
    GetConnectedNodes,
    GetPeersWithCapabilities,
    GetPeerChannels {
        capabilities: Vec<Capability>,
    },
    InsertIfNew {
        node: Node,
    },
    ValidateContact {
        node_id: H256,
        sender_ip: IpAddr,
    },
    GetClosestNodes {
        node_id: H256,
    },
    GetPeersData,
    GetRandomPeer {
        capabilities: Vec<Capability>,
    },
}

#[derive(Debug)]
pub enum OutMessage {
    PeerCount(usize),
    FoundPeer {
        node_id: H256,
        peer_channels: PeerChannels,
    },
    NotFound,
    PeerScore(i64),
    PeersWithCapabilities(Vec<(H256, PeerChannels, Vec<Capability>)>),
    PeerChannels(Vec<(H256, PeerChannels)>),
    Contacts(Vec<Contact>),
    TargetReached(bool),
    IsNew(bool),
    Nodes(Vec<Node>),
    ValidContact(Contact),
    InvalidContact,
    UnknownContact,
    IpMismatch,
    PeersData(Vec<PeerData>),
}

#[derive(Debug, Error)]
pub enum PeerTableError {
    #[error("Internal error: {0}")]
    InternalError(#[from] GenServerError),
}

impl GenServer for PeerTable {
    type CallMsg = CallMessage;
    type CastMsg = CastMessage;
    type OutMsg = OutMessage;
    type Error = PeerTableError;

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        _handle: &GenServerHandle<PeerTable>,
    ) -> CallResponse<Self> {
        match message {
            CallMessage::PeerCount => {
                CallResponse::Reply(Self::OutMsg::PeerCount(self.peers.len()))
            }
            CallMessage::PeerCountByCapabilities { capabilities } => CallResponse::Reply(
                OutMessage::PeerCount(self.peer_count_by_capabilities(capabilities)),
            ),
            CallMessage::FreePeers => CallResponse::Reply(Self::OutMsg::PeerCount(
                self.peers
                    .iter_mut()
                    .filter_map(|(_, peer_data)| {
                        if peer_data.in_use {
                            peer_data.in_use = false;
                            Some(peer_data)
                        } else {
                            None
                        }
                    })
                    .count(),
            )),
            CallMessage::TargetReached {
                target_contacts,
                target_peers,
            } => CallResponse::Reply(Self::OutMsg::TargetReached(
                self.contacts.len() < target_contacts && self.peers.len() < target_peers,
            )),
            CallMessage::GetContactsToInitiate(amount) => CallResponse::Reply(
                Self::OutMsg::Contacts(self.get_contacts_to_initiate(amount)),
            ),
            CallMessage::GetContactsForLookup(amount) => {
                CallResponse::Reply(Self::OutMsg::Contacts(self.get_contacts_for_lookup(amount)))
            }
            CallMessage::GetContactsToRevalidate(revalidation_interval) => CallResponse::Reply(
                Self::OutMsg::Contacts(self.get_contacts_to_revalidate(revalidation_interval)),
            ),
            CallMessage::GetBestPeer { capabilities } => {
                let channels = self.get_best_peer(&capabilities);
                CallResponse::Reply(channels.map_or(
                    Self::OutMsg::NotFound,
                    |(node_id, peer_channels)| Self::OutMsg::FoundPeer {
                        node_id,
                        peer_channels,
                    },
                ))
            }
            CallMessage::UseBestPeer { capabilities } => {
                let channels = self.use_best_peer(&capabilities);
                CallResponse::Reply(channels.map_or(
                    Self::OutMsg::NotFound,
                    |(node_id, peer_channels)| Self::OutMsg::FoundPeer {
                        node_id,
                        peer_channels,
                    },
                ))
            }
            CallMessage::GetScore { node_id } => CallResponse::Reply(Self::OutMsg::PeerScore(
                self.peers
                    .get(&node_id)
                    .map(|peer_data| peer_data.score)
                    .unwrap_or_default(),
            )),
            CallMessage::GetConnectedNodes => CallResponse::Reply(Self::OutMsg::Nodes(
                self.peers
                    .values()
                    .map(|peer_data| peer_data.node.clone())
                    .collect(),
            )),
            CallMessage::GetPeersWithCapabilities => {
                CallResponse::Reply(Self::OutMsg::PeersWithCapabilities(
                    self.peers
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
                        .collect(),
                ))
            }
            CallMessage::GetPeerChannels { capabilities } => CallResponse::Reply(
                OutMessage::PeerChannels(self.get_peer_channels(capabilities)),
            ),
            CallMessage::InsertIfNew { node } => CallResponse::Reply(Self::OutMsg::IsNew(
                match self.contacts.entry(node.node_id()) {
                    Entry::Occupied(_) => false,
                    Entry::Vacant(entry) => {
                        entry.insert(Contact::from(node));
                        true
                    }
                },
            )),
            CallMessage::ValidateContact { node_id, sender_ip } => {
                CallResponse::Reply(self.validate_contact(node_id, sender_ip))
            }
            CallMessage::GetClosestNodes { node_id } => {
                CallResponse::Reply(Self::OutMsg::Nodes(self.get_closest_nodes(node_id)))
            }
            CallMessage::GetPeersData => CallResponse::Reply(OutMessage::PeersData(
                self.peers.values().cloned().collect(),
            )),
            CallMessage::GetRandomPeer { capabilities } => CallResponse::Reply(
                if let Some((node_id, peer_channels)) = self.get_random_peer(capabilities) {
                    OutMessage::FoundPeer {
                        node_id,
                        peer_channels,
                    }
                } else {
                    OutMessage::NotFound
                },
            ),
        }
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<PeerTable>,
    ) -> CastResponse {
        match message {
            CastMessage::NewContacts {
                nodes,
                local_node_id,
            } => {
                self.new_contacts(nodes, local_node_id).await;
            }
            CastMessage::NewConnectedPeer {
                node,
                channels,
                capabilities,
            } => {
                debug!("New peer connected");
                let new_peer_id = node.node_id();
                let new_peer = PeerData::new(node, None, Some(channels), capabilities);
                self.peers.insert(new_peer_id, new_peer);
            }
            CastMessage::RemovePeer { node_id } => {
                self.peers.remove(&node_id);
            }
            CastMessage::SetUnwanted { node_id } => {
                self.contacts
                    .entry(node_id)
                    .and_modify(|contact| contact.unwanted = true);
            }
            CastMessage::MarkInUse { node_id } => {
                self.peers
                    .entry(node_id)
                    .and_modify(|peer_data| peer_data.in_use = true);
            }
            CastMessage::FreePeer { node_id } => {
                self.peers
                    .entry(node_id)
                    .and_modify(|peer_data| peer_data.in_use = false);
            }
            CastMessage::RecordSuccess { node_id } => {
                self.peers
                    .entry(node_id)
                    .and_modify(|peer_data| peer_data.score = (peer_data.score + 1).min(MAX_SCORE));
            }
            CastMessage::RecordFailure { node_id } => {
                self.peers
                    .entry(node_id)
                    .and_modify(|peer_data| peer_data.score = (peer_data.score - 1).max(MIN_SCORE));
            }
            CastMessage::RecordCriticalFailure { node_id } => {
                self.peers
                    .entry(node_id)
                    .and_modify(|peer_data| peer_data.score = MIN_SCORE_CRITICAL);
            }
            CastMessage::RecordPingSent { node_id, hash } => {
                self.contacts
                    .entry(node_id)
                    .and_modify(|contact| contact.record_ping_sent(hash));
            }
            CastMessage::RecordPongReceived { node_id, ping_hash } => {
                // If entry does not exist or hash does not match, ignore pong record
                // Otherwise, reset ping_hash
                self.contacts.entry(node_id).and_modify(|contact| {
                    if contact
                        .ping_hash
                        .map(|value| value == ping_hash)
                        .unwrap_or(false)
                    {
                        contact.ping_hash = None
                    }
                });
            }
            CastMessage::SetDisposable { node_id } => {
                self.contacts
                    .entry(node_id)
                    .and_modify(|contact| contact.disposable = true);
            }
            CastMessage::IncrementFindNodeSent { node_id } => {
                self.contacts
                    .entry(node_id)
                    .and_modify(|contact| contact.n_find_node_sent += 1);
            }
            CastMessage::KnowsUs { node_id } => {
                self.contacts
                    .entry(node_id)
                    .and_modify(|c| c.knows_us = true);
            }
            CastMessage::Prune => self.prune(),
        }
        CastResponse::NoReply
    }
}
