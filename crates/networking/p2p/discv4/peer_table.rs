use crate::{
    rlpx::{self, connection::server::RLPxConnection, p2p::Capability},
    types::{Node, NodeRecord},
};
use ethrex_common::H256;
use spawned_concurrency::tasks::{CallResponse, CastResponse, GenServer, GenServerHandle};
use spawned_rt::tasks::mpsc;
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Instant,
};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::debug;

const MAX_SCORE: i64 = 50;
const MIN_SCORE: i64 = -50;
/// Score assigned to peers who are acting maliciously (e.g., returning a node with wrong hash)
const MIN_SCORE_CRITICAL: i64 = MIN_SCORE * 3;

pub type PeerTableHandle = GenServerHandle<PeerTable>;

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

#[derive(Debug)]
pub struct PeerTable {
    pub table: BTreeMap<H256, Contact>,
    pub peers: BTreeMap<H256, PeerData>,
    pub already_tried_peers: HashSet<H256>,
    pub discarded_contacts: HashSet<H256>,
}

impl PeerTable {
    pub fn spawn() -> PeerTableHandle {
        let peer_table = Self::new();
        peer_table.start()
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub async fn new_connected_peer(
        peer_table: &mut PeerTableHandle,
        node: Node,
        channels: PeerChannels,
        capabilities: Vec<Capability>,
    ) -> Result<(), PeerTableError> {
        peer_table
            .call(CallMessage::NewConnectedPeer {
                node,
                channels,
                capabilities,
            })
            .await
            .map_err(|e| PeerTableError::InternalError(e.to_string()))?;
        Ok(())
    }

    pub async fn remove_peer(
        peer_table: &mut PeerTableHandle,
        node_id: H256,
    ) -> Result<(), PeerTableError> {
        peer_table
            .call(CallMessage::RemovePeer { node_id })
            .await
            .map_err(|e| PeerTableError::InternalError(e.to_string()))?;
        Ok(())
    }

    pub async fn new_contact(
        peer_table: &mut PeerTableHandle,
        node_id: H256,
        contact: Contact,
    ) -> Result<(), PeerTableError> {
        peer_table
            .call(CallMessage::NewContact { node_id, contact })
            .await
            .map_err(|e| PeerTableError::InternalError(e.to_string()))?;
        Ok(())
    }

    pub async fn set_unwanted(
        peer_table: &mut PeerTableHandle,
        node_id: &H256,
    ) -> Result<(), PeerTableError> {
        peer_table
            .call(CallMessage::SetUnwanted { node_id: *node_id })
            .await
            .map_err(|e| PeerTableError::InternalError(e.to_string()))?;
        Ok(())
    }

    pub async fn peer_count(peer_table: &mut PeerTableHandle) -> Result<usize, PeerTableError> {
        if let OutMessage::PeerCount(peer_count) = peer_table
            .call(CallMessage::PeerCount)
            .await
            .map_err(|e| PeerTableError::InternalError(e.to_string()))?
        {
            Ok(peer_count)
        } else {
            Err(PeerTableError::InternalError(
                "Failed to obtain peers".to_owned(),
            ))
        }
    }

    pub async fn prune(peer_table: &mut PeerTableHandle) -> Result<(), PeerTableError> {
        peer_table
            .cast(CastMessage::Prune)
            .await
            .map_err(|e| PeerTableError::InternalError(e.to_string()))?;
        Ok(())
    }

    pub async fn get_peer_channels(
        &self,
        _capabilities: &[Capability],
    ) -> Vec<(H256, PeerChannels)> {
        self.peers
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
        let peer_data = self.peers.get(&peer_id)?;
        peer_data.channels.clone()
    }

    //// Score management functions ////

    pub async fn get_score(&self, peer_id: &H256) -> i64 {
        self.get_score_opt(peer_id).await.unwrap_or(0)
    }

    async fn get_score_opt(&self, peer_id: &H256) -> Option<i64> {
        self.peers.get(peer_id).map(|peer_data| peer_data.score)
    }

    pub async fn record_success(&mut self, peer_id: H256) {
        self.peers
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.score = (peer_data.score + 1).min(MAX_SCORE));
    }

    pub async fn record_failure(&mut self, peer_id: H256) {
        self.peers
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.score = (peer_data.score - 1).max(MIN_SCORE));
    }

    pub async fn record_critical_failure(&mut self, peer_id: H256) {
        self.peers
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.score = MIN_SCORE_CRITICAL);
    }

    pub async fn mark_in_use(&mut self, peer_id: H256) {
        self.peers
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.in_use = true);
    }

    pub async fn free_peer(&mut self, peer_id: H256) {
        self.peers
            .entry(peer_id)
            .and_modify(|peer_data| peer_data.in_use = false);
    }

    /// Returns the peer with the highest score and its peer channel.
    pub async fn get_peer_channel_with_highest_score(
        &self,
        capabilities: &[Capability],
    ) -> Option<(H256, PeerChannels)> {
        self.peers
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
        &mut self,
        capabilities: &[Capability],
    ) -> Option<(H256, PeerChannels)> {
        let (peer_id, peer_channel) = self
            .get_peer_channel_with_highest_score(capabilities)
            .await?;

        self.mark_in_use(peer_id).await;

        Some((peer_id, peer_channel))
    }

    fn prune_internal(&mut self) {
        let disposable_contacts = self
            .table
            .iter()
            .filter_map(|(c_id, c)| c.disposable.then_some(*c_id))
            .collect::<Vec<_>>();

        for contact_to_discard_id in disposable_contacts {
            self.table.remove(&contact_to_discard_id);
            self.discarded_contacts.insert(contact_to_discard_id);
        }
    }
}

impl Default for PeerTable {
    fn default() -> Self {
        Self {
            table: BTreeMap::new(),
            peers: BTreeMap::new(),
            already_tried_peers: HashSet::new(),
            discarded_contacts: HashSet::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum CallMessage {
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
    NewContact {
        node_id: H256,
        contact: Contact,
    },
    PeerCount,
}

#[derive(Debug)]
pub enum OutMessage {
    Ok,
    PeerCount(usize),
}

#[derive(Clone, Debug)]
pub enum CastMessage {
    Prune,
}

#[derive(Debug, Error)]
pub enum PeerTableError {
    #[error("{0}")]
    InternalError(String),
}

impl GenServer for PeerTable {
    type CallMsg = CallMessage;
    type CastMsg = CastMessage;
    type OutMsg = OutMessage;
    type Error = PeerTableError;

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        _handle: &PeerTableHandle,
    ) -> CallResponse<Self> {
        match message {
            CallMessage::NewConnectedPeer {
                node,
                channels,
                capabilities,
            } => {
                debug!("New peer connected");
                let new_peer_id = node.node_id();
                let new_peer = PeerData::new(node, None, channels, capabilities);
                self.peers.insert(new_peer_id, new_peer);
            }
            CallMessage::RemovePeer { node_id } => {
                self.peers.remove(&node_id);
            }
            CallMessage::SetUnwanted { node_id } => {
                if let Some(contact) = self.table.get_mut(&node_id) {
                    contact.unwanted = true;
                }
            }
            CallMessage::NewContact { node_id, contact } => {
                self.table.insert(node_id, contact);
            }
            CallMessage::PeerCount => {
                return CallResponse::Reply(Self::OutMsg::PeerCount(self.peers.len()));
            }
        }
        CallResponse::Reply(Self::OutMsg::Ok)
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &PeerTableHandle,
    ) -> CastResponse {
        match message {
            CastMessage::Prune => self.prune_internal(),
        }
        CastResponse::NoReply
    }
}
