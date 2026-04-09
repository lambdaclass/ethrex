//! Unified peer table for both discv4 and discv5 discovery protocols.
//!
//! This module provides a protocol-agnostic peer table that stores contact
//! information discovered through either discv4 or discv5. The key abstraction
//! is using `Bytes` for ping identifiers:
//! - discv4: converts H256 ping hash to Bytes
//! - discv5: already uses Bytes for req_id
//!
//! Each contact is tagged with the protocol that discovered it, allowing
//! protocol-specific lookups to only query compatible contacts.

use crate::{
    backend,
    metrics::METRICS,
    rlpx::{connection::server::PeerConnection, p2p::Capability},
    types::{Node, NodeRecord},
    utils::distance,
};
use bytes::Bytes;
use ethrex_common::H256;
use ethrex_storage::Store;
use indexmap::{IndexMap, map::Entry};
use rand::seq::{IteratorRandom, SliceRandom};
use rustc_hash::{FxHashMap, FxHashSet};
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, Response, send_message_on},
};
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

const MAX_SCORE: i64 = 50;
const MIN_SCORE: i64 = -50;
/// Score assigned to peers who are acting maliciously (e.g., returning a node with wrong hash)
const MIN_SCORE_CRITICAL: i64 = MIN_SCORE * 3;
/// Maximum amount of FindNode messages sent to a single node.
const MAX_FIND_NODE_PER_PEER: u64 = 20;
/// Score weight for the load balancing function.
const SCORE_WEIGHT: i64 = 1;
/// Weight for amount of requests being handled by the peer for the load balancing function.
const REQUESTS_WEIGHT: i64 = 1;
/// Max amount of ongoing requests per peer.
const MAX_CONCURRENT_REQUESTS_PER_PEER: i64 = 100;
/// The target number of RLPx connections to reach.
pub const TARGET_PEERS: usize = 100;
/// The target number of contacts to maintain in peer_table.
const TARGET_CONTACTS: usize = 100_000;
/// Maximum number of ENRs to return in a FindNode response (discv4 compatible).
pub(crate) const MAX_NODES_IN_NEIGHBORS_PACKET: usize = 16;
/// Maximum number of ENRs to return in a discv5 FindNode response.
const MAX_ENRS_PER_FINDNODE_RESPONSE: usize = 16;

/// Identifies which discovery protocol was used to find a contact.
/// This allows protocol-specific lookups to only query compatible contacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiscoveryProtocol {
    /// Contact discovered via discv4 protocol
    Discv4,
    /// Contact discovered via discv5 protocol
    Discv5,
}

/// Session information for discv5 protocol.
/// Contains symmetric keys derived from ECDH for message encryption/decryption.
pub use crate::discv5::session::Session;

#[derive(Debug, Clone)]
pub struct Contact {
    pub node: Node,
    /// Whether this contact is reachable via discv4 protocol.
    pub is_discv4: bool,
    /// Whether this contact is reachable via discv5 protocol.
    pub is_discv5: bool,
    /// The timestamp when the contact was last sent a ping.
    /// If None, the contact has never been pinged.
    pub validation_timestamp: Option<Instant>,
    /// The identifier of the last unacknowledged ping sent to this contact, or
    /// None if no ping was sent yet or it was already acknowledged.
    /// - discv4: H256 hash converted to Bytes
    /// - discv5: request ID as Bytes
    pub ping_id: Option<Bytes>,

    /// The hash of the last unacknowledged ENRRequest sent to this contact, or
    /// None if no request was sent yet or it was already acknowledged.
    pub enr_request_hash: Option<H256>,

    pub n_find_node_sent: u64,
    /// ENR associated with this contact, if it was provided by the peer.
    pub record: Option<NodeRecord>,
    /// This contact failed to respond our Ping.
    pub disposable: bool,
    /// Set to true after we send a successful ENRResponse to it.
    pub knows_us: bool,
    /// This is a known-bad peer (on another network, no matching capabilities, etc)
    pub unwanted: bool,
    /// Whether the last known fork ID is valid, None if unknown.
    pub is_fork_id_valid: Option<bool>,
    /// Session information for discv5 (None for discv4 contacts)
    session: Option<Session>,
}

impl Contact {
    pub fn was_validated(&self) -> bool {
        self.validation_timestamp.is_some() && !self.has_pending_ping()
    }

    pub fn has_pending_ping(&self) -> bool {
        self.ping_id.is_some()
    }

    pub fn record_ping_sent(&mut self, ping_id: Bytes) {
        self.validation_timestamp = Some(Instant::now());
        self.ping_id = Some(ping_id);
    }

    pub fn record_enr_request_sent(&mut self, request_hash: H256) {
        self.enr_request_hash = Some(request_hash);
    }

    // If hash does not match, ignore. Otherwise, reset enr_request_hash
    pub fn record_enr_response_received(&mut self, request_hash: H256, record: NodeRecord) {
        if self
            .enr_request_hash
            .take_if(|h| *h == request_hash)
            .is_some()
        {
            self.record = Some(record);
        }
    }

    pub fn has_pending_enr_request(&self) -> bool {
        self.enr_request_hash.is_some()
    }
}

impl Contact {
    pub fn new(node: Node, protocol: DiscoveryProtocol) -> Self {
        Self {
            node,
            is_discv4: protocol == DiscoveryProtocol::Discv4,
            is_discv5: protocol == DiscoveryProtocol::Discv5,
            validation_timestamp: None,
            ping_id: None,
            enr_request_hash: None,
            n_find_node_sent: 0,
            record: None,
            disposable: false,
            knows_us: true,
            unwanted: false,
            is_fork_id_valid: None,
            session: None,
        }
    }

    /// Check if this contact supports the given protocol.
    pub fn supports_protocol(&self, protocol: DiscoveryProtocol) -> bool {
        match protocol {
            DiscoveryProtocol::Discv4 => self.is_discv4,
            DiscoveryProtocol::Discv5 => self.is_discv5,
        }
    }

    /// Mark this contact as supporting the given protocol.
    pub fn add_protocol(&mut self, protocol: DiscoveryProtocol) {
        match protocol {
            DiscoveryProtocol::Discv4 => self.is_discv4 = true,
            DiscoveryProtocol::Discv5 => self.is_discv5 = true,
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
    pub connection: Option<PeerConnection>,
    /// This tracks the score of a peer
    score: i64,
    /// Track the amount of concurrent requests this peer is handling
    requests: i64,
}

impl PeerData {
    pub fn new(
        node: Node,
        record: Option<NodeRecord>,
        connection: Option<PeerConnection>,
        capabilities: Vec<Capability>,
    ) -> Self {
        Self {
            node,
            record,
            supported_capabilities: capabilities,
            is_connection_inbound: false,
            connection,
            score: Default::default(),
            requests: Default::default(),
        }
    }
}

/// Result of contact validation.
#[derive(Debug, Clone)]
pub enum ContactValidation {
    Valid(Box<Contact>),
    InvalidContact,
    UnknownContact,
    IpMismatch,
}

#[protocol]
pub trait PeerTableServerProtocol: Send + Sync {
    // Send (cast) methods
    fn new_contacts(
        &self,
        nodes: Vec<Node>,
        local_node_id: H256,
        protocol: DiscoveryProtocol,
    ) -> Result<(), ActorError>;
    fn new_contact_records(
        &self,
        node_records: Vec<NodeRecord>,
        local_node_id: H256,
    ) -> Result<(), ActorError>;
    fn new_connected_peer(
        &self,
        node: Node,
        connection: PeerConnection,
        capabilities: Vec<Capability>,
    ) -> Result<(), ActorError>;
    fn set_session_info(&self, node_id: H256, session: Session) -> Result<(), ActorError>;
    fn remove_peer(&self, node_id: H256) -> Result<(), ActorError>;
    fn inc_requests(&self, node_id: H256) -> Result<(), ActorError>;
    fn dec_requests(&self, node_id: H256) -> Result<(), ActorError>;
    fn set_unwanted(&self, node_id: H256) -> Result<(), ActorError>;
    fn set_is_fork_id_valid(&self, node_id: H256, valid: bool) -> Result<(), ActorError>;
    fn record_success(&self, node_id: H256) -> Result<(), ActorError>;
    fn record_failure(&self, node_id: H256) -> Result<(), ActorError>;
    fn record_critical_failure(&self, node_id: H256) -> Result<(), ActorError>;
    fn record_ping_sent(&self, node_id: H256, ping_id: Bytes) -> Result<(), ActorError>;
    fn record_pong_received(&self, node_id: H256, ping_id: Bytes) -> Result<(), ActorError>;
    fn record_enr_request_sent(&self, node_id: H256, request_hash: H256) -> Result<(), ActorError>;
    fn record_enr_response_received(
        &self,
        node_id: H256,
        request_hash: H256,
        record: NodeRecord,
    ) -> Result<(), ActorError>;
    fn set_disposable(&self, node_id: H256) -> Result<(), ActorError>;
    fn increment_find_node_sent(&self, node_id: H256) -> Result<(), ActorError>;
    fn mark_knows_us(&self, node_id: H256) -> Result<(), ActorError>;
    fn prune_table(&self) -> Result<(), ActorError>;
    fn shutdown(&self) -> Result<(), ActorError>;

    // Request (call) methods
    fn peer_count(&self) -> Response<usize>;
    fn peer_count_by_capabilities(&self, capabilities: Vec<Capability>) -> Response<usize>;
    fn target_reached(&self) -> Response<bool>;
    fn target_peers_reached(&self) -> Response<bool>;
    fn target_peers_completion(&self) -> Response<f64>;
    fn get_contact_to_initiate(&self) -> Response<Option<Box<Contact>>>;
    fn get_contact_for_lookup(&self, protocol: DiscoveryProtocol)
    -> Response<Option<Box<Contact>>>;
    fn get_contact_for_enr_lookup(&self) -> Response<Option<Box<Contact>>>;
    fn get_contact(&self, node_id: H256) -> Response<Option<Box<Contact>>>;
    fn get_contact_to_revalidate(
        &self,
        revalidation_interval: Duration,
        protocol: DiscoveryProtocol,
    ) -> Response<Option<Box<Contact>>>;
    fn get_best_peer(
        &self,
        capabilities: Vec<Capability>,
    ) -> Response<Option<(H256, PeerConnection)>>;
    fn get_score(&self, node_id: H256) -> Response<i64>;
    fn get_connected_nodes(&self) -> Response<Vec<Node>>;
    fn get_peers_with_capabilities(&self)
    -> Response<Vec<(H256, PeerConnection, Vec<Capability>)>>;
    fn get_peer_connections(
        &self,
        capabilities: Vec<Capability>,
    ) -> Response<Vec<(H256, PeerConnection)>>;
    fn insert_if_new(&self, node: Node, protocol: DiscoveryProtocol) -> Response<bool>;
    fn validate_contact(&self, node_id: H256, sender_ip: IpAddr) -> Response<ContactValidation>;
    fn get_closest_nodes(&self, node_id: H256) -> Response<Vec<Node>>;
    fn get_nodes_at_distances(
        &self,
        local_node_id: H256,
        distances: Vec<u32>,
    ) -> Response<Vec<NodeRecord>>;
    fn get_peers_data(&self) -> Response<Vec<PeerData>>;
    fn get_random_peer(
        &self,
        capabilities: Vec<Capability>,
    ) -> Response<Option<(H256, PeerConnection)>>;
    fn get_session_info(&self, node_id: H256) -> Response<Option<Session>>;
}

#[derive(Debug)]
pub struct PeerTableServer {
    contacts: IndexMap<H256, Contact>,
    peers: IndexMap<H256, PeerData>,
    already_tried_peers: FxHashSet<H256>,
    discarded_contacts: FxHashSet<H256>,
    target_peers: usize,
    store: Store,
    /// Standalone session store, independent of contacts.
    /// Allows sessions to be stored even before the contact's ENR is known/parseable.
    sessions: FxHashMap<H256, Session>,
}

#[actor(protocol = PeerTableServerProtocol)]
impl PeerTableServer {
    pub fn spawn(target_peers: usize, store: Store) -> PeerTable {
        PeerTableServer::new(target_peers, store).start()
    }

    pub(crate) fn new(target_peers: usize, store: Store) -> Self {
        Self {
            contacts: Default::default(),
            peers: Default::default(),
            already_tried_peers: Default::default(),
            discarded_contacts: Default::default(),
            target_peers,
            store,
            sessions: Default::default(),
        }
    }

    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        send_message_on(
            ctx.clone(),
            tokio::signal::ctrl_c(),
            peer_table_server_protocol::Shutdown,
        );
    }

    // === Send handlers ===

    #[send_handler]
    async fn handle_new_contacts(
        &mut self,
        msg: peer_table_server_protocol::NewContacts,
        _ctx: &Context<Self>,
    ) {
        self.do_new_contacts(msg.nodes, msg.local_node_id, msg.protocol)
            .await;
    }

    #[send_handler]
    async fn handle_new_contact_records(
        &mut self,
        msg: peer_table_server_protocol::NewContactRecords,
        _ctx: &Context<Self>,
    ) {
        self.do_new_contact_records(msg.node_records, msg.local_node_id)
            .await;
    }

    #[send_handler]
    async fn handle_new_connected_peer(
        &mut self,
        msg: peer_table_server_protocol::NewConnectedPeer,
        _ctx: &Context<Self>,
    ) {
        let new_peer_id = msg.node.node_id();
        let new_peer = PeerData::new(msg.node, None, Some(msg.connection), msg.capabilities);
        self.peers.insert(new_peer_id, new_peer);
    }

    #[send_handler]
    async fn handle_set_session_info(
        &mut self,
        msg: peer_table_server_protocol::SetSessionInfo,
        _ctx: &Context<Self>,
    ) {
        // Store in the standalone sessions map (always succeeds, no contact required).
        self.sessions.insert(msg.node_id, msg.session.clone());
        // Also update the contact's cached session if the contact exists.
        if let Some(contact) = self.contacts.get_mut(&msg.node_id) {
            contact.session = Some(msg.session);
        }
    }

    #[send_handler]
    async fn handle_remove_peer(
        &mut self,
        msg: peer_table_server_protocol::RemovePeer,
        _ctx: &Context<Self>,
    ) {
        self.peers.swap_remove(&msg.node_id);
    }

    #[send_handler]
    async fn handle_inc_requests(
        &mut self,
        msg: peer_table_server_protocol::IncRequests,
        _ctx: &Context<Self>,
    ) {
        self.peers
            .entry(msg.node_id)
            .and_modify(|peer_data| peer_data.requests += 1);
    }

    #[send_handler]
    async fn handle_dec_requests(
        &mut self,
        msg: peer_table_server_protocol::DecRequests,
        _ctx: &Context<Self>,
    ) {
        self.peers
            .entry(msg.node_id)
            .and_modify(|peer_data| peer_data.requests = peer_data.requests.saturating_sub(1));
    }

    #[send_handler]
    async fn handle_set_unwanted(
        &mut self,
        msg: peer_table_server_protocol::SetUnwanted,
        _ctx: &Context<Self>,
    ) {
        self.contacts
            .entry(msg.node_id)
            .and_modify(|contact| contact.unwanted = true);
    }

    #[send_handler]
    async fn handle_set_is_fork_id_valid(
        &mut self,
        msg: peer_table_server_protocol::SetIsForkIdValid,
        _ctx: &Context<Self>,
    ) {
        self.contacts
            .entry(msg.node_id)
            .and_modify(|contact| contact.is_fork_id_valid = Some(msg.valid));
    }

    #[send_handler]
    async fn handle_record_success(
        &mut self,
        msg: peer_table_server_protocol::RecordSuccess,
        _ctx: &Context<Self>,
    ) {
        self.peers
            .entry(msg.node_id)
            .and_modify(|peer_data| peer_data.score = (peer_data.score + 1).min(MAX_SCORE));
    }

    #[send_handler]
    async fn handle_record_failure(
        &mut self,
        msg: peer_table_server_protocol::RecordFailure,
        _ctx: &Context<Self>,
    ) {
        self.peers
            .entry(msg.node_id)
            .and_modify(|peer_data| peer_data.score = (peer_data.score - 1).max(MIN_SCORE));
    }

    #[send_handler]
    async fn handle_record_critical_failure(
        &mut self,
        msg: peer_table_server_protocol::RecordCriticalFailure,
        _ctx: &Context<Self>,
    ) {
        self.peers
            .entry(msg.node_id)
            .and_modify(|peer_data| peer_data.score = MIN_SCORE_CRITICAL);
    }

    #[send_handler]
    async fn handle_record_ping_sent(
        &mut self,
        msg: peer_table_server_protocol::RecordPingSent,
        _ctx: &Context<Self>,
    ) {
        self.contacts
            .entry(msg.node_id)
            .and_modify(|contact| contact.record_ping_sent(msg.ping_id));
    }

    #[send_handler]
    async fn handle_record_pong_received(
        &mut self,
        msg: peer_table_server_protocol::RecordPongReceived,
        _ctx: &Context<Self>,
    ) {
        self.contacts.entry(msg.node_id).and_modify(|contact| {
            if contact
                .ping_id
                .as_ref()
                .map(|value| *value == msg.ping_id)
                .unwrap_or(false)
            {
                contact.ping_id = None
            }
        });
    }

    #[send_handler]
    async fn handle_record_enr_request_sent(
        &mut self,
        msg: peer_table_server_protocol::RecordEnrRequestSent,
        _ctx: &Context<Self>,
    ) {
        self.contacts
            .entry(msg.node_id)
            .and_modify(|contact| contact.record_enr_request_sent(msg.request_hash));
    }

    #[send_handler]
    async fn handle_record_enr_response_received(
        &mut self,
        msg: peer_table_server_protocol::RecordEnrResponseReceived,
        _ctx: &Context<Self>,
    ) {
        self.contacts.entry(msg.node_id).and_modify(|contact| {
            contact.record_enr_response_received(msg.request_hash, msg.record);
        });
    }

    #[send_handler]
    async fn handle_set_disposable(
        &mut self,
        msg: peer_table_server_protocol::SetDisposable,
        _ctx: &Context<Self>,
    ) {
        self.contacts
            .entry(msg.node_id)
            .and_modify(|contact| contact.disposable = true);
    }

    #[send_handler]
    async fn handle_increment_find_node_sent(
        &mut self,
        msg: peer_table_server_protocol::IncrementFindNodeSent,
        _ctx: &Context<Self>,
    ) {
        self.contacts
            .entry(msg.node_id)
            .and_modify(|contact| contact.n_find_node_sent += 1);
    }

    #[send_handler]
    async fn handle_mark_knows_us(
        &mut self,
        msg: peer_table_server_protocol::MarkKnowsUs,
        _ctx: &Context<Self>,
    ) {
        self.contacts
            .entry(msg.node_id)
            .and_modify(|c| c.knows_us = true);
    }

    #[send_handler]
    async fn handle_prune_table(
        &mut self,
        _msg: peer_table_server_protocol::PruneTable,
        _ctx: &Context<Self>,
    ) {
        self.prune();
    }

    #[send_handler]
    async fn handle_shutdown(
        &mut self,
        _msg: peer_table_server_protocol::Shutdown,
        ctx: &Context<Self>,
    ) {
        ctx.stop();
    }

    // === Request handlers ===

    #[request_handler]
    async fn handle_peer_count(
        &mut self,
        _msg: peer_table_server_protocol::PeerCount,
        _ctx: &Context<Self>,
    ) -> usize {
        self.peers.len()
    }

    #[request_handler]
    async fn handle_peer_count_by_capabilities(
        &mut self,
        msg: peer_table_server_protocol::PeerCountByCapabilities,
        _ctx: &Context<Self>,
    ) -> usize {
        self.do_peer_count_by_capabilities(msg.capabilities)
    }

    #[request_handler]
    async fn handle_target_reached(
        &mut self,
        _msg: peer_table_server_protocol::TargetReached,
        _ctx: &Context<Self>,
    ) -> bool {
        self.contacts.len() >= TARGET_CONTACTS && self.peers.len() >= self.target_peers
    }

    #[request_handler]
    async fn handle_target_peers_reached(
        &mut self,
        _msg: peer_table_server_protocol::TargetPeersReached,
        _ctx: &Context<Self>,
    ) -> bool {
        self.peers.len() >= self.target_peers
    }

    #[request_handler]
    async fn handle_target_peers_completion(
        &mut self,
        _msg: peer_table_server_protocol::TargetPeersCompletion,
        _ctx: &Context<Self>,
    ) -> f64 {
        self.peers.len() as f64 / self.target_peers as f64
    }

    #[request_handler]
    async fn handle_get_contact_to_initiate(
        &mut self,
        _msg: peer_table_server_protocol::GetContactToInitiate,
        _ctx: &Context<Self>,
    ) -> Option<Box<Contact>> {
        self.do_get_contact_to_initiate().map(Box::new)
    }

    #[request_handler]
    async fn handle_get_contact_for_lookup(
        &mut self,
        msg: peer_table_server_protocol::GetContactForLookup,
        _ctx: &Context<Self>,
    ) -> Option<Box<Contact>> {
        self.do_get_contact_for_lookup(msg.protocol).map(Box::new)
    }

    #[request_handler]
    async fn handle_get_contact_for_enr_lookup(
        &mut self,
        _msg: peer_table_server_protocol::GetContactForEnrLookup,
        _ctx: &Context<Self>,
    ) -> Option<Box<Contact>> {
        self.do_get_contact_for_enr_lookup().map(Box::new)
    }

    #[request_handler]
    async fn handle_get_contact(
        &mut self,
        msg: peer_table_server_protocol::GetContact,
        _ctx: &Context<Self>,
    ) -> Option<Box<Contact>> {
        self.contacts.get(&msg.node_id).cloned().map(Box::new)
    }

    #[request_handler]
    async fn handle_get_contact_to_revalidate(
        &mut self,
        msg: peer_table_server_protocol::GetContactToRevalidate,
        _ctx: &Context<Self>,
    ) -> Option<Box<Contact>> {
        self.do_get_contact_to_revalidate(msg.revalidation_interval, msg.protocol)
    }

    #[request_handler]
    async fn handle_get_best_peer(
        &mut self,
        msg: peer_table_server_protocol::GetBestPeer,
        _ctx: &Context<Self>,
    ) -> Option<(H256, PeerConnection)> {
        self.do_get_best_peer(&msg.capabilities)
    }

    #[request_handler]
    async fn handle_get_score(
        &mut self,
        msg: peer_table_server_protocol::GetScore,
        _ctx: &Context<Self>,
    ) -> i64 {
        self.peers
            .get(&msg.node_id)
            .map(|peer_data| peer_data.score)
            .unwrap_or_default()
    }

    #[request_handler]
    async fn handle_get_connected_nodes(
        &mut self,
        _msg: peer_table_server_protocol::GetConnectedNodes,
        _ctx: &Context<Self>,
    ) -> Vec<Node> {
        self.peers
            .values()
            .map(|peer_data| peer_data.node.clone())
            .collect()
    }

    #[request_handler]
    async fn handle_get_peers_with_capabilities(
        &mut self,
        _msg: peer_table_server_protocol::GetPeersWithCapabilities,
        _ctx: &Context<Self>,
    ) -> Vec<(H256, PeerConnection, Vec<Capability>)> {
        self.peers
            .iter()
            .filter_map(|(peer_id, peer_data)| {
                peer_data.connection.clone().map(|connection| {
                    (
                        *peer_id,
                        connection,
                        peer_data.supported_capabilities.clone(),
                    )
                })
            })
            .collect()
    }

    #[request_handler]
    async fn handle_get_peer_connections(
        &mut self,
        msg: peer_table_server_protocol::GetPeerConnections,
        _ctx: &Context<Self>,
    ) -> Vec<(H256, PeerConnection)> {
        self.do_get_peer_connections(msg.capabilities)
    }

    #[request_handler]
    async fn handle_insert_if_new(
        &mut self,
        msg: peer_table_server_protocol::InsertIfNew,
        _ctx: &Context<Self>,
    ) -> bool {
        match self.contacts.entry(msg.node.node_id()) {
            Entry::Occupied(_) => false,
            Entry::Vacant(entry) => {
                METRICS.record_new_discovery().await;
                entry.insert(Contact::new(msg.node, msg.protocol));
                true
            }
        }
    }

    #[request_handler]
    async fn handle_validate_contact(
        &mut self,
        msg: peer_table_server_protocol::ValidateContact,
        _ctx: &Context<Self>,
    ) -> ContactValidation {
        self.do_validate_contact(msg.node_id, msg.sender_ip)
    }

    #[request_handler]
    async fn handle_get_closest_nodes(
        &mut self,
        msg: peer_table_server_protocol::GetClosestNodes,
        _ctx: &Context<Self>,
    ) -> Vec<Node> {
        self.do_get_closest_nodes(msg.node_id)
    }

    #[request_handler]
    async fn handle_get_nodes_at_distances(
        &mut self,
        msg: peer_table_server_protocol::GetNodesAtDistances,
        _ctx: &Context<Self>,
    ) -> Vec<NodeRecord> {
        self.do_get_nodes_at_distances(msg.local_node_id, &msg.distances)
    }

    #[request_handler]
    async fn handle_get_peers_data(
        &mut self,
        _msg: peer_table_server_protocol::GetPeersData,
        _ctx: &Context<Self>,
    ) -> Vec<PeerData> {
        self.peers.values().cloned().collect()
    }

    #[request_handler]
    async fn handle_get_random_peer(
        &mut self,
        msg: peer_table_server_protocol::GetRandomPeer,
        _ctx: &Context<Self>,
    ) -> Option<(H256, PeerConnection)> {
        self.do_get_random_peer(msg.capabilities)
    }

    #[request_handler]
    async fn handle_get_session_info(
        &mut self,
        msg: peer_table_server_protocol::GetSessionInfo,
        _ctx: &Context<Self>,
    ) -> Option<Session> {
        // Check standalone sessions map first; fall back to contact.session.
        self.sessions
            .get(&msg.node_id)
            .cloned()
            .or_else(|| self.contacts.get(&msg.node_id)?.session.clone())
    }

    // === Private helper methods ===

    // Weighting function used to select best peer
    fn weight_peer(&self, score: &i64, requests: &i64) -> i64 {
        score * SCORE_WEIGHT - requests * REQUESTS_WEIGHT
    }

    // Returns if the peer has room for more connections given the current score
    // and amount of inflight requests
    fn can_try_more_requests(&self, score: &i64, requests: &i64) -> bool {
        let score_ratio = (score - MIN_SCORE) as f64 / (MAX_SCORE - MIN_SCORE) as f64;
        let max_requests = (MAX_CONCURRENT_REQUESTS_PER_PEER as f64 * score_ratio).max(1.0);
        (*requests as f64) < max_requests
    }

    fn do_get_best_peer(&self, capabilities: &[Capability]) -> Option<(H256, PeerConnection)> {
        self.peers
            .iter()
            .filter_map(|(id, peer_data)| {
                if !self.can_try_more_requests(&peer_data.score, &peer_data.requests)
                    || !capabilities
                        .iter()
                        .any(|cap| peer_data.supported_capabilities.contains(cap))
                {
                    None
                } else {
                    let connection = peer_data.connection.clone()?;
                    Some((*id, peer_data.score, peer_data.requests, connection))
                }
            })
            .max_by_key(|(_, score, reqs, _)| self.weight_peer(score, reqs))
            .map(|(k, _, _, v)| (k, v))
    }

    fn prune(&mut self) {
        let disposable_contacts = self
            .contacts
            .iter()
            .filter_map(|(c_id, c)| c.disposable.then_some(*c_id))
            .collect::<Vec<_>>();

        for contact_to_discard_id in disposable_contacts {
            self.contacts.swap_remove(&contact_to_discard_id);
            self.discarded_contacts.insert(contact_to_discard_id);
        }
    }

    fn do_get_contact_to_initiate(&mut self) -> Option<Contact> {
        for contact in self.contacts.values() {
            let node_id = contact.node.node_id();
            if !self.peers.contains_key(&node_id)
                && !self.already_tried_peers.contains(&node_id)
                && contact.knows_us
                && !contact.unwanted
                && contact.is_fork_id_valid != Some(false)
            {
                self.already_tried_peers.insert(node_id);
                return Some(contact.clone());
            }
        }
        tracing::trace!("Resetting list of tried peers.");
        self.already_tried_peers.clear();
        None
    }

    fn do_get_contact_for_lookup(&self, protocol: DiscoveryProtocol) -> Option<Contact> {
        self.contacts
            .values()
            .filter(|c| {
                c.supports_protocol(protocol)
                    && c.n_find_node_sent < MAX_FIND_NODE_PER_PEER
                    && !c.disposable
            })
            .collect::<Vec<_>>()
            .choose(&mut rand::rngs::OsRng)
            .cloned()
            .cloned()
    }

    /// Get contact for ENR lookup (discv4 only)
    fn do_get_contact_for_enr_lookup(&mut self) -> Option<Contact> {
        self.contacts
            .values()
            .filter(|c| {
                c.is_discv4
                    && c.was_validated()
                    && !c.has_pending_enr_request()
                    && c.record.is_none()
                    && !c.disposable
            })
            .collect::<Vec<_>>()
            .choose(&mut rand::rngs::OsRng)
            .cloned()
            .cloned()
    }

    fn do_get_contact_to_revalidate(
        &self,
        revalidation_interval: Duration,
        protocol: DiscoveryProtocol,
    ) -> Option<Box<Contact>> {
        self.contacts
            .values()
            .filter(|c| {
                c.supports_protocol(protocol)
                    && Self::is_validation_needed(c, revalidation_interval)
            })
            .choose(&mut rand::rngs::OsRng)
            .cloned()
            .map(Box::new)
    }

    fn do_validate_contact(&self, node_id: H256, sender_ip: IpAddr) -> ContactValidation {
        let Some(contact) = self.contacts.get(&node_id) else {
            return ContactValidation::UnknownContact;
        };
        if !contact.was_validated() {
            return ContactValidation::InvalidContact;
        }

        // Check that the IP address from which we receive the request matches the one we have stored
        // to prevent amplification attacks.
        if sender_ip != contact.node.ip {
            return ContactValidation::IpMismatch;
        }
        ContactValidation::Valid(Box::new(contact.clone()))
    }

    /// Get closest nodes for discv4 (returns Vec<Node>)
    fn do_get_closest_nodes(&self, node_id: H256) -> Vec<Node> {
        let mut nodes: Vec<(Node, usize)> = vec![];

        for (contact_id, contact) in &self.contacts {
            let dist = Self::distance(&node_id, contact_id);
            if nodes.len() < MAX_NODES_IN_NEIGHBORS_PACKET {
                nodes.push((contact.node.clone(), dist));
            } else {
                for (i, (_, d)) in &mut nodes.iter().enumerate() {
                    if dist < *d {
                        nodes[i] = (contact.node.clone(), dist);
                        break;
                    }
                }
            }
        }
        nodes.into_iter().map(|(node, _)| node).collect()
    }

    /// Get nodes at distances for discv5 (returns Vec<NodeRecord>).
    /// Uses the discv5 spec log-distance: `floor(log2(XOR))` for non-zero XOR.
    /// Distance 0 is reserved for the local node itself (handled by the caller),
    /// so contacts start at distance >= 1.
    fn do_get_nodes_at_distances(&self, local_node_id: H256, distances: &[u32]) -> Vec<NodeRecord> {
        self.contacts
            .iter()
            .filter_map(|(contact_id, contact)| {
                let dist = distance(&local_node_id, contact_id) as u32;
                if distances.contains(&dist) {
                    contact.record.clone()
                } else {
                    None
                }
            })
            .take(MAX_ENRS_PER_FINDNODE_RESPONSE)
            .collect()
    }

    async fn do_new_contacts(
        &mut self,
        nodes: Vec<Node>,
        local_node_id: H256,
        protocol: DiscoveryProtocol,
    ) {
        for node in nodes {
            let node_id = node.node_id();
            if self.discarded_contacts.contains(&node_id) || node_id == local_node_id {
                continue;
            }
            match self.contacts.entry(node_id) {
                Entry::Vacant(vacant_entry) => {
                    vacant_entry.insert(Contact::new(node, protocol));
                    METRICS.record_new_discovery().await;
                }
                Entry::Occupied(mut occupied_entry) => {
                    // Contact already exists, just add the protocol
                    occupied_entry.get_mut().add_protocol(protocol);
                }
            }
        }
    }

    async fn do_new_contact_records(&mut self, node_records: Vec<NodeRecord>, local_node_id: H256) {
        for node_record in node_records {
            if !node_record.verify_signature() {
                continue;
            }
            if let Ok(node) = Node::from_enr(&node_record) {
                let node_id = node.node_id();
                if self.discarded_contacts.contains(&node_id) || node_id == local_node_id {
                    continue;
                }
                match self.contacts.entry(node_id) {
                    Entry::Vacant(vacant_entry) => {
                        let is_fork_id_valid =
                            Self::evaluate_fork_id(&node_record, &self.store).await;
                        let mut contact = Contact::new(node, DiscoveryProtocol::Discv5);
                        contact.is_fork_id_valid = is_fork_id_valid;
                        contact.record = Some(node_record);
                        vacant_entry.insert(contact);
                        METRICS.record_new_discovery().await;
                    }
                    Entry::Occupied(mut occupied_entry) => {
                        let should_update = match occupied_entry.get().record.as_ref() {
                            None => true,
                            Some(r) => node_record.seq > r.seq,
                        };
                        let contact = occupied_entry.get_mut();
                        contact.add_protocol(DiscoveryProtocol::Discv5);
                        if should_update {
                            let is_fork_id_valid =
                                Self::evaluate_fork_id(&node_record, &self.store).await;
                            if contact.node.ip != node.ip || contact.node.udp_port != node.udp_port
                            {
                                contact.validation_timestamp = None;
                                contact.ping_id = None;
                            }
                            contact.node = node;
                            contact.record = Some(node_record);
                            contact.is_fork_id_valid = is_fork_id_valid;
                        }
                    }
                }
            }
        }
    }

    async fn evaluate_fork_id(record: &NodeRecord, store: &Store) -> Option<bool> {
        if let Some(remote_fork_id) = record.get_fork_id() {
            backend::is_fork_id_valid(store, remote_fork_id)
                .await
                .ok()
                .or(Some(false))
        } else {
            Some(false)
        }
    }

    fn do_peer_count_by_capabilities(&self, capabilities: Vec<Capability>) -> usize {
        self.peers
            .values()
            .filter(|peer_data| {
                capabilities
                    .iter()
                    .any(|cap| peer_data.supported_capabilities.contains(cap))
            })
            .count()
    }

    fn do_get_peer_connections(
        &self,
        capabilities: Vec<Capability>,
    ) -> Vec<(H256, PeerConnection)> {
        self.peers
            .iter()
            .filter_map(|(peer_id, peer_data)| {
                if !capabilities
                    .iter()
                    .any(|cap| peer_data.supported_capabilities.contains(cap))
                {
                    return None;
                }
                peer_data
                    .connection
                    .clone()
                    .map(|connection| (*peer_id, connection))
            })
            .collect()
    }

    fn do_get_random_peer(&self, capabilities: Vec<Capability>) -> Option<(H256, PeerConnection)> {
        let peers: Vec<(H256, PeerConnection)> = self
            .peers
            .iter()
            .filter_map(|(node_id, peer_data)| {
                if !capabilities
                    .iter()
                    .any(|cap| peer_data.supported_capabilities.contains(cap))
                {
                    return None;
                }
                peer_data
                    .connection
                    .clone()
                    .map(|connection| (*node_id, connection))
            })
            .collect();
        peers.choose(&mut rand::rngs::OsRng).cloned()
    }

    fn distance(node_id_1: &H256, node_id_2: &H256) -> usize {
        distance(node_id_1, node_id_2)
    }

    fn is_validation_needed(contact: &Contact, revalidation_interval: Duration) -> bool {
        if contact.disposable {
            return false;
        }

        let sent_ping_ttl = Duration::from_secs(30);

        if contact.has_pending_ping() {
            // Outstanding ping — only re-ping if it timed out (stale).
            contact
                .validation_timestamp
                .map(|ts| Instant::now().saturating_duration_since(ts) > sent_ping_ttl)
                .unwrap_or(false)
        } else {
            // No pending ping — check if never validated or validation expired.
            !contact.was_validated()
                || contact
                    .validation_timestamp
                    .map(|ts| Instant::now().saturating_duration_since(ts) > revalidation_interval)
                    .unwrap_or(false)
        }
    }
}

pub type PeerTable = ActorRef<PeerTableServer>;
