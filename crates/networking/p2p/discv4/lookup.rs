use super::{helpers::get_expiration, DiscoveryError, Message};
use crate::{
    kademlia::{bucket_number, MAX_NODES_PER_BUCKET},
    node_id_from_signing_key,
    types::Node,
    KademliaTable,
};
use ethrex_core::H512;
use k256::ecdsa::SigningKey;
use rand::rngs::OsRng;
use std::{collections::HashSet, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::UdpSocket, sync::Mutex};
use tokio_util::task::TaskTracker;
use tracing::debug;

/// Starts a tokio scheduler that:
/// - performs random lookups to discover new nodes. Currently this is configure to run every `PEERS_RANDOM_LOOKUP_TIME_IN_MIN`
///
/// **Random lookups**
///
/// Random lookups work in the following manner:
/// 1. Every 30min we spawn three concurrent lookups: one closest to our pubkey
///    and three other closest to random generated pubkeys.
/// 2. Every lookup starts with the closest nodes from our table.
///    Each lookup keeps track of:
///    - Peers that have already been asked for nodes
///    - Peers that have been already seen
///    - Potential peers to query for nodes: a vector of up to 16 entries holding the closest peers to the pubkey.
///      This vector is initially filled with nodes from our table.
/// 3. We send a `find_node` to the closest 3 nodes (that we have not yet asked) from the pubkey.
/// 4. We wait for the neighbors response and pushed or replace those that are closer to the potential peers.
/// 5. We select three other nodes from the potential peers vector and do the same until one lookup
///    doesn't have any node to ask.
///
/// See more https://github.com/ethereum/devp2p/blob/master/discv4.md#recursive-lookup
#[derive(Clone, Debug)]
pub struct Disv4LookupHandler {
    local_node: Node,
    signer: SigningKey,
    udp_socket: Arc<UdpSocket>,
    table: Arc<Mutex<KademliaTable>>,
    interval_minutes: u64,
    tracker: TaskTracker,
}

impl Disv4LookupHandler {
    pub fn new(
        local_node: Node,
        signer: SigningKey,
        udp_socket: Arc<UdpSocket>,
        table: Arc<Mutex<KademliaTable>>,
        interval_minutes: u64,
        tracker: TaskTracker,
    ) -> Self {
        Self {
            local_node,
            signer,
            udp_socket,
            table,
            interval_minutes,
            tracker,
        }
    }

    pub async fn start(&self, initial_interval_wait_seconds: u64) {
        let mut interval = tokio::time::interval(Duration::from_secs(self.interval_minutes));
        tokio::time::sleep(Duration::from_secs(initial_interval_wait_seconds)).await;

        loop {
            // Notice that the first tick is immediate,
            // so as soon as the server starts we'll do a lookup with the seeder nodes.
            interval.tick().await;

            debug!("Starting lookup");

            self.tracker.spawn({
                let self_clone = self.clone();
                async move {
                    self_clone
                        .recursive_lookup(self_clone.local_node.node_id)
                        .await
                }
            });

            // lookup closest to 3 random keys
            for _ in 0..3 {
                let random_pub_key = SigningKey::random(&mut OsRng);
                self.tracker.spawn({
                    let self_clone = self.clone();
                    async move {
                        self_clone
                            .recursive_lookup(node_id_from_signing_key(&random_pub_key))
                            .await
                    }
                });
            }

            debug!("Lookup finished");
        }
    }

    async fn recursive_lookup(&self, target: H512) {
        let mut asked_peers = HashSet::default();
        // lookups start with the closest from our table
        let closest_nodes = self.table.lock().await.get_closest_nodes(target);
        let mut seen_peers: HashSet<H512> = HashSet::default();

        seen_peers.insert(self.local_node.node_id);
        for node in &closest_nodes {
            seen_peers.insert(node.node_id);
        }

        let mut peers_to_ask: Vec<Node> = closest_nodes;

        loop {
            let (nodes_found, queries) = self
                .clone()
                .lookup(target, &mut asked_peers, &peers_to_ask)
                .await;

            // only push the peers that have not been seen
            // that is those who have not been yet pushed, which also accounts for
            // those peers that were in the array but have been replaced for closer peers
            for node in nodes_found {
                if !seen_peers.contains(&node.node_id) {
                    seen_peers.insert(node.node_id);
                    self.peers_to_ask_push(&mut peers_to_ask, target, node);
                }
            }

            // the lookup finishes when there are no more queries to do
            // that happens when we have asked all the peers
            if queries == 0 {
                break;
            }
        }
    }

    async fn lookup(
        &self,
        target: H512,
        asked_peers: &mut HashSet<H512>,
        nodes_to_ask: &Vec<Node>,
    ) -> (Vec<Node>, u32) {
        // ask FIND_NODE as much as three times
        let alpha = 3;
        let mut queries = 0;
        let mut nodes = vec![];

        for node in nodes_to_ask {
            if !asked_peers.contains(&node.node_id) {
                #[allow(unused_assignments)]
                let mut rx = None;
                {
                    let mut table = self.table.lock().await;
                    let peer = table.get_by_node_id_mut(node.node_id);
                    if let Some(peer) = peer {
                        // if the peer has an ongoing find_node request, don't query
                        if peer.find_node_request.is_some() {
                            continue;
                        }
                        let (tx, receiver) = tokio::sync::mpsc::unbounded_channel::<Vec<Node>>();
                        peer.new_find_node_request_with_sender(tx);
                        rx = Some(receiver);
                    } else {
                        // if peer isn't inserted to table, don't query
                        continue;
                    }
                }

                queries += 1;
                asked_peers.insert(node.node_id);

                if let Ok(mut found_nodes) = self
                    .find_node_and_wait_for_response(*node, target, &mut rx.unwrap())
                    .await
                {
                    nodes.append(&mut found_nodes);
                }
            }

            if queries == alpha {
                break;
            }
        }

        (nodes, queries)
    }

    /**
     * TODO explain what this does
     */
    fn peers_to_ask_push(&self, peers_to_ask: &mut Vec<Node>, target: H512, node: Node) {
        let distance = bucket_number(target, node.node_id);

        if peers_to_ask.len() < MAX_NODES_PER_BUCKET {
            peers_to_ask.push(node);
            return;
        }

        // replace this node for the one whose distance to the target is the highest
        let (mut idx_to_replace, mut highest_distance) = (None, 0);

        for (i, peer) in peers_to_ask.iter().enumerate() {
            let current_distance = bucket_number(peer.node_id, target);

            if distance < current_distance && current_distance >= highest_distance {
                highest_distance = current_distance;
                idx_to_replace = Some(i);
            }
        }

        if let Some(idx) = idx_to_replace {
            peers_to_ask[idx] = node;
        }
    }

    async fn find_node_and_wait_for_response(
        &self,
        node: Node,
        target_id: H512,
        request_receiver: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<Node>>,
    ) -> Result<Vec<Node>, DiscoveryError> {
        let expiration: u64 = get_expiration(20);

        let msg = Message::FindNode(super::FindNodeMessage::new(target_id, expiration));

        let mut buf = Vec::new();
        msg.encode_with_header(&mut buf, &self.signer);
        let bytes_sent = self
            .udp_socket
            .send_to(&buf, SocketAddr::new(node.ip, node.udp_port))
            .await
            .map_err(|e| DiscoveryError::MessageSendFailure(e))?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryError::PartialMessageSent);
        }

        let mut nodes = vec![];
        loop {
            // wait as much as 5 seconds for the response
            match tokio::time::timeout(Duration::from_secs(5), request_receiver.recv()).await {
                Ok(Some(mut found_nodes)) => {
                    nodes.append(&mut found_nodes);
                    if nodes.len() == MAX_NODES_PER_BUCKET {
                        return Ok(nodes);
                    };
                }
                Ok(None) => {
                    return Ok(nodes);
                }
                Err(_) => {
                    // timeout expired
                    return Ok(nodes);
                }
            }
        }
    }
}
