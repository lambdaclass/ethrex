use crate::{
    backend,
    discv5::session::Session,
    metrics::METRICS,
    rlpx::{connection::server::PeerConnection, p2p::Capability},
    types::{Node, NodeRecord},
    utils::distance,
};
use bytes::Bytes;
use ethrex_common::H256;
use ethrex_storage::Store;
use indexmap::{IndexMap, map::Entry};
use rand::seq::SliceRandom;
use rustc_hash::FxHashSet;
use spawned_concurrency::{
    error::GenServerError,
    tasks::{CallResponse, CastResponse, GenServer, GenServerHandle, InitResult, send_message_on},
};
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};
use thiserror::Error;

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
/// Maximum number of ENRs to return in a FindNode response (across all NODES messages).
/// See: https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire.md#nodes-response-0x04
const MAX_ENRS_PER_FINDNODE_RESPONSE: usize = 16;

#[derive(Debug, Clone)]
pub struct Contact {
    pub node: Node,
    /// The timestamp when the contact was last sent a ping.
    /// If None, the contact has never been pinged.
    pub validation_timestamp: Option<Instant>,
    /// The req_id of the last unacknowledged ping sent to this contact, or
    /// None if no ping was sent yet or it was already acknowledged.
    pub ping_req_id: Option<Bytes>,

    /// The hash of the last unacknowledged ENRRequest sent to this contact, or
    /// None if no request was sent yet or it was already acknowledged.
    pub enr_request_hash: Option<H256>,

    pub n_find_node_sent: u64,
    /// ENR associated with this contact, if it was provided by the peer.
    pub record: Option<NodeRecord>,
    // This contact failed to respond our Ping.
    pub disposable: bool,
    // Set to true after we send a successful ENRResponse to it.
    pub knows_us: bool,
    // This is a known-bad peer (on another network, no matching capabilities, etc)
    pub unwanted: bool,
    /// Whether the last known fork ID is valid, None if unknown.
    pub is_fork_id_valid: Option<bool>,
    /// Session information for discv5
    session: Option<Session>,
}

impl Contact {
    pub fn was_validated(&self) -> bool {
        self.validation_timestamp.is_some() && !self.has_pending_ping()
    }

    pub fn has_pending_ping(&self) -> bool {
        self.ping_req_id.is_some()
    }

    pub fn record_ping_sent(&mut self, req_id: Bytes) {
        self.validation_timestamp = Some(Instant::now());
        self.ping_req_id = Some(req_id);
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

impl From<Node> for Contact {
    fn from(node: Node) -> Self {
        Self {
            node,
            validation_timestamp: None,
            ping_req_id: None,
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

#[derive(Clone, Debug)]
pub struct PeerTable {
    handle: GenServerHandle<PeerTableServer>,
}

impl PeerTable {
    pub fn spawn(target_peers: usize, store: Store) -> PeerTable {
        PeerTable {
            handle: PeerTableServer::new(target_peers, store).start(),
        }
    }

    /// We received a list of Nodes to contact. No conection has been established yet.
    pub async fn new_contacts(
        &mut self,
        nodes: Vec<Node>,
        local_node_id: H256,
    ) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::NewContacts {
                nodes,
                local_node_id,
            })
            .await?;
        Ok(())
    }

    /// We received a list of NodeRecords to contact. No conection has been established yet.
    pub async fn new_contact_records(
        &mut self,
        node_records: Vec<NodeRecord>,
        local_node_id: H256,
    ) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::NewContactRecords {
                node_records,
                local_node_id,
            })
            .await?;
        Ok(())
    }

    /// We have established a connection with the remote peer.
    pub async fn new_connected_peer(
        &mut self,
        node: Node,
        connection: PeerConnection,
        capabilities: Vec<Capability>,
    ) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::NewConnectedPeer {
                node,
                connection,
                capabilities,
            })
            .await?;
        Ok(())
    }

    /// Set or update discv5 Session info.
    pub async fn set_session_info(
        &mut self,
        node_id: H256,
        session: Session,
    ) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::SetSessionInfo { node_id, session })
            .await?;
        Ok(())
    }

    /// Remove from list of connected peers.
    pub async fn remove_peer(&mut self, node_id: H256) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::RemovePeer { node_id })
            .await?;
        Ok(())
    }

    /// Increment the number of ongoing requests for this peer
    pub async fn inc_requests(&mut self, node_id: H256) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::IncRequests { node_id })
            .await?;
        Ok(())
    }

    /// Decrement the number of ongoing requests for this peer
    pub async fn dec_requests(&mut self, node_id: H256) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::DecRequests { node_id })
            .await?;
        Ok(())
    }

    /// Mark node as not wanted
    pub async fn set_unwanted(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::SetUnwanted { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Set whether the contact fork id is valid.
    pub async fn set_is_fork_id_valid(
        &mut self,
        node_id: &H256,
        valid: bool,
    ) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::SetIsForkIdValid {
                node_id: *node_id,
                valid,
            })
            .await?;
        Ok(())
    }

    /// Record a successful connection, used to score peers
    pub async fn record_success(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::RecordSuccess { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Record a failed connection, used to score peers
    pub async fn record_failure(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::RecordFailure { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Record a critical failure for connection, used to score peers
    pub async fn record_critical_failure(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::RecordCriticalFailure { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Record ping sent, store the req_id for later check
    pub async fn record_ping_sent(
        &mut self,
        node_id: &H256,
        req_id: Bytes,
    ) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::RecordPingSent {
                node_id: *node_id,
                req_id,
            })
            .await?;
        Ok(())
    }

    /// Record a pong received. Check previously saved req_id and reset it if it matches
    pub async fn record_pong_received(
        &mut self,
        node_id: &H256,
        req_id: Bytes,
    ) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::RecordPongReceived {
                node_id: *node_id,
                req_id,
            })
            .await?;
        Ok(())
    }

    /// Record request sent, store the request hash for later check
    pub async fn record_enr_request_sent(
        &mut self,
        node_id: &H256,
        request_hash: H256,
    ) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::RecordEnrRequestSent {
                node_id: *node_id,
                request_hash,
            })
            .await?;
        Ok(())
    }

    /// Record a response received. Check previously saved hash and reset it if it matches
    pub async fn record_enr_response_received(
        &mut self,
        node_id: &H256,
        request_hash: H256,
        record: NodeRecord,
    ) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::RecordEnrResponseReceived {
                node_id: *node_id,
                request_hash,
                record,
            })
            .await?;
        Ok(())
    }

    /// Set peer as disposable
    pub async fn set_disposable(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::SetDisposable { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Increment FindNode message counter for peer
    pub async fn increment_find_node_sent(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::IncrementFindNodeSent { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Set flag for peer that tells that it knows us
    pub async fn knows_us(&mut self, node_id: &H256) -> Result<(), PeerTableError> {
        self.handle
            .cast(CastMessage::KnowsUs { node_id: *node_id })
            .await?;
        Ok(())
    }

    /// Remove from list of contacts the ones marked as disposable
    pub async fn prune(&mut self) -> Result<(), PeerTableError> {
        self.handle.cast(CastMessage::Prune).await?;
        Ok(())
    }

    /// Return the amount of connected peers
    pub async fn peer_count(&mut self) -> Result<usize, PeerTableError> {
        match self.handle.call(CallMessage::PeerCount).await? {
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
            .handle
            .call(CallMessage::PeerCountByCapabilities {
                capabilities: capabilities.to_vec(),
            })
            .await?
        {
            OutMessage::PeerCount(peer_count) => Ok(peer_count),
            _ => unreachable!(),
        }
    }

    /// Check if target number of contacts and connected peers is reached
    pub async fn target_reached(&mut self) -> Result<bool, PeerTableError> {
        match self.handle.call(CallMessage::TargetReached).await? {
            OutMessage::TargetReached(result) => Ok(result),
            _ => unreachable!(),
        }
    }

    /// Check if target number of connected peers is reached
    pub async fn target_peers_reached(&mut self) -> Result<bool, PeerTableError> {
        match self.handle.call(CallMessage::TargetPeersReached).await? {
            OutMessage::TargetReached(result) => Ok(result),
            _ => unreachable!(),
        }
    }

    /// Return rate of target peers completion
    pub async fn target_peers_completion(&mut self) -> Result<f64, PeerTableError> {
        match self.handle.call(CallMessage::TargetPeersCompletion).await? {
            OutMessage::TargetCompletion(result) => Ok(result),
            _ => unreachable!(),
        }
    }

    /// Provide a contact to initiate a connection
    pub async fn get_contact_to_initiate(&mut self) -> Result<Option<Contact>, PeerTableError> {
        match self.handle.call(CallMessage::GetContactToInitiate).await? {
            OutMessage::Contact(contact) => Ok(Some(*contact)),
            OutMessage::NotFound => Ok(None),
            _ => unreachable!(),
        }
    }

    /// Provide a contact to perform Discovery lookup
    pub async fn get_contact_for_lookup(&mut self) -> Result<Option<Contact>, PeerTableError> {
        match self.handle.call(CallMessage::GetContactForLookup).await? {
            OutMessage::Contact(contact) => Ok(Some(*contact)),
            OutMessage::NotFound => Ok(None),
            _ => unreachable!(),
        }
    }

    /// Provide a contact to perform ENR lookup
    pub async fn get_contact_for_enr_lookup(&mut self) -> Result<Option<Contact>, PeerTableError> {
        match self
            .handle
            .call(CallMessage::GetContactForEnrLookup)
            .await?
        {
            OutMessage::Contact(contact) => Ok(Some(*contact)),
            OutMessage::NotFound => Ok(None),
            _ => unreachable!(),
        }
    }

    /// Get a contact using node_id
    pub async fn get_contact(&mut self, node_id: H256) -> Result<Option<Contact>, PeerTableError> {
        match self
            .handle
            .call(CallMessage::GetContact { node_id })
            .await?
        {
            OutMessage::Contact(contact) => Ok(Some(*contact)),
            OutMessage::NotFound => Ok(None),
            _ => unreachable!(),
        }
    }

    /// Get discv5 Session info.
    pub async fn get_session_info(
        &mut self,
        node_id: H256,
    ) -> Result<Option<Session>, PeerTableError> {
        match self
            .handle
            .call(CallMessage::GetContact { node_id })
            .await?
        {
            OutMessage::Contact(contact) => Ok(contact.session),
            OutMessage::NotFound => Ok(None),
            _ => unreachable!(),
        }
    }

    /// Get all contacts available to revalidate
    pub async fn get_contacts_to_revalidate(
        &mut self,
        revalidation_interval: Duration,
    ) -> Result<Vec<Contact>, PeerTableError> {
        match self
            .handle
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
    ) -> Result<Option<(H256, PeerConnection)>, PeerTableError> {
        match self
            .handle
            .call(CallMessage::GetBestPeer {
                capabilities: capabilities.to_vec(),
            })
            .await?
        {
            OutMessage::FoundPeer {
                node_id,
                connection,
            } => Ok(Some((node_id, connection))),
            OutMessage::NotFound => Ok(None),
            _ => unreachable!(),
        }
    }

    /// Get peer score
    pub async fn get_score(&mut self, node_id: &H256) -> Result<i64, PeerTableError> {
        match self
            .handle
            .call(CallMessage::GetScore { node_id: *node_id })
            .await?
        {
            OutMessage::PeerScore(score) => Ok(score),
            _ => unreachable!(),
        }
    }

    /// Get list of connected peers
    pub async fn get_connected_nodes(&mut self) -> Result<Vec<Node>, PeerTableError> {
        if let OutMessage::Nodes(nodes) = self.handle.call(CallMessage::GetConnectedNodes).await? {
            Ok(nodes)
        } else {
            unreachable!()
        }
    }

    /// Get list of connected peers with their capabilities
    pub async fn get_peers_with_capabilities(
        &mut self,
    ) -> Result<Vec<(H256, PeerConnection, Vec<Capability>)>, PeerTableError> {
        match self
            .handle
            .call(CallMessage::GetPeersWithCapabilities)
            .await?
        {
            OutMessage::PeersWithCapabilities(peers_with_capabilities) => {
                Ok(peers_with_capabilities)
            }
            _ => unreachable!(),
        }
    }

    /// Get peer channels for communication. It returns a PeerConnection that implements
    /// at least one of the required capabilities.
    pub async fn get_peer_connections(
        &mut self,
        capabilities: &[Capability],
    ) -> Result<Vec<(H256, PeerConnection)>, PeerTableError> {
        match self
            .handle
            .call(CallMessage::GetPeerConnections {
                capabilities: capabilities.to_vec(),
            })
            .await?
        {
            OutMessage::PeerConnection(connection) => Ok(connection),
            _ => unreachable!(),
        }
    }

    /// Insert new peer if it is new. Returns a boolean telling if it was new or not.
    pub async fn insert_if_new(&mut self, node: &Node) -> Result<bool, PeerTableError> {
        match self
            .handle
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
        self.handle
            .call(CallMessage::ValidateContact {
                node_id: *node_id,
                sender_ip,
            })
            .await
            .map_err(PeerTableError::InternalError)
    }

    /// Get closest nodes according to kademlia's distance
    pub async fn get_nodes_at_distances(
        &mut self,
        local_node_id: H256,
        distances: Vec<u32>,
    ) -> Result<Vec<NodeRecord>, PeerTableError> {
        match self
            .handle
            .call(CallMessage::GetNodesAtDistances {
                local_node_id,
                distances,
            })
            .await?
        {
            OutMessage::NodeRecords(records) => Ok(records),
            _ => unreachable!(),
        }
    }

    /// Get metadata associated to peer
    pub async fn get_peers_data(&mut self) -> Result<Vec<PeerData>, PeerTableError> {
        match self.handle.call(CallMessage::GetPeersData).await? {
            OutMessage::PeersData(peers_data) => Ok(peers_data),
            _ => unreachable!(),
        }
    }

    /// Retrieve a random peer.
    pub async fn get_random_peer(
        &mut self,
        capabilities: &[Capability],
    ) -> Result<Option<(H256, PeerConnection)>, PeerTableError> {
        match self
            .handle
            .call(CallMessage::GetRandomPeer {
                capabilities: capabilities.to_vec(),
            })
            .await?
        {
            OutMessage::FoundPeer {
                node_id,
                connection,
            } => Ok(Some((node_id, connection))),
            OutMessage::NotFound => Ok(None),
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
struct PeerTableServer {
    contacts: IndexMap<H256, Contact>,
    peers: IndexMap<H256, PeerData>,
    already_tried_peers: FxHashSet<H256>,
    discarded_contacts: FxHashSet<H256>,
    target_peers: usize,
    store: Store,
}

impl PeerTableServer {
    pub(crate) fn new(target_peers: usize, store: Store) -> Self {
        Self {
            contacts: Default::default(),
            peers: Default::default(),
            already_tried_peers: Default::default(),
            discarded_contacts: Default::default(),
            target_peers,
            store,
        }
    }
    // Internal functions //

    // Weighting function used to select best peer
    // TODO: Review this formula and weight constants.
    fn weight_peer(&self, score: &i64, requests: &i64) -> i64 {
        score * SCORE_WEIGHT - requests * REQUESTS_WEIGHT
    }

    // Returns if the peer has room for more connections given the current score
    // and amount of inflight requests
    fn can_try_more_requests(&self, score: &i64, requests: &i64) -> bool {
        let score_ratio = (score - MIN_SCORE) as f64 / (MAX_SCORE - MIN_SCORE) as f64;
        (*requests as f64) <= MAX_CONCURRENT_REQUESTS_PER_PEER as f64 * score_ratio
    }

    fn get_best_peer(&self, capabilities: &[Capability]) -> Option<(H256, PeerConnection)> {
        self.peers
            .iter()
            // We filter only to those peers which are useful to us
            .filter_map(|(id, peer_data)| {
                // Skip the peer if it has too many ongoing requests or if it doesn't match
                // the capabilities
                if !self.can_try_more_requests(&peer_data.score, &peer_data.requests)
                    || !capabilities
                        .iter()
                        .any(|cap| peer_data.supported_capabilities.contains(cap))
                {
                    None
                } else {
                    // if the peer doesn't have the channel open, we skip it.
                    let connection = peer_data.connection.clone()?;

                    // We return the id, the score and the channel to connect with.
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

    fn get_contact_to_initiate(&mut self) -> Option<Contact> {
        for contact in self.contacts.values() {
            let node_id = contact.node.node_id();
            if !self.peers.contains_key(&node_id)
                && !self.already_tried_peers.contains(&node_id)
                && contact.knows_us
                && !contact.unwanted
                && contact.is_fork_id_valid == Some(true)
            {
                self.already_tried_peers.insert(node_id);

                return Some(contact.clone());
            }
        }
        // No untried contact found, resetting tried peers.
        tracing::trace!("Resetting list of tried peers.");
        self.already_tried_peers.clear();
        None
    }

    fn get_contact_for_lookup(&self) -> Option<Contact> {
        self.contacts
            .values()
            .filter(|c| c.n_find_node_sent < MAX_FIND_NODE_PER_PEER && !c.disposable)
            .collect::<Vec<_>>()
            .choose(&mut rand::rngs::OsRng)
            .cloned()
            .cloned()
    }

    fn get_contact_for_enr_lookup(&mut self) -> Option<Contact> {
        self.contacts
            .values()
            .filter(|c| {
                c.was_validated()
                    && !c.has_pending_enr_request()
                    && c.record.is_none()
                    && !c.disposable
            })
            .collect::<Vec<_>>()
            .choose(&mut rand::rngs::OsRng)
            .cloned()
            .cloned()
    }

    fn get_contacts_to_revalidate(&self, revalidation_interval: Duration) -> Vec<Contact> {
        self.contacts
            .values()
            .filter(|c| Self::is_validation_needed(c, revalidation_interval))
            .cloned()
            .collect()
    }

    fn validate_contact(&self, node_id: H256, sender_ip: IpAddr) -> OutMessage {
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
        OutMessage::Contact(Box::new(contact.clone()))
    }

    fn get_nodes_at_distances(&self, local_node_id: H256, distances: &[u32]) -> Vec<NodeRecord> {
        self.contacts
            .iter()
            .filter_map(|(contact_id, contact)| {
                let d = distance(&local_node_id, contact_id) as u32;
                if distances.contains(&d) {
                    contact.record.clone()
                } else {
                    None
                }
            })
            .take(MAX_ENRS_PER_FINDNODE_RESPONSE)
            .collect()
    }

    async fn new_contacts(&mut self, nodes: Vec<Node>, local_node_id: H256) {
        for node in nodes {
            let node_id = node.node_id();
            if let Entry::Vacant(vacant_entry) = self.contacts.entry(node_id)
                && !self.discarded_contacts.contains(&node_id)
                && node_id != local_node_id
            {
                vacant_entry.insert(Contact::from(node));
                METRICS.record_new_discovery().await;
            }
        }
    }

    async fn new_contact_records(&mut self, node_records: Vec<NodeRecord>, local_node_id: H256) {
        for node_record in node_records {
            if let Ok(node) = Node::from_enr(&node_record) {
                let node_id = node.node_id();
                if let Entry::Vacant(vacant_entry) = self.contacts.entry(node_id)
                    && !self.discarded_contacts.contains(&node_id)
                    && node_id != local_node_id
                {
                    let mut contact = Contact::from(node);
                    let is_fork_id_valid =
                        if let Some(remote_fork_id) = node_record.decode_pairs().eth {
                            backend::is_fork_id_valid(&self.store, &remote_fork_id)
                                .await
                                .ok()
                                .or(Some(false))
                        } else {
                            Some(false)
                        };
                    contact.is_fork_id_valid = is_fork_id_valid;
                    contact.record = Some(node_record);
                    vacant_entry.insert(contact);
                    METRICS.record_new_discovery().await;
                }
                // TODO Handle the case the contact is already present
            }
        }
    }

    fn peer_count_by_capabilities(&self, capabilities: Vec<Capability>) -> usize {
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

    fn get_peer_connections(&self, capabilities: Vec<Capability>) -> Vec<(H256, PeerConnection)> {
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
                    .connection
                    .clone()
                    .map(|connection| (*peer_id, connection))
            })
            .collect()
    }

    fn get_random_peer(&self, capabilities: Vec<Capability>) -> Option<(H256, PeerConnection)> {
        let peers: Vec<(H256, PeerConnection)> = self
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
                    .connection
                    .clone()
                    .map(|connection| (*node_id, connection))
            })
            .collect();
        peers.choose(&mut rand::rngs::OsRng).cloned()
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
enum CastMessage {
    NewContacts {
        nodes: Vec<Node>,
        local_node_id: H256,
    },
    NewContactRecords {
        node_records: Vec<NodeRecord>,
        local_node_id: H256,
    },
    NewConnectedPeer {
        node: Node,
        connection: PeerConnection,
        capabilities: Vec<Capability>,
    },
    SetSessionInfo {
        node_id: H256,
        session: Session,
    },
    RemovePeer {
        node_id: H256,
    },
    IncRequests {
        node_id: H256,
    },
    DecRequests {
        node_id: H256,
    },
    SetUnwanted {
        node_id: H256,
    },
    SetIsForkIdValid {
        node_id: H256,
        valid: bool,
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
        req_id: Bytes,
    },
    RecordPongReceived {
        node_id: H256,
        req_id: Bytes,
    },
    RecordEnrRequestSent {
        node_id: H256,
        request_hash: H256,
    },
    RecordEnrResponseReceived {
        node_id: H256,
        request_hash: H256,
        record: NodeRecord,
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
    Shutdown,
}

#[derive(Clone, Debug)]
enum CallMessage {
    PeerCount,
    PeerCountByCapabilities {
        capabilities: Vec<Capability>,
    },
    TargetReached,
    TargetPeersReached,
    TargetPeersCompletion,
    GetContactToInitiate,
    GetContactForLookup,
    GetContactForEnrLookup,
    GetContact {
        node_id: H256,
    },
    GetContactsToRevalidate(Duration),
    GetBestPeer {
        capabilities: Vec<Capability>,
    },
    GetScore {
        node_id: H256,
    },
    GetConnectedNodes,
    GetPeersWithCapabilities,
    GetPeerConnections {
        capabilities: Vec<Capability>,
    },
    InsertIfNew {
        node: Node,
    },
    ValidateContact {
        node_id: H256,
        sender_ip: IpAddr,
    },
    GetNodesAtDistances {
        local_node_id: H256,
        distances: Vec<u32>,
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
        connection: PeerConnection,
    },
    NotFound,
    PeerScore(i64),
    PeersWithCapabilities(Vec<(H256, PeerConnection, Vec<Capability>)>),
    PeerConnection(Vec<(H256, PeerConnection)>),
    Contacts(Vec<Contact>),
    TargetReached(bool),
    TargetCompletion(f64),
    IsNew(bool),
    Nodes(Vec<Node>),
    NodeRecords(Vec<NodeRecord>),
    Contact(Box<Contact>),
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

impl GenServer for PeerTableServer {
    type CallMsg = CallMessage;
    type CastMsg = CastMessage;
    type OutMsg = OutMessage;
    type Error = PeerTableError;

    async fn init(self, handle: &GenServerHandle<Self>) -> Result<InitResult<Self>, Self::Error> {
        send_message_on(
            handle.clone(),
            tokio::signal::ctrl_c(),
            CastMessage::Shutdown,
        );
        Ok(InitResult::Success(self))
    }

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        _handle: &GenServerHandle<PeerTableServer>,
    ) -> CallResponse<Self> {
        match message {
            CallMessage::PeerCount => {
                CallResponse::Reply(Self::OutMsg::PeerCount(self.peers.len()))
            }
            CallMessage::PeerCountByCapabilities { capabilities } => CallResponse::Reply(
                OutMessage::PeerCount(self.peer_count_by_capabilities(capabilities)),
            ),
            CallMessage::TargetReached => CallResponse::Reply(Self::OutMsg::TargetReached(
                self.contacts.len() >= TARGET_CONTACTS && self.peers.len() >= self.target_peers,
            )),
            CallMessage::TargetPeersReached => CallResponse::Reply(Self::OutMsg::TargetReached(
                self.peers.len() >= self.target_peers,
            )),
            CallMessage::TargetPeersCompletion => CallResponse::Reply(
                Self::OutMsg::TargetCompletion(self.peers.len() as f64 / self.target_peers as f64),
            ),
            CallMessage::GetContactToInitiate => CallResponse::Reply(
                self.get_contact_to_initiate()
                    .map(Box::new)
                    .map_or(Self::OutMsg::NotFound, Self::OutMsg::Contact),
            ),
            CallMessage::GetContactForLookup => CallResponse::Reply(
                self.get_contact_for_lookup()
                    .map(Box::new)
                    .map_or(Self::OutMsg::NotFound, Self::OutMsg::Contact),
            ),
            CallMessage::GetContactForEnrLookup => CallResponse::Reply(
                self.get_contact_for_enr_lookup()
                    .map(Box::new)
                    .map_or(Self::OutMsg::NotFound, Self::OutMsg::Contact),
            ),
            CallMessage::GetContact { node_id } => CallResponse::Reply(
                self.contacts
                    .get(&node_id)
                    .cloned()
                    .map(Box::new)
                    .map_or(Self::OutMsg::NotFound, Self::OutMsg::Contact),
            ),
            CallMessage::GetContactsToRevalidate(revalidation_interval) => CallResponse::Reply(
                Self::OutMsg::Contacts(self.get_contacts_to_revalidate(revalidation_interval)),
            ),
            CallMessage::GetBestPeer { capabilities } => {
                let channels = self.get_best_peer(&capabilities);
                CallResponse::Reply(channels.map_or(
                    Self::OutMsg::NotFound,
                    |(node_id, connection)| Self::OutMsg::FoundPeer {
                        node_id,
                        connection,
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
                            peer_data.connection.clone().map(|connection| {
                                (
                                    *peer_id,
                                    connection,
                                    peer_data.supported_capabilities.clone(),
                                )
                            })
                        })
                        .collect(),
                ))
            }
            CallMessage::GetPeerConnections { capabilities } => CallResponse::Reply(
                OutMessage::PeerConnection(self.get_peer_connections(capabilities)),
            ),
            CallMessage::InsertIfNew { node } => CallResponse::Reply(Self::OutMsg::IsNew(
                match self.contacts.entry(node.node_id()) {
                    Entry::Occupied(_) => false,
                    Entry::Vacant(entry) => {
                        METRICS.record_new_discovery().await;
                        entry.insert(Contact::from(node));
                        true
                    }
                },
            )),
            CallMessage::ValidateContact { node_id, sender_ip } => {
                CallResponse::Reply(self.validate_contact(node_id, sender_ip))
            }
            CallMessage::GetNodesAtDistances {
                local_node_id,
                distances,
            } => CallResponse::Reply(Self::OutMsg::NodeRecords(
                self.get_nodes_at_distances(local_node_id, &distances),
            )),
            CallMessage::GetPeersData => CallResponse::Reply(OutMessage::PeersData(
                self.peers.values().cloned().collect(),
            )),
            CallMessage::GetRandomPeer { capabilities } => CallResponse::Reply(
                if let Some((node_id, connection)) = self.get_random_peer(capabilities) {
                    OutMessage::FoundPeer {
                        node_id,
                        connection,
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
        _handle: &GenServerHandle<PeerTableServer>,
    ) -> CastResponse {
        match message {
            CastMessage::NewContacts {
                nodes,
                local_node_id,
            } => {
                self.new_contacts(nodes, local_node_id).await;
            }
            CastMessage::NewContactRecords {
                node_records,
                local_node_id,
            } => {
                self.new_contact_records(node_records, local_node_id).await;
            }
            CastMessage::NewConnectedPeer {
                node,
                connection,
                capabilities,
            } => {
                let new_peer_id = node.node_id();
                let new_peer = PeerData::new(node, None, Some(connection), capabilities);
                self.peers.insert(new_peer_id, new_peer);
            }
            CastMessage::SetSessionInfo { node_id, session } => {
                self.contacts
                    .entry(node_id)
                    .and_modify(|contact| contact.session = Some(session));
            }
            CastMessage::RemovePeer { node_id } => {
                self.peers.swap_remove(&node_id);
            }
            CastMessage::IncRequests { node_id } => {
                self.peers
                    .entry(node_id)
                    .and_modify(|peer_data| peer_data.requests += 1);
            }
            CastMessage::DecRequests { node_id } => {
                self.peers
                    .entry(node_id)
                    .and_modify(|peer_data| peer_data.requests -= 1);
            }
            CastMessage::SetUnwanted { node_id } => {
                self.contacts
                    .entry(node_id)
                    .and_modify(|contact| contact.unwanted = true);
            }
            CastMessage::SetIsForkIdValid { node_id, valid } => {
                self.contacts
                    .entry(node_id)
                    .and_modify(|contact| contact.is_fork_id_valid = Some(valid));
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
            CastMessage::RecordPingSent { node_id, req_id } => {
                self.contacts
                    .entry(node_id)
                    .and_modify(|contact| contact.record_ping_sent(req_id));
            }
            CastMessage::RecordPongReceived { node_id, req_id } => {
                // If entry does not exist or req_id does not match, ignore
                // Otherwise, reset ping_req_id
                self.contacts.entry(node_id).and_modify(|contact| {
                    if contact
                        .ping_req_id
                        .as_ref()
                        .map(|value| *value == req_id)
                        .unwrap_or(false)
                    {
                        contact.ping_req_id = None
                    }
                });
            }
            CastMessage::RecordEnrRequestSent {
                node_id,
                request_hash,
            } => {
                self.contacts
                    .entry(node_id)
                    .and_modify(|contact| contact.record_enr_request_sent(request_hash));
            }
            CastMessage::RecordEnrResponseReceived {
                node_id,
                request_hash,
                record,
            } => {
                self.contacts.entry(node_id).and_modify(|contact| {
                    contact.record_enr_response_received(request_hash, record);
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
            CastMessage::Shutdown => return CastResponse::Stop,
        }
        CastResponse::NoReply
    }
}
