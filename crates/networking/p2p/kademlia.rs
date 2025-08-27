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
pub const MAX_NODES_PER_BUCKET: u64 = 16;
const NUMBER_OF_BUCKETS: u32 = 256;
const MAX_NUMBER_OF_REPLACEMENTS: u32 = 10;

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
        }
    }
}

impl KademliaTable {
    pub fn new(local_node_id: H256) -> Self {
        let buckets: Vec<Bucket> = vec![Bucket::default(); NUMBER_OF_BUCKETS as usize];
        Self {
            node,
            validation_timestamp: None,
            ping_hash: None,
            n_find_node_sent: 0,
            disposable: false,
            knows_us: true,
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
        if self.buckets[bucket_idx].peers.len() as u64 >= MAX_NODES_PER_BUCKET && !force_push {
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
        if bucket.replacements.len() >= MAX_NUMBER_OF_REPLACEMENTS as usize {
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
                if (nodes.len() as u64) < MAX_NODES_PER_BUCKET {
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
            .map(|peer| (peer.score + 6).max(1).expect("Score should never be negative 💀💀💀") as u32)
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
    pub record: Option<NodeRecord>,
    pub supported_capabilities: Vec<Capability>,
    /// Set to true if the connection is inbound (aka the connection was started by the peer and not by this node)
    /// It is only valid as long as is_connected is true
    pub is_connection_inbound: bool,
    /// communication channels between the peer data and its active connection
    pub channels: Option<PeerChannels>,
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
        debug!("New peer connected");

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

#[cfg(test)]
mod tests {
    use crate::{network::public_key_from_signing_key, rlpx::utils::node_id};

    use super::*;
    use ethrex_common::H512;
    use hex_literal::hex;
    use rand::rngs::OsRng;
    use secp256k1::SecretKey;
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
        let public_key = public_key_from_signing_key(&SecretKey::new(&mut OsRng));
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
        let signer = SecretKey::new(&mut OsRng);
        let local_public_key = public_key_from_signing_key(&signer);
        let local_node_id = node_id(&local_public_key);

        KademliaTable::new(local_node_id)
    }

    #[test]
    fn get_least_recently_pinged_peers_should_return_the_right_peers() {
        let mut table = get_test_table();
        let node_1_pubkey = public_key_from_signing_key(&SecretKey::new(&mut OsRng));
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

        let node_2_pubkey = public_key_from_signing_key(&SecretKey::new(&mut OsRng));
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

        let node_3_pubkey = public_key_from_signing_key(&SecretKey::new(&mut OsRng));
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
            bucket.replacements[MAX_NUMBER_OF_REPLACEMENTS as usize - 1]
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
        let public_key = public_key_from_signing_key(&SecretKey::new(&mut OsRng));
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
            .map(|_| public_key_from_signing_key(&SecretKey::new(&mut OsRng)))
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
