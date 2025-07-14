use crate::{
    discv4::messages::FindNodeRequest,
    rlpx::{connection::server::RLPxConnection, message::Message as RLPxMessage, p2p::Capability},
    types::{Node, NodeRecord},
};
use ethrex_common::{H256, U256};
use rand::random;
use spawned_concurrency::tasks::GenServerHandle;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::{Mutex, mpsc};
use tracing::debug;

pub const MAX_NODES_PER_BUCKET: usize = 16;
const NUMBER_OF_BUCKETS: usize = 256;
const MAX_NUMBER_OF_REPLACEMENTS: usize = 10;

/// Maximum Peer Score to avoid overflows upon weigh calculations
const PEER_SCORE_UPPER_BOUND: i32 = 500;
/// Mininum Peer Score, this is a soft bound that can be temporarily exceeded by a critical failure
const PEER_SCORE_LOWER_BOUND: i32 = -500;

#[derive(Clone, Debug, Default)]
pub struct Bucket {
    pub peers: Vec<PeerData>,
    pub replacements: Vec<PeerData>,
}

#[derive(Debug)]
pub struct KademliaTable {
    local_node_id: H256,
    buckets: Vec<Bucket>,
}

impl KademliaTable {
    pub fn new(local_node_id: H256) -> Self {
        let buckets: Vec<Bucket> = vec![Bucket::default(); NUMBER_OF_BUCKETS];
        Self {
            local_node_id,
            buckets,
        }
    }

    #[allow(unused)]
    pub fn buckets(&self) -> &Vec<Bucket> {
        &self.buckets
    }

    pub fn get_by_node_id(&self, node_id: H256) -> Option<&PeerData> {
        let bucket = &self.buckets[bucket_number(node_id, self.local_node_id)];
        bucket
            .peers
            .iter()
            .find(|entry| entry.node.node_id() == node_id)
    }

    pub fn get_by_node_id_mut(&mut self, node_id: H256) -> Option<&mut PeerData> {
        let bucket = &mut self.buckets[bucket_number(node_id, self.local_node_id)];
        bucket
            .peers
            .iter_mut()
            .find(|entry| entry.node.node_id() == node_id)
    }

    /// Will try to insert a node into the table. If the table is full then it pushes it to the replacement list.
    /// # Returns
    /// A tuple containing:
    ///     1. PeerData: none if the peer was already in the table or as a potential replacement
    ///     2. A bool indicating if the node was inserted to the table
    pub fn insert_node(&mut self, node: Node) -> (Option<PeerData>, bool) {
        let bucket_idx = bucket_number(node.node_id(), self.local_node_id);

        self.insert_node_inner(node, bucket_idx, false)
    }

    /// Inserts a node into the table, even if the bucket is full.
    /// # Returns
    /// A tuple containing:
    ///     1. PeerData: none if the peer was already in the table or as a potential replacement
    ///     2. A bool indicating if the node was inserted to the table
    pub fn insert_node_forced(&mut self, node: Node) -> (Option<PeerData>, bool) {
        let bucket_idx = bucket_number(node.node_id(), self.local_node_id);

        self.insert_node_inner(node, bucket_idx, true)
    }

    #[cfg(test)]
    pub fn insert_node_on_custom_bucket(
        &mut self,
        node: Node,
        bucket_idx: usize,
    ) -> (Option<PeerData>, bool) {
        self.insert_node_inner(node, bucket_idx, false)
    }

    fn insert_node_inner(
        &mut self,
        node: Node,
        bucket_idx: usize,
        force_push: bool,
    ) -> (Option<PeerData>, bool) {
        let peer_already_in_table = self.buckets[bucket_idx]
            .peers
            .iter()
            .any(|p| p.node.node_id() == node.node_id());
        if peer_already_in_table {
            return (None, false);
        }
        let peer_already_in_replacements = self.buckets[bucket_idx]
            .replacements
            .iter()
            .any(|p| p.node.node_id() == node.node_id());
        if peer_already_in_replacements {
            return (None, false);
        }

        let peer = PeerData::new(node, NodeRecord::default(), false);

        // If bucket is full push to replacements. Unless forced
        if self.buckets[bucket_idx].peers.len() >= MAX_NODES_PER_BUCKET && !force_push {
            self.insert_as_replacement(&peer, bucket_idx);
            (Some(peer), false)
        } else {
            self.remove_from_replacements(peer.node.node_id(), bucket_idx);
            self.buckets[bucket_idx].peers.push(peer.clone());
            (Some(peer), true)
        }
    }

    fn insert_as_replacement(&mut self, node: &PeerData, bucket_idx: usize) {
        let bucket = &mut self.buckets[bucket_idx];
        if bucket.replacements.len() >= MAX_NUMBER_OF_REPLACEMENTS {
            bucket.replacements.remove(0);
        }
        bucket.replacements.push(node.clone());
    }

    fn remove_from_replacements(&mut self, node_id: H256, bucket_idx: usize) {
        let bucket = &mut self.buckets[bucket_idx];

        bucket.replacements = bucket
            .replacements
            .drain(..)
            .filter(|r| r.node.node_id() != node_id)
            .collect();
    }

    pub fn get_closest_nodes(&self, node_id: H256) -> Vec<Node> {
        let mut nodes: Vec<(Node, usize)> = vec![];

        // todo see if there is a more efficient way of doing this
        // though the bucket isn't that large and it shouldn't be an issue I guess
        for bucket in &self.buckets {
            for peer in &bucket.peers {
                let distance = bucket_number(node_id, peer.node.node_id());
                if nodes.len() < MAX_NODES_PER_BUCKET {
                    nodes.push((peer.node.clone(), distance));
                } else {
                    for (i, (_, dis)) in &mut nodes.iter().enumerate() {
                        if distance < *dis {
                            nodes[i] = (peer.node.clone(), distance);
                            break;
                        }
                    }
                }
            }
        }

        nodes.into_iter().map(|a| a.0).collect()
    }

    pub fn pong_answered(&mut self, node_id: H256, pong_at: u64) {
        let Some(peer) = self.get_by_node_id_mut(node_id) else {
            return;
        };

        peer.is_proven = true;
        peer.last_pong = pong_at;
        peer.last_ping_hash = None;
        peer.revalidation = peer.revalidation.and(Some(true));
    }

    pub fn update_peer_ping(&mut self, node_id: H256, ping_hash: Option<H256>, ping_at: u64) {
        let Some(peer) = self.get_by_node_id_mut(node_id) else {
            return;
        };

        peer.last_ping_hash = ping_hash;
        peer.last_ping = ping_at;
    }

    /// ## Returns
    /// The a vector of length of the provided `limit` of the peers who have the highest `last_ping` timestamp,
    /// that is, those peers that were pinged least recently. Careful with the `limit` param, as a
    /// it might get expensive.
    ///
    /// ## Dev note:
    /// This function should be improved:
    /// We might keep the `peers` list sorted by last_ping as we would avoid unnecessary loops
    pub fn get_least_recently_pinged_peers(&self, limit: usize) -> Vec<PeerData> {
        let mut peers = vec![];

        for bucket in &self.buckets {
            for peer in &bucket.peers {
                if peers.len() < limit {
                    peers.push(peer.clone());
                } else {
                    // replace the most recent from the list
                    let mut most_recent_index = 0;
                    for (i, other_peer) in peers.iter().enumerate() {
                        if other_peer.last_pong > peers[most_recent_index].last_pong {
                            most_recent_index = i;
                        }
                    }

                    if peer.last_pong < peers[most_recent_index].last_pong {
                        peers[most_recent_index] = peer.clone();
                    }
                }
            }
        }

        peers
    }

    /// Returns an iterator for all peers in the table
    pub fn iter_peers(&self) -> impl Iterator<Item = &PeerData> {
        self.buckets.iter().flat_map(|bucket| bucket.peers.iter())
    }

    /// Counts the number of connected peers
    pub fn count_connected_peers(&self) -> usize {
        self.filter_peers(&|peer| peer.is_connected).count()
    }

    /// Returns an iterator for all peers in the table that match the filter
    pub fn filter_peers<'a>(
        &'a self,
        filter: &'a dyn Fn(&'a PeerData) -> bool,
    ) -> impl Iterator<Item = &'a PeerData> {
        self.iter_peers().filter(|peer| filter(peer))
    }

    /// Select a peer with simple weighted selection based on scores
    fn get_peer_with_score_filter<'a>(
        &'a self,
        filter: &'a dyn Fn(&'a PeerData) -> bool,
    ) -> Option<&'a PeerData> {
        let filtered_peers: Vec<&PeerData> = self.filter_peers(filter).collect();

        if filtered_peers.is_empty() {
            return None;
        }

        // Simple weighted selection: convert scores to weights
        // Score -5 -> weight 1, Score 0 -> weight 6, Score 2 -> weight 8, etc.
        let weights: Vec<u32> = filtered_peers
            .iter()
            .map(|peer| (peer.score + 6).max(1) as u32)
            .collect();

        let total_weight: u32 = weights.iter().sum();
        if total_weight == 0 {
            // Fallback to random selection if somehow all weights are 0
            let peer_idx = random::<usize>() % filtered_peers.len();
            return filtered_peers.get(peer_idx).cloned();
        }

        // Weighted random selection using cumulative weights
        let random_value = random::<u32>() % total_weight;
        let mut cumulative_weight = 0u32;

        for (i, &weight) in weights.iter().enumerate() {
            cumulative_weight += weight;
            if random_value < cumulative_weight {
                return filtered_peers.get(i).cloned();
            }
        }

        // Fallback (should not reach here due to the total_weight check above)
        filtered_peers.last().cloned()
    }

    /// Replaces the peer with the given id with the latest replacement stored.
    /// If there are no replacements, it simply remove it
    ///
    /// # Returns
    ///
    /// A mutable reference to the inserted peer or None in case there was no replacement
    pub fn replace_peer(&mut self, node_id: H256) -> Option<PeerData> {
        let bucket_idx = bucket_number(self.local_node_id, node_id);
        self.replace_peer_inner(node_id, bucket_idx)
    }

    #[cfg(test)]
    pub fn replace_peer_on_custom_bucket(
        &mut self,
        node_id: H256,
        bucket_idx: usize,
    ) -> Option<PeerData> {
        self.replace_peer_inner(node_id, bucket_idx)
    }

    fn replace_peer_inner(&mut self, node_id: H256, bucket_idx: usize) -> Option<PeerData> {
        let idx_to_remove = self.buckets[bucket_idx]
            .peers
            .iter()
            .position(|peer| peer.node.node_id() == node_id);

        if let Some(idx) = idx_to_remove {
            let bucket = &mut self.buckets[bucket_idx];
            let new_peer = bucket.replacements.pop();

            if let Some(new_peer) = new_peer {
                bucket.peers[idx] = new_peer.clone();
                return Some(new_peer);
            } else {
                bucket.peers.remove(idx);
                return None;
            }
        };

        None
    }

    /// Sets the necessary data for the peer to be usable from the node's backend
    /// Set the sender end of the channel between the kademlia table and the peer's active connection
    /// Set the peer's supported capabilities
    /// This function should be called each time a connection is established so the backend can send requests to the peers
    /// Receives a boolean indicating if the connection is inbound (aka if it was started by the peer and not by this node)
    pub(crate) fn init_backend_communication(
        &mut self,
        node_id: H256,
        channels: PeerChannels,
        capabilities: Vec<Capability>,
        inbound: bool,
    ) {
        let peer = self.get_by_node_id_mut(node_id);
        if let Some(peer) = peer {
            peer.channels = Some(channels);
            peer.supported_capabilities = capabilities;
            peer.is_connected = true;
            peer.is_connection_inbound = inbound;
        } else {
            debug!(
                "[PEERS] Peer with node_id {:?} not found in the kademlia table when trying to init backend communication",
                node_id
            );
        }
    }

    /// Reward a peer for successful response
    pub fn reward_peer(&mut self, node_id: H256) {
        if let Some(peer) = self.get_by_node_id_mut(node_id) {
            peer.reward_peer();
        }
    }

    /// Penalize a peer for failed response or timeout
    pub fn penalize_peer(&mut self, node_id: H256) {
        if let Some(peer) = self.get_by_node_id_mut(node_id) {
            peer.penalize_peer(false);
        }
    }

    pub fn critically_penalize_peer(&mut self, node_id: H256) {
        if let Some(peer) = self.get_by_node_id_mut(node_id) {
            peer.penalize_peer(true);
        }
    }

    /// Returns the node id and channel ends to an active peer connection that supports the given capability
    /// The peer is selected using simple weighted selection based on scores (better peers more likely)
    pub fn get_peer_channels(&self, capabilities: &[Capability]) -> Option<(H256, PeerChannels)> {
        let filter = |peer: &PeerData| -> bool {
            // Search for peers with an active connection that support the required capabilities
            peer.channels.is_some()
                && capabilities
                    .iter()
                    .any(|cap| peer.supported_capabilities.contains(cap))
        };
        self.get_peer_with_score_filter(&filter).and_then(|peer| {
            peer.channels
                .clone()
                .map(|channel| (peer.node.node_id(), channel))
        })
    }
}

/// Computes the distance between two nodes according to the discv4 protocol
/// and returns the corresponding bucket number
/// <https://github.com/ethereum/devp2p/blob/master/discv4.md#node-identities>
pub fn bucket_number(node_id_1: H256, node_id_2: H256) -> usize {
    let xor = node_id_1 ^ node_id_2;
    let distance = U256::from_big_endian(xor.as_bytes());
    distance.bits().saturating_sub(1)
}

#[derive(Debug, Clone)]
pub struct PeerData {
    pub node: Node,
    pub record: NodeRecord,
    pub last_ping: u64,
    pub last_pong: u64,
    pub last_ping_hash: Option<H256>,
    pub is_proven: bool,
    pub find_node_request: Option<FindNodeRequest>,
    pub enr_request_hash: Option<H256>,
    pub supported_capabilities: Vec<Capability>,
    /// a ration to track the peers's ping responses
    pub liveness: u16,
    /// if a revalidation was sent to the peer, the bool marks if it has answered
    pub revalidation: Option<bool>,
    /// communication channels between the peer data and its active connection
    pub channels: Option<PeerChannels>,
    /// Starts as false when a node is added. Set to true when a connection becomes active. When a
    /// connection fails, the peer record is removed, so no need to set it to false.
    pub is_connected: bool,
    /// Set to true if the connection is inbound (aka the connection was started by the peer and not by this node)
    /// It is only valid as long as is_connected is true
    pub is_connection_inbound: bool,
    /// Simple peer score: +1 for success, -1 for failure
    pub score: i32,
}

impl PeerData {
    pub fn new(node: Node, record: NodeRecord, is_proven: bool) -> Self {
        Self {
            node,
            record,
            last_ping: 0,
            last_pong: 0,
            is_proven,
            liveness: 1,
            last_ping_hash: None,
            find_node_request: None,
            enr_request_hash: None,
            revalidation: None,
            channels: None,
            supported_capabilities: vec![],
            is_connected: false,
            is_connection_inbound: false,
            score: 0,
        }
    }

    #[allow(unused)]
    pub fn new_find_node_request(&mut self) {
        self.find_node_request = Some(FindNodeRequest::default());
    }

    pub fn new_find_node_request_with_sender(&mut self, sender: UnboundedSender<Vec<Node>>) {
        self.find_node_request = Some(FindNodeRequest::new_with_sender(sender));
    }

    pub fn increment_liveness(&mut self) {
        self.liveness += 1;
    }

    pub fn decrement_liveness(&mut self) {
        self.liveness /= 3;
    }

    /// Simple scoring: +1 for success
    pub fn reward_peer(&mut self) {
        if self.score < PEER_SCORE_UPPER_BOUND {
            self.score += 1;
        }

        debug!(
            "[PEERS] Rewarding peer with node_id {:?}, new score: {}",
            self.node.node_id(),
            self.score,
        );
    }

    /// Simple scoring: -5 for critical failure, -1 for non-critical
    pub fn penalize_peer(&mut self, critical: bool) {
        if self.score > PEER_SCORE_LOWER_BOUND {
            if critical {
                self.score -= 5;
            } else {
                self.score -= 1;
            };
        }

        debug!(
            "[PEERS] Penalizing peer with node_id {:?}, new score: {}",
            self.node.node_id(),
            self.score,
        );
    }
}

pub const MAX_MESSAGES_IN_PEER_CHANNEL: usize = 25;

#[derive(Debug, Clone)]
/// Holds the respective sender and receiver ends of the communication channels bewteen the peer data and its active connection
pub struct PeerChannels {
    pub(crate) connection: GenServerHandle<RLPxConnection>,
    pub(crate) receiver: Arc<Mutex<mpsc::Receiver<RLPxMessage>>>,
}

impl PeerChannels {
    /// Sets up the communication channels for the peer
    /// Returns the channel endpoints to send to the active connection's listen loop
    pub(crate) fn create(
        connection: GenServerHandle<RLPxConnection>,
    ) -> (Self, mpsc::Sender<RLPxMessage>) {
        let (connection_sender, receiver) =
            mpsc::channel::<RLPxMessage>(MAX_MESSAGES_IN_PEER_CHANNEL);
        (
            Self {
                connection,
                receiver: Arc::new(Mutex::new(receiver)),
            },
            connection_sender,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{network::public_key_from_signing_key, rlpx::utils::node_id};

    use super::*;
    use ethrex_common::H512;
    use hex_literal::hex;
    use k256::{ecdsa::SigningKey, elliptic_curve::rand_core::OsRng};
    use std::{
        net::{IpAddr, Ipv4Addr},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn bucket_number_works_as_expected() {
        let public_key_1 = H512(hex!(
            "4dc429669029ceb17d6438a35c80c29e09ca2c25cc810d690f5ee690aa322274043a504b8d42740079c4f4cef50777c991010208b333b80bee7b9ae8e5f6b6f0"
        ));
        let public_key_2 = H512(hex!(
            "034ee575a025a661e19f8cda2b6fd8b2fd4fe062f6f2f75f0ec3447e23c1bb59beb1e91b2337b264c7386150b24b621b8224180c9e4aaf3e00584402dc4a8386"
        ));
        let node_id_1 = node_id(&public_key_1);
        let node_id_2 = node_id(&public_key_2);
        let expected_bucket = 255;
        let result = bucket_number(node_id_1, node_id_2);
        assert_eq!(result, expected_bucket);
    }

    fn insert_random_node_on_custom_bucket(
        table: &mut KademliaTable,
        bucket_idx: usize,
    ) -> (Option<PeerData>, bool) {
        let public_key = public_key_from_signing_key(&SigningKey::random(&mut OsRng));
        let node = Node::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0, 0, public_key);
        table.insert_node_on_custom_bucket(node, bucket_idx)
    }

    fn fill_table_with_random_nodes(table: &mut KademliaTable) {
        for i in 0..256 {
            for _ in 0..16 {
                insert_random_node_on_custom_bucket(table, i);
            }
        }
    }

    fn get_test_table() -> KademliaTable {
        let signer = SigningKey::random(&mut OsRng);
        let local_public_key = public_key_from_signing_key(&signer);
        let local_node_id = node_id(&local_public_key);

        KademliaTable::new(local_node_id)
    }

    #[test]
    fn get_least_recently_pinged_peers_should_return_the_right_peers() {
        let mut table = get_test_table();
        let node_1_pubkey = public_key_from_signing_key(&SigningKey::random(&mut OsRng));
        {
            table.insert_node(Node::new(
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                0,
                0,
                node_1_pubkey,
            ));
            let node_1_id = node_id(&node_1_pubkey);
            table.get_by_node_id_mut(node_1_id).unwrap().last_pong = (SystemTime::now()
                - Duration::from_secs(12 * 60 * 60))
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        }

        let node_2_pubkey = public_key_from_signing_key(&SigningKey::random(&mut OsRng));
        {
            table.insert_node(Node::new(
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                0,
                0,
                node_2_pubkey,
            ));
            let node_2_id = node_id(&node_2_pubkey);
            table.get_by_node_id_mut(node_2_id).unwrap().last_pong = (SystemTime::now()
                - Duration::from_secs(36 * 60 * 60))
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        }

        let node_3_pubkey = public_key_from_signing_key(&SigningKey::random(&mut OsRng));
        {
            table.insert_node(Node::new(
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                0,
                0,
                node_3_pubkey,
            ));
            let node_3_id = node_id(&node_3_pubkey);
            table.get_by_node_id_mut(node_3_id).unwrap().last_pong = (SystemTime::now()
                - Duration::from_secs(10 * 60 * 60))
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        }

        // we expect the node_1 & node_2 to be returned here
        let peers: Vec<H512> = table
            .get_least_recently_pinged_peers(2)
            .iter()
            .map(|p| p.node.public_key)
            .collect();

        assert!(peers.contains(&node_1_pubkey));
        assert!(peers.contains(&node_2_pubkey));
        assert!(!peers.contains(&node_3_pubkey));
    }

    #[test]
    fn insert_peer_should_remove_first_replacement_when_list_is_full() {
        let mut table = get_test_table();
        fill_table_with_random_nodes(&mut table);
        let bucket_idx = 0;

        let (first_node, inserted_to_table) =
            insert_random_node_on_custom_bucket(&mut table, bucket_idx);
        let first_node = first_node.unwrap();
        assert!(!inserted_to_table);

        // here we are forcingly pushing to the first bucket, that is, the distance might
        // not be in accordance with the bucket index
        // but we don't care about that here, we just want to check if the replacement works as expected
        for _ in 1..MAX_NUMBER_OF_REPLACEMENTS {
            let (_, inserted_to_table) =
                insert_random_node_on_custom_bucket(&mut table, bucket_idx);
            assert!(!inserted_to_table);
        }

        {
            let bucket = &table.buckets[bucket_idx];
            assert_eq!(
                first_node.node.public_key,
                bucket.replacements[0].node.public_key
            );
        }

        // push one more element, this should replace the first one pushed
        let (last, inserted_to_table) = insert_random_node_on_custom_bucket(&mut table, bucket_idx);
        let last = last.unwrap();
        assert!(!inserted_to_table);

        let bucket = &table.buckets[bucket_idx];
        assert_ne!(
            first_node.node.public_key,
            bucket.replacements[0].node.public_key
        );
        assert_eq!(
            last.node.public_key,
            bucket.replacements[MAX_NUMBER_OF_REPLACEMENTS - 1]
                .node
                .public_key
        );
    }

    #[test]
    fn replace_peer_should_replace_peer() {
        let mut table = get_test_table();
        let bucket_idx = 0;
        fill_table_with_random_nodes(&mut table);

        let (replacement_peer, inserted_to_table) =
            insert_random_node_on_custom_bucket(&mut table, bucket_idx);
        let replacement_peer = replacement_peer.unwrap();
        assert!(!inserted_to_table);

        let node_id_to_replace = table.buckets[bucket_idx].peers[0].node.node_id();
        let replacement = table.replace_peer_on_custom_bucket(node_id_to_replace, bucket_idx);

        assert_eq!(
            replacement.unwrap().node.node_id(),
            replacement_peer.node.node_id()
        );
        assert_eq!(
            table.buckets[bucket_idx].peers[0].node.node_id(),
            replacement_peer.node.node_id()
        );
    }
    #[test]
    fn replace_peer_should_remove_peer_but_not_replace() {
        // here, we will remove the peer, but with no replacements peers available
        let mut table = get_test_table();
        let bucket_idx = 0;
        fill_table_with_random_nodes(&mut table);

        let node_id_to_replace = table.buckets[bucket_idx].peers[0].node.node_id();
        let len_before = table.buckets[bucket_idx].peers.len();
        let replacement = table.replace_peer_on_custom_bucket(node_id_to_replace, bucket_idx);
        let len_after = table.buckets[bucket_idx].peers.len();

        assert!(replacement.is_none());
        assert!(len_before - 1 == len_after);
    }

    #[test]
    fn test_peer_scoring_system() {
        let mut table = get_test_table();

        // Initialization and basic scoring operations
        let public_key = public_key_from_signing_key(&SigningKey::random(&mut OsRng));
        let node = Node::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0, 0, public_key);
        table.insert_node(node);
        let first_node_id = node_id(&public_key);

        // New peers start with score 0
        assert_eq!(table.get_by_node_id(first_node_id).unwrap().score, 0);

        // Test rewards and penalties
        table.reward_peer(first_node_id);
        table.reward_peer(first_node_id);
        assert_eq!(table.get_by_node_id(first_node_id).unwrap().score, 2);

        table.penalize_peer(first_node_id);
        assert_eq!(table.get_by_node_id(first_node_id).unwrap().score, 1);

        // Edge cases and weight calculation
        // Very negative score
        for _ in 0..3 {
            table.critically_penalize_peer(first_node_id);
        }
        assert_eq!(table.get_by_node_id(first_node_id).unwrap().score, -14);

        // Very positive score
        for _ in 0..20 {
            table.reward_peer(first_node_id);
        }
        assert_eq!(table.get_by_node_id(first_node_id).unwrap().score, 6);

        // Weighted selection with multiple peers
        let peer_keys: Vec<_> = (0..3)
            .map(|_| public_key_from_signing_key(&SigningKey::random(&mut OsRng)))
            .collect();
        let mut peer_ids = Vec::new();

        let mut table = get_test_table();

        for key in &peer_keys {
            let node = Node::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0, 0, *key);
            table.insert_node(node);
            peer_ids.push(node_id(key));
        }

        // Set different scores: -2, 0, 3
        table.penalize_peer(peer_ids[0]);
        table.penalize_peer(peer_ids[0]);
        table.reward_peer(peer_ids[2]);
        table.reward_peer(peer_ids[2]);
        table.reward_peer(peer_ids[2]);

        assert_eq!(table.get_by_node_id(peer_ids[0]).unwrap().score, -2);
        assert_eq!(table.get_by_node_id(peer_ids[1]).unwrap().score, 0);
        assert_eq!(table.get_by_node_id(peer_ids[2]).unwrap().score, 3);

        // Test weighted selection distribution
        let mut selection_counts = [0; 3];
        for _ in 0..1000 {
            if let Some(selected) = table.get_peer_with_score_filter(&|_| true) {
                for (i, &peer_id) in peer_ids.iter().enumerate() {
                    if selected.node.node_id() == peer_id {
                        selection_counts[i] += 1;
                        break;
                    }
                }
            }
        }

        // Higher scoring peers should be selected more often
        assert!(selection_counts[0] < selection_counts[1]); // -2 < 0
        assert!(selection_counts[1] < selection_counts[2]); // 0 < 3
        assert!(selection_counts[0] > 0); // No complete exclusion

        // Edge cases
        // Non-existent peer should not panic
        table.reward_peer(H256::random());
        table.penalize_peer(H256::random());

        // Empty table should return None
        let empty_table = get_test_table();
        assert!(empty_table.get_peer_with_score_filter(&|_| true).is_none());
    }
}
