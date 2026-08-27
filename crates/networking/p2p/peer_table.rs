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
    metrics::METRICS,
    peer_filter::{EthForkIdFilter, PeerFilter},
    rlpx::{connection::server::PeerConnection, p2p::Capability},
    types::{Node, NodeRecord},
    utils::distance,
};
use bytes::Bytes;
use ethrex_common::{H256, U256};
use ethrex_storage::Store;
use indexmap::IndexMap;
use rand::distributions::WeightedIndex;
use rand::prelude::Distribution;
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
/// Score weight for the load balancing function.
const SCORE_WEIGHT: i64 = 1;
/// Weight for amount of requests being handled by the peer for the load balancing function.
const REQUESTS_WEIGHT: i64 = 1;
/// Max amount of ongoing requests per peer.
const MAX_CONCURRENT_REQUESTS_PER_PEER: i64 = 100;
/// The target number of RLPx connections to reach.
pub const TARGET_PEERS: usize = 100;
/// Maximum number of ENRs to return in a FindNode response (discv4 compatible).
pub(crate) const MAX_NODES_IN_NEIGHBORS_PACKET: usize = 16;
/// Maximum number of ENRs to return in a discv5 FindNode response.
const MAX_ENRS_PER_FINDNODE_RESPONSE: usize = 16;

/// Number of k-buckets in the Kademlia routing table (one per bit of the 256-bit node ID).
const NUMBER_OF_BUCKETS: usize = 256;
/// Maximum number of contacts per k-bucket (Kademlia k parameter).
pub const MAX_NODES_PER_BUCKET: usize = 16;
/// Maximum number of replacement entries per k-bucket.
const MAX_REPLACEMENTS_PER_BUCKET: usize = 10;
/// Maximum number of entries in the flat connection candidate pool.
/// This pool is separate from the k-bucket routing table and retains
/// more contacts for RLPx connection initiation than the k-bucket
/// structure allows (256 × 16 = 4,096 vs this larger capacity).
/// 10K matches what Reth and Nethermind use for their candidate pools.
const MAX_CONNECTION_POOL_SIZE: usize = 10_000;

/// A single k-bucket in the Kademlia routing table.
/// Each bucket stores contacts at a specific XOR distance range from the local node.
#[derive(Debug, Clone, Default)]
pub struct KBucket {
    pub(crate) contacts: Vec<(H256, Contact)>,
    pub(crate) replacements: Vec<(H256, Contact)>,
}

impl KBucket {
    /// Find a contact by node ID in the main list.
    fn get(&self, node_id: &H256) -> Option<&Contact> {
        self.contacts
            .iter()
            .find(|(id, _)| id == node_id)
            .map(|(_, c)| c)
    }

    /// Find a contact by node ID in either the main or replacement list.
    fn get_any(&self, node_id: &H256) -> Option<&Contact> {
        self.get(node_id).or_else(|| {
            self.replacements
                .iter()
                .find(|(id, _)| id == node_id)
                .map(|(_, c)| c)
        })
    }

    /// Find a mutable reference to a contact by node ID (main or replacement list).
    fn get_mut(&mut self, node_id: &H256) -> Option<&mut Contact> {
        if let Some((_, c)) = self.contacts.iter_mut().find(|(id, _)| id == node_id) {
            return Some(c);
        }
        self.replacements
            .iter_mut()
            .find(|(id, _)| id == node_id)
            .map(|(_, c)| c)
    }

    /// Check if a contact exists in this bucket (main or replacement list).
    fn contains(&self, node_id: &H256) -> bool {
        self.contacts.iter().any(|(id, _)| id == node_id)
            || self.replacements.iter().any(|(id, _)| id == node_id)
    }

    /// Insert a contact into the bucket. Returns true if inserted into main list.
    /// If the bucket is full, the contact is added to the replacement list instead.
    fn insert(&mut self, node_id: H256, contact: Contact) -> bool {
        if self.contacts.len() < MAX_NODES_PER_BUCKET {
            self.contacts.push((node_id, contact));
            true
        } else {
            self.insert_replacement(node_id, contact);
            false
        }
    }

    /// Add a contact to the replacement list, evicting the oldest if full.
    fn insert_replacement(&mut self, node_id: H256, contact: Contact) {
        if self.replacements.len() >= MAX_REPLACEMENTS_PER_BUCKET {
            self.replacements.remove(0);
        }
        self.replacements.push((node_id, contact));
    }

    /// Remove a contact from the main list and promote a replacement if available.
    /// Returns the promoted replacement's node ID, if any.
    fn remove_and_promote(&mut self, node_id: &H256) -> Option<H256> {
        let idx = self.contacts.iter().position(|(id, _)| id == node_id)?;
        self.contacts.remove(idx);
        if !self.replacements.is_empty() {
            let (replacement_id, replacement) = self.replacements.remove(0);
            self.contacts.push((replacement_id, replacement));
            Some(replacement_id)
        } else {
            None
        }
    }
}

/// Computes the bucket index for a node relative to the local node.
/// Uses XOR distance: bucket = floor(log2(XOR(local, remote))), i.e. the
/// position of the highest set bit minus 1.
/// Returns None for the local node itself (XOR = 0).
fn bucket_index(local_node_id: &H256, node_id: &H256) -> Option<usize> {
    let xor = *local_node_id ^ *node_id;
    let dist = U256::from_big_endian(xor.as_bytes());
    if dist.is_zero() {
        None
    } else {
        Some(dist.bits() - 1)
    }
}

/// Computes the raw XOR distance between two node IDs.
/// Used for comparing relative closeness: a is closer to target than b
/// iff xor_distance(target, a) < xor_distance(target, b).
pub(crate) fn xor_distance(a: &H256, b: &H256) -> H256 {
    *a ^ *b
}

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

    /// ENR associated with this contact, if it was provided by the peer.
    pub record: Option<NodeRecord>,
    /// This contact failed to respond our Ping.
    pub disposable: bool,
    /// Set to true after we send a successful ENRResponse to it.
    pub knows_us: bool,
    /// This is a known-bad peer (on another network, no matching capabilities, etc)
    pub unwanted: bool,
    /// Whether this contact's last known ENR made it through the consumer's
    /// [`PeerFilter`], or `None` while it has never been filtered.
    ///
    /// Unfiltered stays dialable: a contact discovered without an ENR never
    /// reaches the filter at all, and treating that as a rejection would leave
    /// it permanently untriable. That is why bootnodes, which arrive as bare
    /// endpoints, are dialable before they have published anything.
    pub passes_filter: Option<bool>,
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

    /// Stores `record` if it answers the ENR request we have outstanding.
    ///
    /// Returns whether it was stored, so the caller knows whether the contact's
    /// cached [`Self::passes_filter`] still describes the record it holds. A
    /// response whose hash does not match is ignored outright: letting it
    /// through would let a peer restate its own standing from a record we
    /// refused to keep.
    pub fn record_enr_response_received(&mut self, request_hash: H256, record: NodeRecord) -> bool {
        if self
            .enr_request_hash
            .take_if(|h| *h == request_hash)
            .is_some()
        {
            self.record = Some(record);
            return true;
        }
        false
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
            record: None,
            disposable: false,
            knows_us: true,
            unwanted: false,
            passes_filter: None,
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
    /// Timestamp (seconds since UNIX epoch) of the last successful response from this peer
    pub last_response_time: Option<u64>,
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
            last_response_time: None,
        }
    }
}

/// Diagnostic snapshot of a peer's state, used by admin RPC endpoints.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PeerDiagnostics {
    pub peer_id: H256,
    pub score: i64,
    pub inflight_requests: i64,
    pub eligible: bool,
    pub capabilities: Vec<String>,
    pub ip: IpAddr,
    pub client_version: String,
    pub connection_direction: String,
    pub last_response_time: Option<u64>,
}

/// Result of contact validation.
#[derive(Debug, Clone)]
pub enum ContactValidation {
    Valid(Box<Contact>),
    InvalidContact,
    UnknownContact,
    IpMismatch,
}

/// Reservation handle for a peer request slot.
///
/// **Contract:** when a `RequestPermit` exists, the `requests` counter for
/// its peer has been incremented by one. Dropping the permit releases the
/// slot via a fire-and-forget `DecRequests` message. The handler that
/// returns the permit also bumps the counter atomically under `&mut self`,
/// so selection and reservation cannot be observed out of order.
///
/// The permit must travel with whatever code owns the outstanding request —
/// move it into spawned tasks, send it through channels alongside results,
/// etc. Dropping early releases the slot early.
#[must_use = "dropping this permit immediately releases the peer's request slot"]
pub struct RequestPermit {
    peer_table: PeerTable,
    peer_id: H256,
}

/// A peer picked for one request: its node id, a live connection, the permit
/// holding its request slot, and the capabilities it advertised — the last so
/// callers can derive the negotiated protocol version where the wire format
/// depends on it (receipts differ between eth/69 and eth/70).
pub type SelectedPeer = (H256, PeerConnection, RequestPermit, Vec<Capability>);

impl RequestPermit {
    pub(crate) fn new(peer_table: PeerTable, peer_id: H256) -> Self {
        Self {
            peer_table,
            peer_id,
        }
    }
}

impl std::fmt::Debug for RequestPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestPermit")
            .field("peer_id", &self.peer_id)
            .finish_non_exhaustive()
    }
}

impl Drop for RequestPermit {
    fn drop(&mut self) {
        // Fire-and-forget. If the actor mailbox is closed, p2p is already
        // shutting down — the lost decrement is a non-issue.
        let _ = self.peer_table.dec_requests(self.peer_id);
    }
}

#[protocol]
pub trait PeerTableServerProtocol: Send + Sync {
    // Send (cast) methods
    fn new_contacts(&self, nodes: Vec<Node>, protocol: DiscoveryProtocol)
    -> Result<(), ActorError>;
    fn new_contact_records(&self, node_records: Vec<NodeRecord>) -> Result<(), ActorError>;
    fn new_connected_peer(
        &self,
        node: Node,
        connection: PeerConnection,
        capabilities: Vec<Capability>,
        is_inbound: bool,
    ) -> Result<(), ActorError>;
    fn set_session_info(&self, node_id: H256, session: Session) -> Result<(), ActorError>;
    fn remove_peer(&self, node_id: H256) -> Result<(), ActorError>;
    fn dec_requests(&self, node_id: H256) -> Result<(), ActorError>;
    fn set_unwanted(&self, node_id: H256) -> Result<(), ActorError>;
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
    fn get_contact_for_enr_lookup(&self) -> Response<Option<Box<Contact>>>;
    fn get_closest_from_pool(&self, target: H256, count: usize) -> Response<Vec<(H256, Node)>>;
    fn get_contact(&self, node_id: H256) -> Response<Option<Box<Contact>>>;
    fn get_contact_to_revalidate(
        &self,
        revalidation_interval: Duration,
        protocol: DiscoveryProtocol,
    ) -> Response<Option<Box<Contact>>>;
    fn get_best_peer(
        &self,
        capabilities: Vec<Capability>,
    ) -> Response<Option<(H256, PeerConnection, RequestPermit)>>;
    fn get_best_peer_excluding(
        &self,
        capabilities: Vec<Capability>,
        excluded: Vec<H256>,
    ) -> Response<Option<(H256, PeerConnection, RequestPermit)>>;
    fn get_best_n_peers(
        &self,
        capabilities: Vec<Capability>,
        n: usize,
    ) -> Response<Vec<(H256, PeerConnection, RequestPermit)>>;
    /// Read-only predicate: is there any eligible peer matching `capabilities`?
    /// Does not reserve a slot; use for capacity/rotation probes only.
    fn has_eligible_peer(&self, capabilities: Vec<Capability>) -> Response<bool>;
    fn get_score(&self, node_id: H256) -> Response<i64>;
    fn get_connected_nodes(&self) -> Response<Vec<Node>>;
    fn get_peers_with_capabilities(&self)
    -> Response<Vec<(H256, PeerConnection, Vec<Capability>)>>;
    fn insert_if_new(&self, node: Node, protocol: DiscoveryProtocol) -> Response<bool>;
    fn validate_contact(&self, node_id: H256, sender_ip: IpAddr) -> Response<ContactValidation>;
    fn get_closest_nodes(&self, node_id: H256) -> Response<Vec<Node>>;
    fn get_nodes_at_distances(&self, distances: Vec<u32>) -> Response<Vec<NodeRecord>>;
    fn get_peers_data(&self) -> Response<Vec<PeerData>>;
    fn get_random_peer(&self, capabilities: Vec<Capability>) -> Response<Option<SelectedPeer>>;
    fn get_session_info(&self, node_id: H256) -> Response<Option<Session>>;
    fn get_peer_diagnostics(&self) -> Response<Vec<PeerDiagnostics>>;
    fn get_peer_connection(&self, peer_id: H256) -> Response<Option<PeerConnection>>;
}

pub struct PeerTableServer {
    local_node_id: H256,
    buckets: Vec<KBucket>,
    peers: IndexMap<H256, PeerData>,
    already_tried_peers: FxHashSet<H256>,
    target_peers: usize,
    /// What this consumer requires of a discovered peer. Judged as each ENR
    /// arrives, over either discovery protocol; the answer is cached on the
    /// contact as [`Contact::passes_filter`].
    filter: Box<dyn PeerFilter>,
    /// Standalone session store, independent of contacts.
    /// Allows sessions to be stored even before the contact's ENR is known/parseable.
    sessions: FxHashMap<H256, Session>,
    /// Flat pool of discovered contacts for RLPx connection initiation.
    /// Decoupled from the k-bucket routing table so that connection initiation
    /// has access to a much larger candidate pool than the k-bucket structure
    /// allows (k-buckets: 256 × 16 = 4,096 max; this pool: up to 50,000).
    /// K-buckets are still used for all Kademlia protocol operations.
    connection_pool: IndexMap<H256, Node>,
}

// Hand-written because `Box<dyn PeerFilter>` is not `Debug`, and requiring that
// of every consumer's filter buys less than keeping the actor's state printable.
impl std::fmt::Debug for PeerTableServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerTableServer")
            .field("local_node_id", &self.local_node_id)
            .field("peers", &self.peers)
            .field("already_tried_peers", &self.already_tried_peers)
            .field("target_peers", &self.target_peers)
            .field("sessions", &self.sessions)
            .field("connection_pool", &self.connection_pool)
            .finish_non_exhaustive()
    }
}

#[actor(protocol = PeerTableServerProtocol)]
impl PeerTableServer {
    /// Spawn a peer table for an Ethereum L1 node, screening every ENR it sees
    /// for an EIP-2124 fork id compatible with our chain.
    ///
    /// A contact discovered without an ENR is never screened and stays dialable,
    /// so bootnodes are usable before they have published anything. See
    /// [`Self::spawn_with_filter`] for consumers on other networks.
    pub fn spawn(local_node_id: H256, target_peers: usize, store: Store) -> PeerTable {
        Self::spawn_with_filter(local_node_id, target_peers, EthForkIdFilter::new(store))
    }

    pub fn spawn_with_filter(
        local_node_id: H256,
        target_peers: usize,
        filter: impl PeerFilter + 'static,
    ) -> PeerTable {
        PeerTableServer::new(local_node_id, target_peers, Box::new(filter)).start()
    }

    pub(crate) fn new(
        local_node_id: H256,
        target_peers: usize,
        filter: Box<dyn PeerFilter>,
    ) -> Self {
        Self {
            local_node_id,
            buckets: vec![KBucket::default(); NUMBER_OF_BUCKETS],
            peers: Default::default(),
            already_tried_peers: Default::default(),
            target_peers,
            filter,
            sessions: Default::default(),
            connection_pool: IndexMap::with_capacity(MAX_CONNECTION_POOL_SIZE),
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
        self.do_new_contacts(msg.nodes, msg.protocol).await;
    }

    #[send_handler]
    async fn handle_new_contact_records(
        &mut self,
        msg: peer_table_server_protocol::NewContactRecords,
        _ctx: &Context<Self>,
    ) {
        self.do_new_contact_records(msg.node_records).await;
    }

    #[send_handler]
    async fn handle_new_connected_peer(
        &mut self,
        msg: peer_table_server_protocol::NewConnectedPeer,
        _ctx: &Context<Self>,
    ) {
        let new_peer_id = msg.node.node_id();
        let mut new_peer = PeerData::new(msg.node, None, Some(msg.connection), msg.capabilities);
        new_peer.is_connection_inbound = msg.is_inbound;
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
        if let Some(contact) = self.get_contact_mut(&msg.node_id) {
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
        // Also drop the standalone discv5 session so it isn't retained after the peer leaves
        // (the sessions map was previously insert-only and grew per handshake).
        self.sessions.remove(&msg.node_id);
    }

    #[send_handler]
    async fn handle_dec_requests(
        &mut self,
        msg: peer_table_server_protocol::DecRequests,
        _ctx: &Context<Self>,
    ) {
        self.peers.entry(msg.node_id).and_modify(|peer_data| {
            if peer_data.requests <= 0 {
                // Expected under the reconnect race (stale permit fires
                // after remove_peer + new_connected_peer), self-heals.
                // Otherwise points to a bookkeeping bug worth chasing.
                tracing::debug!(
                    peer_id = ?msg.node_id,
                    requests = peer_data.requests,
                    "dec_requests with counter already <= 0",
                );
            }
            peer_data.requests = peer_data.requests.saturating_sub(1).max(0)
        });
    }

    #[send_handler]
    async fn handle_set_unwanted(
        &mut self,
        msg: peer_table_server_protocol::SetUnwanted,
        _ctx: &Context<Self>,
    ) {
        if let Some(contact) = self.get_contact_mut(&msg.node_id) {
            contact.unwanted = true;
        }
    }

    #[send_handler]
    async fn handle_record_success(
        &mut self,
        msg: peer_table_server_protocol::RecordSuccess,
        _ctx: &Context<Self>,
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.peers.entry(msg.node_id).and_modify(|peer_data| {
            peer_data.score = (peer_data.score + 1).min(MAX_SCORE);
            peer_data.last_response_time = Some(now);
        });
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
        if let Some(contact) = self.get_contact_mut(&msg.node_id) {
            contact.record_ping_sent(msg.ping_id);
        }
    }

    #[send_handler]
    async fn handle_record_pong_received(
        &mut self,
        msg: peer_table_server_protocol::RecordPongReceived,
        _ctx: &Context<Self>,
    ) {
        if let Some(contact) = self.get_contact_mut(&msg.node_id)
            && contact
                .ping_id
                .as_ref()
                .map(|value| *value == msg.ping_id)
                .unwrap_or(false)
        {
            contact.ping_id = None;
        }
    }

    #[send_handler]
    async fn handle_record_enr_request_sent(
        &mut self,
        msg: peer_table_server_protocol::RecordEnrRequestSent,
        _ctx: &Context<Self>,
    ) {
        self.do_record_enr_request_sent(msg.node_id, msg.request_hash);
    }

    #[send_handler]
    async fn handle_record_enr_response_received(
        &mut self,
        msg: peer_table_server_protocol::RecordEnrResponseReceived,
        _ctx: &Context<Self>,
    ) {
        self.do_record_enr_response_received(msg.node_id, msg.request_hash, msg.record);
    }

    #[send_handler]
    async fn handle_set_disposable(
        &mut self,
        msg: peer_table_server_protocol::SetDisposable,
        _ctx: &Context<Self>,
    ) {
        if let Some(contact) = self.get_contact_mut(&msg.node_id) {
            contact.disposable = true;
        }
    }

    #[send_handler]
    async fn handle_mark_knows_us(
        &mut self,
        msg: peer_table_server_protocol::MarkKnowsUs,
        _ctx: &Context<Self>,
    ) {
        if let Some(contact) = self.get_contact_mut(&msg.node_id) {
            contact.knows_us = true;
        }
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
        self.peers.len() >= self.target_peers
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
    async fn handle_get_closest_from_pool(
        &mut self,
        msg: peer_table_server_protocol::GetClosestFromPool,
        _ctx: &Context<Self>,
    ) -> Vec<(H256, Node)> {
        self.do_get_closest_from_pool(msg.target, msg.count)
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
        self.get_contact(&msg.node_id).cloned().map(Box::new)
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
        ctx: &Context<Self>,
    ) -> Option<(H256, PeerConnection, RequestPermit)> {
        let (peer_id, conn) = self.do_get_best_peer(&msg.capabilities)?;
        self.peers
            .get_mut(&peer_id)
            .expect("peer returned by do_get_best_peer must be present in self.peers")
            .requests += 1;
        Some((peer_id, conn, RequestPermit::new(ctx.actor_ref(), peer_id)))
    }

    #[request_handler]
    async fn handle_get_best_peer_excluding(
        &mut self,
        msg: peer_table_server_protocol::GetBestPeerExcluding,
        ctx: &Context<Self>,
    ) -> Option<(H256, PeerConnection, RequestPermit)> {
        let (peer_id, conn) = self.do_get_best_peer_excluding(&msg.capabilities, &msg.excluded)?;
        self.peers
            .get_mut(&peer_id)
            .expect("peer returned by do_get_best_peer_excluding must be present in self.peers")
            .requests += 1;
        Some((peer_id, conn, RequestPermit::new(ctx.actor_ref(), peer_id)))
    }

    #[request_handler]
    async fn handle_get_best_n_peers(
        &mut self,
        msg: peer_table_server_protocol::GetBestNPeers,
        ctx: &Context<Self>,
    ) -> Vec<(H256, PeerConnection, RequestPermit)> {
        let picks = self.do_get_best_n_peers(&msg.capabilities, msg.n);
        let mut out = Vec::with_capacity(picks.len());
        for (peer_id, conn) in picks {
            self.peers
                .get_mut(&peer_id)
                .expect("peer returned by do_get_best_n_peers must be present in self.peers")
                .requests += 1;
            out.push((peer_id, conn, RequestPermit::new(ctx.actor_ref(), peer_id)));
        }
        out
    }

    #[request_handler]
    async fn handle_has_eligible_peer(
        &mut self,
        msg: peer_table_server_protocol::HasEligiblePeer,
        _ctx: &Context<Self>,
    ) -> bool {
        self.peers.values().any(|peer_data| {
            peer_data.connection.is_some()
                && self.can_try_more_requests(&peer_data.score, &peer_data.requests)
                && msg
                    .capabilities
                    .iter()
                    .any(|cap| peer_data.supported_capabilities.contains(cap))
        })
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
    async fn handle_insert_if_new(
        &mut self,
        msg: peer_table_server_protocol::InsertIfNew,
        _ctx: &Context<Self>,
    ) -> bool {
        let node_id = msg.node.node_id();
        // Always add to the connection pool
        self.insert_to_connection_pool(node_id, msg.node.clone());
        if self.contact_exists(&node_id) {
            return false;
        }
        let contact = Contact::new(msg.node, msg.protocol);
        // Return true for any genuinely new node, even if it overflows to the
        // replacement list.  This ensures the caller sends a reciprocal ping
        // which establishes the bond needed for FindNode validation.
        self.insert_contact(node_id, contact);
        METRICS.record_new_discovery().await;
        true
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
        self.do_get_nodes_at_distances(&msg.distances)
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
        ctx: &Context<Self>,
    ) -> Option<SelectedPeer> {
        let (peer_id, conn, capabilities) = self.do_get_random_peer(msg.capabilities)?;
        self.peers
            .get_mut(&peer_id)
            .expect("peer returned by do_get_random_peer must be present in self.peers")
            .requests += 1;
        Some((
            peer_id,
            conn,
            RequestPermit::new(ctx.actor_ref(), peer_id),
            capabilities,
        ))
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
            .or_else(|| self.get_contact(&msg.node_id)?.session.clone())
    }

    #[request_handler]
    async fn handle_get_peer_connection(
        &mut self,
        msg: peer_table_server_protocol::GetPeerConnection,
        _ctx: &Context<Self>,
    ) -> Option<PeerConnection> {
        self.peers
            .get(&msg.peer_id)
            .and_then(|peer_data| peer_data.connection.clone())
    }

    #[request_handler]
    async fn handle_get_peer_diagnostics(
        &mut self,
        _msg: peer_table_server_protocol::GetPeerDiagnostics,
        _ctx: &Context<Self>,
    ) -> Vec<PeerDiagnostics> {
        self.peers
            .iter()
            .map(|(id, peer_data)| PeerDiagnostics {
                peer_id: *id,
                score: peer_data.score,
                inflight_requests: peer_data.requests,
                eligible: self.can_try_more_requests(&peer_data.score, &peer_data.requests),
                capabilities: peer_data
                    .supported_capabilities
                    .iter()
                    .map(|c| format!("{}/{}", c.protocol(), c.version))
                    .collect(),
                ip: peer_data.node.ip,
                client_version: peer_data.node.version.clone().unwrap_or_default(),
                connection_direction: if peer_data.is_connection_inbound {
                    "inbound".to_string()
                } else {
                    "outbound".to_string()
                },
                last_response_time: peer_data.last_response_time,
            })
            .collect()
    }

    // === Private helper methods ===

    // --- K-bucket accessors ---

    /// Get the bucket index for a node ID, or None if it's the local node.
    fn bucket_for(&self, node_id: &H256) -> Option<usize> {
        bucket_index(&self.local_node_id, node_id)
    }

    /// Look up a contact by node ID in main or replacement list (O(K) within the bucket).
    fn get_contact(&self, node_id: &H256) -> Option<&Contact> {
        let idx = self.bucket_for(node_id)?;
        self.buckets[idx].get_any(node_id)
    }

    /// Look up a mutable reference to a contact by node ID.
    fn get_contact_mut(&mut self, node_id: &H256) -> Option<&mut Contact> {
        let idx = self.bucket_for(node_id)?;
        self.buckets[idx].get_mut(node_id)
    }

    /// Check if a contact exists in any bucket (main or replacement list).
    fn contact_exists(&self, node_id: &H256) -> bool {
        let Some(idx) = self.bucket_for(node_id) else {
            return false;
        };
        self.buckets[idx].contains(node_id)
    }

    /// Insert a contact into the appropriate k-bucket. Returns true if inserted
    /// into the main list, false if the node went to the replacement list or is
    /// the local node.
    fn insert_contact(&mut self, node_id: H256, contact: Contact) -> bool {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();

        let Some(idx) = self.bucket_for(&node_id) else {
            return false;
        };
        let result = self.buckets[idx].insert(node_id, contact);

        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.observe_insert_contact_duration(start.elapsed().as_secs_f64());
        }

        result
    }

    /// Insert a node into the flat connection pool for RLPx initiation.
    /// Evicts the oldest entry when the pool is at capacity.
    fn insert_to_connection_pool(&mut self, node_id: H256, node: Node) {
        if self.connection_pool.contains_key(&node_id) {
            return;
        }
        if self.connection_pool.len() >= MAX_CONNECTION_POOL_SIZE {
            self.connection_pool.shift_remove_index(0);
        }
        self.connection_pool.insert(node_id, node);
    }

    /// Look up a contact by node ID in either the main or replacement list.
    fn get_contact_or_replacement(&self, node_id: &H256) -> Option<&Contact> {
        let idx = self.bucket_for(node_id)?;
        self.buckets[idx].get_any(node_id)
    }

    /// Look up a mutable reference in either the main or replacement list.
    fn get_contact_or_replacement_mut(&mut self, node_id: &H256) -> Option<&mut Contact> {
        let idx = self.bucket_for(node_id)?;
        let bucket = &mut self.buckets[idx];
        // Search main list first, then replacement list.
        // Done inline to avoid borrow-checker issues with or_else closures.
        if let Some(pos) = bucket.contacts.iter().position(|(id, _)| id == node_id) {
            return Some(&mut bucket.contacts[pos].1);
        }
        if let Some(pos) = bucket.replacements.iter().position(|(id, _)| id == node_id) {
            return Some(&mut bucket.replacements[pos].1);
        }
        None
    }

    /// Iterate over all contacts across all buckets (main and replacement lists).
    fn iter_contacts(&self) -> impl Iterator<Item = (&H256, &Contact)> {
        self.buckets.iter().flat_map(|bucket| {
            bucket
                .contacts
                .iter()
                .chain(bucket.replacements.iter())
                .map(|(id, c)| (id, c))
        })
    }

    // --- Peer selection ---

    fn weight_peer(&self, score: &i64, requests: &i64) -> i64 {
        score * SCORE_WEIGHT - requests * REQUESTS_WEIGHT
    }

    fn can_try_more_requests(&self, score: &i64, requests: &i64) -> bool {
        let score_ratio = (score - MIN_SCORE) as f64 / (MAX_SCORE - MIN_SCORE) as f64;
        let max_requests = (MAX_CONCURRENT_REQUESTS_PER_PEER as f64 * score_ratio).max(1.0);
        (*requests as f64) < max_requests
    }

    fn do_get_best_peer(&self, capabilities: &[Capability]) -> Option<(H256, PeerConnection)> {
        self.do_get_best_peer_excluding(capabilities, &[])
    }

    /// Like `do_get_best_peer`, but excludes specific peers from selection.
    /// Used by `update_pivot` to rotate through peers on repeated failures.
    fn do_get_best_peer_excluding(
        &self,
        capabilities: &[Capability],
        excluded: &[H256],
    ) -> Option<(H256, PeerConnection)> {
        self.peers
            .iter()
            .filter_map(|(id, peer_data)| {
                if excluded.contains(id)
                    || !self.can_try_more_requests(&peer_data.score, &peer_data.requests)
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

    /// Returns up to `n` best peers with capability overlap, sorted by weight
    /// descending. Excludes peers at capacity. Does NOT mutate state — caller
    /// is responsible for incrementing `requests` on each returned peer. The
    /// sort uses a pre-increment snapshot: later picks don't see earlier
    /// picks' bumps, which is fine for small `n`.
    fn do_get_best_n_peers(
        &self,
        capabilities: &[Capability],
        n: usize,
    ) -> Vec<(H256, PeerConnection)> {
        let mut candidates: Vec<(H256, i64, i64, PeerConnection)> = self
            .peers
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
            .collect();

        candidates.sort_by_key(|(_, score, reqs, _)| -self.weight_peer(score, reqs));
        candidates
            .into_iter()
            .take(n)
            .map(|(id, _, _, conn)| (id, conn))
            .collect()
    }

    // --- Contact operations ---

    /// Prune disposable contacts from both main and replacement lists.
    /// When a main contact is removed, a replacement is automatically promoted.
    /// Pruned contacts remain in the connection pool so they can be retried
    /// later — the RLPx handshake will reject them if they're truly bad.
    fn prune(&mut self) {
        for bucket in &mut self.buckets {
            // Collect disposable contacts from main list
            let main_disposable: Vec<H256> = bucket
                .contacts
                .iter()
                .filter(|(_, c)| c.disposable)
                .map(|(id, _)| *id)
                .collect();

            // Remove from main list and promote replacements
            for node_id in main_disposable {
                bucket.remove_and_promote(&node_id);
            }

            // Remove disposable contacts from replacement list
            // (these don't get promoted, just removed)
            bucket.replacements.retain(|(_, c)| !c.disposable);
        }
    }

    fn do_get_contact_to_initiate(&mut self) -> Option<Contact> {
        // Draw from the flat connection pool using O(1) random index probing.
        // Pick a random start index and scan forward (wrapping) until we find
        // an eligible candidate or complete a full loop.
        let pool_len = self.connection_pool.len();
        if pool_len == 0 {
            return None;
        }

        let start = rand::random::<usize>() % pool_len;
        for offset in 0..pool_len {
            let idx = (start + offset) % pool_len;
            let Some((node_id, node)) = self.connection_pool.get_index(idx) else {
                continue;
            };
            let node_id = *node_id;

            if self.peers.contains_key(&node_id)
                || self.already_tried_peers.contains(&node_id)
                || self
                    .get_contact_or_replacement(&node_id)
                    .map(|c| !c.knows_us || c.unwanted || c.passes_filter == Some(false))
                    .unwrap_or(false)
            {
                continue;
            }

            let node = node.clone();
            self.already_tried_peers.insert(node_id);
            let contact = self
                .get_contact_or_replacement(&node_id)
                .cloned()
                .unwrap_or_else(|| Contact::new(node, DiscoveryProtocol::Discv4));
            return Some(contact);
        }

        // Exhausted all candidates — reset tried set for next cycle.
        tracing::trace!("Resetting list of tried peers.");
        self.already_tried_peers.clear();
        None
    }

    /// Get the `count` closest nodes from the connection pool, sorted by XOR distance to `target`.
    fn do_get_closest_from_pool(&self, target: H256, count: usize) -> Vec<(H256, Node)> {
        let mut nodes: Vec<(H256, Node, H256)> = Vec::with_capacity(count);

        for (node_id, node) in &self.connection_pool {
            let dist = xor_distance(&target, node_id);
            if nodes.len() < count {
                nodes.push((*node_id, node.clone(), dist));
            } else if let Some((farthest_idx, _)) =
                nodes.iter().enumerate().max_by_key(|(_, (_, _, d))| *d)
                && dist < nodes[farthest_idx].2
            {
                nodes[farthest_idx] = (*node_id, node.clone(), dist);
            }
        }

        nodes.sort_by(|a, b| a.2.cmp(&b.2));
        nodes.into_iter().map(|(id, node, _)| (id, node)).collect()
    }

    /// Get contact for ENR lookup (discv4 only)
    fn do_get_contact_for_enr_lookup(&mut self) -> Option<Contact> {
        self.iter_contacts()
            .filter(|(_, c)| {
                c.is_discv4
                    && c.was_validated()
                    && !c.has_pending_enr_request()
                    && c.record.is_none()
                    && !c.disposable
            })
            .map(|(_, c)| c)
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
        self.iter_contacts()
            .filter(|(_, c)| {
                c.supports_protocol(protocol)
                    && Self::is_validation_needed(c, revalidation_interval)
            })
            .map(|(_, c)| c)
            .choose(&mut rand::rngs::OsRng)
            .cloned()
            .map(Box::new)
    }

    fn do_validate_contact(&self, node_id: H256, sender_ip: IpAddr) -> ContactValidation {
        let Some(contact) = self.get_contact(&node_id) else {
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

    /// Get closest nodes using raw XOR distance for accurate ordering.
    fn do_get_closest_nodes(&self, node_id: H256) -> Vec<Node> {
        #[cfg(feature = "metrics")]
        let scan_start = std::time::Instant::now();

        let mut nodes: Vec<(Node, H256)> = vec![];

        for (contact_id, contact) in self.iter_contacts() {
            let dist = xor_distance(&node_id, contact_id);
            if nodes.len() < MAX_NODES_IN_NEIGHBORS_PACKET {
                nodes.push((contact.node.clone(), dist));
            } else if let Some((farthest_idx, _)) =
                nodes.iter().enumerate().max_by_key(|(_, (_, d))| *d)
                && dist < nodes[farthest_idx].1
            {
                nodes[farthest_idx] = (contact.node.clone(), dist);
            }
        }

        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.observe_iter_contacts_duration(scan_start.elapsed().as_secs_f64());
        }

        nodes.into_iter().map(|(node, _)| node).collect()
    }

    /// Get nodes at distances for discv5 (returns Vec<NodeRecord>).
    /// Uses the discv5 spec log-distance: `floor(log2(XOR))` for non-zero XOR.
    /// Distance 0 is reserved for the local node itself (handled by the caller),
    /// so contacts start at distance >= 1.
    fn do_get_nodes_at_distances(&self, distances: &[u32]) -> Vec<NodeRecord> {
        self.iter_contacts()
            .filter_map(|(contact_id, contact)| {
                let dist = distance(&self.local_node_id, contact_id) as u32;
                if distances.contains(&dist) {
                    contact.record.clone()
                } else {
                    None
                }
            })
            .take(MAX_ENRS_PER_FINDNODE_RESPONSE)
            .collect()
    }

    async fn do_new_contacts(&mut self, nodes: Vec<Node>, protocol: DiscoveryProtocol) {
        for node in nodes {
            let node_id = node.node_id();
            if node_id == self.local_node_id {
                continue;
            }
            #[cfg(feature = "metrics")]
            let insert_start = std::time::Instant::now();

            // Always add to the connection pool (regardless of k-bucket capacity)
            self.insert_to_connection_pool(node_id, node.clone());

            if self.contact_exists(&node_id) {
                // Contact already exists (main or replacement list), update protocol
                if let Some(contact) = self.get_contact_or_replacement_mut(&node_id) {
                    contact.add_protocol(protocol);
                }
            } else {
                let contact = Contact::new(node, protocol);
                self.insert_contact(node_id, contact);
                METRICS.record_new_discovery().await;
            }

            #[cfg(feature = "metrics")]
            {
                use ethrex_metrics::p2p::METRICS_P2P;
                METRICS_P2P.observe_insert_contact_duration(insert_start.elapsed().as_secs_f64());
            }
        }
    }

    fn do_record_enr_request_sent(&mut self, node_id: H256, request_hash: H256) {
        if let Some(contact) = self.get_contact_mut(&node_id) {
            contact.record_enr_request_sent(request_hash);
        }
    }

    fn do_record_enr_response_received(
        &mut self,
        node_id: H256,
        request_hash: H256,
        record: NodeRecord,
    ) {
        // Filtered here, before the mutable borrow, so a record that reaches us
        // over discv4 is judged by the same filter as one that arrives over
        // discv5. The verdict is recorded only if the record was actually
        // stored, so it always describes the record the contact holds.
        let passes_filter = self.filter.accepts(&record);
        if let Some(contact) = self.get_contact_mut(&node_id)
            && contact.record_enr_response_received(request_hash, record)
        {
            contact.passes_filter = Some(passes_filter);
        }
    }

    async fn do_new_contact_records(&mut self, node_records: Vec<NodeRecord>) {
        for node_record in node_records {
            if !node_record.verify_signature() {
                continue;
            }
            if let Ok(node) = Node::from_enr(&node_record) {
                let node_id = node.node_id();
                if node_id == self.local_node_id {
                    continue;
                }

                // Always add to the connection pool (regardless of k-bucket capacity)
                self.insert_to_connection_pool(node_id, node.clone());

                if self.contact_exists(&node_id) {
                    // Check if we need to evaluate fork_id before taking
                    // the mutable borrow.
                    let should_update = self
                        .get_contact_or_replacement(&node_id)
                        .map(|c| match c.record.as_ref() {
                            None => true,
                            Some(r) => node_record.seq > r.seq,
                        })
                        .unwrap_or(false);
                    // Filtered here, before the mutable borrow, and only when
                    // the record is newer than the one we already hold.
                    let passes_filter = should_update.then(|| self.filter.accepts(&node_record));
                    if let Some(contact) = self.get_contact_or_replacement_mut(&node_id) {
                        contact.add_protocol(DiscoveryProtocol::Discv5);
                        if should_update {
                            if contact.node.ip != node.ip || contact.node.udp_port != node.udp_port
                            {
                                contact.validation_timestamp = None;
                                contact.ping_id = None;
                            }
                            contact.node = node;
                            contact.record = Some(node_record);
                            contact.passes_filter = passes_filter;
                        }
                    }
                } else {
                    let passes_filter = self.filter.accepts(&node_record);
                    let mut contact = Contact::new(node, DiscoveryProtocol::Discv5);
                    contact.passes_filter = Some(passes_filter);
                    contact.record = Some(node_record);
                    self.insert_contact(node_id, contact);
                    METRICS.record_new_discovery().await;
                }
            }
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

    /// Picks a random connected peer advertising any of `capabilities`, weighted by
    /// score. Also returns the peer's advertised capabilities so callers can derive
    /// the negotiated protocol version (needed where the wire format differs between
    /// versions, e.g. receipts across eth/69 and eth/70).
    fn do_get_random_peer(
        &self,
        capabilities: Vec<Capability>,
    ) -> Option<(H256, PeerConnection, Vec<Capability>)> {
        let peers: Vec<(H256, &PeerConnection, i64, &Vec<Capability>)> = self
            .peers
            .iter()
            .filter_map(|(node_id, peer_data)| {
                if !capabilities
                    .iter()
                    .any(|cap| peer_data.supported_capabilities.contains(cap))
                {
                    return None;
                }
                peer_data.connection.as_ref().map(|connection| {
                    (
                        *node_id,
                        connection,
                        peer_data.score,
                        &peer_data.supported_capabilities,
                    )
                })
            })
            .collect();
        if peers.is_empty() {
            return None;
        }
        // Weight by score: maps [-150, 50] to [1, 201] so bad peers are unlikely but not excluded
        let weights: Vec<u64> = peers
            .iter()
            .map(|(_, _, score, _)| {
                (score.max(&MIN_SCORE_CRITICAL) - MIN_SCORE_CRITICAL + 1) as u64
            })
            .collect();
        let dist = WeightedIndex::new(&weights).ok()?;
        let idx = dist.sample(&mut rand::rngs::OsRng);
        Some((peers[idx].0, peers[idx].1.clone(), peers[idx].3.clone()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NodeRecordPairs;
    use ethrex_common::H512;
    use std::net::Ipv4Addr;

    /// Helper: build a dummy contact with a unique node derived from `seed`.
    fn dummy_contact(seed: u8) -> (H256, Contact) {
        let pk = H512::from_low_u64_be(seed as u64 + 1);
        let node = Node::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, seed)), 30303, 30303, pk);
        let node_id = node.node_id();
        let contact = Contact::new(node, DiscoveryProtocol::Discv4);
        (node_id, contact)
    }

    /// A filter with a fixed answer, so peer-table behaviour can be exercised
    /// without a storage engine or a real chain behind it.
    struct FixedAnswer(bool);

    impl PeerFilter for FixedAnswer {
        fn accepts(&self, _record: &NodeRecord) -> bool {
            self.0
        }
    }

    fn table_with(filter: impl PeerFilter + 'static) -> PeerTableServer {
        PeerTableServer::new(H256::zero(), 10, Box::new(filter))
    }

    /// A signed record for `seed`'s node at sequence number `seq`.
    fn record_for(seed: u8, seq: u64) -> (H256, NodeRecord) {
        let signer = secp256k1::SecretKey::from_slice(&[seed.max(1); 32]).unwrap();
        let record = NodeRecord::from_pairs(
            seq,
            &signer,
            NodeRecordPairs {
                ip: Some(Ipv4Addr::new(127, 0, 0, seed)),
                udp_port: Some(30303),
                ..Default::default()
            },
        )
        .unwrap();
        (Node::from_enr(&record).unwrap().node_id(), record)
    }

    // --- the filter decides which contacts are dialable ---

    #[tokio::test]
    async fn an_arriving_record_is_run_through_the_filter() {
        let mut table = table_with(FixedAnswer(false));
        let (node_id, record) = record_for(1, 1);

        table.do_new_contact_records(vec![record]).await;

        let contact = table.get_contact(&node_id).expect("contact inserted");
        assert_eq!(contact.passes_filter, Some(false));
    }

    #[tokio::test]
    async fn a_rejected_contact_is_never_offered_for_dialing() {
        let mut table = table_with(FixedAnswer(false));
        let (node_id, record) = record_for(2, 1);

        table.do_new_contact_records(vec![record]).await;
        assert!(table.get_contact(&node_id).is_some(), "contact is present");

        assert!(
            table.do_get_contact_to_initiate().is_none(),
            "a rejected contact must not be handed out to dial"
        );
    }

    #[tokio::test]
    async fn an_accepted_contact_is_offered_for_dialing() {
        let mut table = table_with(FixedAnswer(true));
        let (node_id, record) = record_for(3, 1);

        table.do_new_contact_records(vec![record]).await;

        // Asserting the stored answer too, not just dialability: `None` is also
        // dialable, so `is_some()` alone would pass even if the filter never ran.
        assert_eq!(
            table.get_contact(&node_id).unwrap().passes_filter,
            Some(true)
        );
        assert!(table.do_get_contact_to_initiate().is_some());
    }

    #[tokio::test]
    async fn a_contact_discovered_without_a_record_is_never_filtered() {
        // Bootnodes and discv4 neighbours arrive as bare endpoints. They have
        // published nothing to judge, so they must stay dialable rather than be
        // written off by a filter that never saw them.
        let mut table = table_with(FixedAnswer(false));
        let (node_id, record) = record_for(6, 1);
        let node = Node::from_enr(&record).unwrap();

        table
            .do_new_contacts(vec![node], DiscoveryProtocol::Discv4)
            .await;

        assert_eq!(table.get_contact(&node_id).unwrap().passes_filter, None);
        assert!(table.do_get_contact_to_initiate().is_some());
    }

    #[tokio::test]
    async fn a_discv4_enr_response_is_run_through_the_filter() {
        // The discv4 path used to bypass the filter entirely and write a
        // hardcoded fork-id verdict into the same field, so a consumer's own
        // policy was overridden depending on which protocol found the peer.
        let mut table = table_with(FixedAnswer(false));
        let (node_id, record) = record_for(7, 1);
        let node = Node::from_enr(&record).unwrap();
        let request_hash = H256::repeat_byte(0xab);

        table
            .do_new_contacts(vec![node], DiscoveryProtocol::Discv4)
            .await;
        table.do_record_enr_request_sent(node_id, request_hash);
        table.do_record_enr_response_received(node_id, request_hash, record);

        assert_eq!(
            table.get_contact(&node_id).unwrap().passes_filter,
            Some(false)
        );
    }

    #[tokio::test]
    async fn an_unsolicited_enr_response_does_not_set_the_verdict() {
        // The record is not stored when the hash does not match, so recording a
        // verdict from it would let a peer restate its own standing from a
        // record the table refused to keep.
        let mut table = table_with(FixedAnswer(true));
        let (node_id, record) = record_for(8, 1);
        let node = Node::from_enr(&record).unwrap();

        table
            .do_new_contacts(vec![node], DiscoveryProtocol::Discv4)
            .await;
        table.do_record_enr_request_sent(node_id, H256::repeat_byte(0x01));
        table.do_record_enr_response_received(node_id, H256::repeat_byte(0x02), record);

        let contact = table.get_contact(&node_id).unwrap();
        assert_eq!(contact.passes_filter, None);
        assert!(contact.record.is_none(), "the record must not be stored");
    }

    /// Rejects the first record it is shown and accepts every later one, so a
    /// test can tell whether a second record was filtered at all.
    #[derive(Default)]
    struct AcceptsFromTheSecondRecordOn(std::sync::atomic::AtomicUsize);

    impl PeerFilter for AcceptsFromTheSecondRecordOn {
        fn accepts(&self, _record: &NodeRecord) -> bool {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0
        }
    }

    #[tokio::test]
    async fn a_rejection_is_reconsidered_on_a_newer_record() {
        // The reason a rejection is stored rather than acted on once: the peer
        // republishes and we look again, instead of writing it off for the life
        // of the process over a fork id read against a head we had not synced.
        let mut table = table_with(AcceptsFromTheSecondRecordOn::default());
        let (node_id, first) = record_for(4, 1);
        let (_, newer) = record_for(4, 2);

        table.do_new_contact_records(vec![first]).await;
        assert_eq!(
            table.get_contact(&node_id).unwrap().passes_filter,
            Some(false)
        );

        table.do_new_contact_records(vec![newer]).await;
        assert_eq!(
            table.get_contact(&node_id).unwrap().passes_filter,
            Some(true),
            "a higher-seq record must get a fresh hearing"
        );
    }

    #[tokio::test]
    async fn an_older_record_does_not_re_filter_the_contact() {
        // `should_update` false means the record has nothing new to say, so the
        // answer already on the contact has to survive it.
        let mut table = table_with(AcceptsFromTheSecondRecordOn::default());
        let (node_id, first) = record_for(5, 2);
        let (_, older) = record_for(5, 1);

        table.do_new_contact_records(vec![first]).await;
        table.do_new_contact_records(vec![older]).await;

        assert_eq!(
            table.get_contact(&node_id).unwrap().passes_filter,
            Some(false),
            "a stale record must not overwrite the answer we hold"
        );
    }

    // --- KBucket::insert ---

    #[test]
    fn insert_into_empty_bucket() {
        let mut bucket = KBucket::default();
        let (id, contact) = dummy_contact(1);
        assert!(bucket.insert(id, contact));
        assert_eq!(bucket.contacts.len(), 1);
        assert!(bucket.replacements.is_empty());
    }

    #[test]
    fn insert_fills_bucket_then_goes_to_replacements() {
        let mut bucket = KBucket::default();

        // Fill the main list to capacity.
        for i in 0..MAX_NODES_PER_BUCKET as u8 {
            let (id, contact) = dummy_contact(i);
            assert!(bucket.insert(id, contact), "contact {i} should go to main");
        }
        assert_eq!(bucket.contacts.len(), MAX_NODES_PER_BUCKET);

        // The next insert should go to the replacement list.
        let (id, contact) = dummy_contact(200);
        assert!(!bucket.insert(id, contact));
        assert_eq!(bucket.contacts.len(), MAX_NODES_PER_BUCKET);
        assert_eq!(bucket.replacements.len(), 1);
    }

    // --- KBucket::contains ---

    #[test]
    fn contains_checks_main_and_replacement() {
        let mut bucket = KBucket::default();

        let (id_main, contact_main) = dummy_contact(1);
        bucket.insert(id_main, contact_main);
        assert!(bucket.contains(&id_main));

        // Fill bucket so next goes to replacement.
        for i in 2..=(MAX_NODES_PER_BUCKET as u8) {
            let (id, c) = dummy_contact(i);
            bucket.insert(id, c);
        }
        let (id_repl, contact_repl) = dummy_contact(100);
        bucket.insert(id_repl, contact_repl);

        assert!(bucket.contains(&id_repl));
        assert!(!bucket.contains(&H256::zero()));
    }

    // --- KBucket::get / get_any ---

    #[test]
    fn get_returns_main_list_only() {
        let mut bucket = KBucket::default();
        let (id, contact) = dummy_contact(1);
        bucket.insert(id, contact);
        assert!(bucket.get(&id).is_some());
        assert!(bucket.get(&H256::zero()).is_none());
    }

    #[test]
    fn get_any_returns_from_replacement() {
        let mut bucket = KBucket::default();
        // Fill main list.
        for i in 0..MAX_NODES_PER_BUCKET as u8 {
            let (id, c) = dummy_contact(i);
            bucket.insert(id, c);
        }
        // Insert into replacements.
        let (id_repl, c_repl) = dummy_contact(200);
        bucket.insert(id_repl, c_repl);

        assert!(bucket.get(&id_repl).is_none()); // not in main
        assert!(bucket.get_any(&id_repl).is_some()); // found via replacement
    }

    // --- KBucket::remove_and_promote ---

    #[test]
    fn remove_and_promote_with_replacement() {
        let mut bucket = KBucket::default();

        // Fill main list.
        let mut main_ids = Vec::new();
        for i in 0..MAX_NODES_PER_BUCKET as u8 {
            let (id, c) = dummy_contact(i);
            main_ids.push(id);
            bucket.insert(id, c);
        }

        // Add a replacement.
        let (repl_id, repl_contact) = dummy_contact(200);
        bucket.insert(repl_id, repl_contact);

        // Remove a main contact — the replacement should be promoted.
        let promoted = bucket.remove_and_promote(&main_ids[0]);
        assert_eq!(promoted, Some(repl_id));
        assert_eq!(bucket.contacts.len(), MAX_NODES_PER_BUCKET);
        assert!(bucket.replacements.is_empty());
        assert!(!bucket.contains(&main_ids[0]));
        assert!(bucket.contains(&repl_id));
    }

    #[test]
    fn remove_and_promote_without_replacement() {
        let mut bucket = KBucket::default();
        let (id, c) = dummy_contact(1);
        bucket.insert(id, c);

        let promoted = bucket.remove_and_promote(&id);
        assert!(promoted.is_none());
        assert!(bucket.contacts.is_empty());
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mut bucket = KBucket::default();
        assert!(bucket.remove_and_promote(&H256::zero()).is_none());
    }

    // --- Replacement eviction ---

    #[test]
    fn replacement_list_evicts_oldest_when_full() {
        let mut bucket = KBucket::default();
        // Fill main list.
        for i in 0..MAX_NODES_PER_BUCKET as u8 {
            let (id, c) = dummy_contact(i);
            bucket.insert(id, c);
        }

        // Fill replacement list beyond capacity.
        let mut repl_ids = Vec::new();
        for i in 0..(MAX_REPLACEMENTS_PER_BUCKET + 2) as u8 {
            let seed = 100 + i;
            let (id, c) = dummy_contact(seed);
            repl_ids.push(id);
            bucket.insert(id, c);
        }

        assert_eq!(bucket.replacements.len(), MAX_REPLACEMENTS_PER_BUCKET);
        // The oldest two should have been evicted.
        assert!(!bucket.contains(&repl_ids[0]));
        assert!(!bucket.contains(&repl_ids[1]));
        // The most recent ones should still be there.
        assert!(bucket.contains(repl_ids.last().unwrap()));
    }

    // --- bucket_index ---

    #[test]
    fn bucket_index_self_is_none() {
        let id = H256::random();
        assert_eq!(bucket_index(&id, &id), None);
    }

    #[test]
    fn bucket_index_minimal_distance() {
        let local = H256::zero();
        // XOR distance = 1 → highest bit is bit 0 → bucket 0
        let mut remote = H256::zero();
        remote.0[31] = 1;
        assert_eq!(bucket_index(&local, &remote), Some(0));
    }

    #[test]
    fn bucket_index_maximal_distance() {
        let local = H256::zero();
        // XOR distance has highest bit at position 255 → bucket 255
        let mut remote = H256::zero();
        remote.0[0] = 0x80;
        assert_eq!(bucket_index(&local, &remote), Some(255));
    }
}
