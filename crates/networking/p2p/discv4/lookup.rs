use std::{collections::HashSet, net::SocketAddr, sync::Arc, time::Duration};

use ethrex_core::H512;
use k256::ecdsa::SigningKey;
use rand::rngs::OsRng;
use tokio::{net::UdpSocket, sync::Mutex, try_join};
use tracing::debug;

use crate::{
    kademlia::{bucket_number, MAX_NODES_PER_BUCKET},
    node_id_from_signing_key,
    types::Node,
    KademliaTable,
};

use super::requests::find_node_and_wait_for_response;

const PEERS_RANDOM_LOOKUP_TIME_IN_MIN: u64 = 30; // same as above

#[derive(Clone, Debug)]
pub struct DiscoveryLookupHandler {
    local_node: Node,
    signer: SigningKey,
    udp_socket: Arc<UdpSocket>,
    table: Arc<Mutex<KademliaTable>>,
    lookup_interval_minutes: u64,
    seen_peers: HashSet<H512>,
    asked_peers: HashSet<H512>,
}

impl DiscoveryLookupHandler {
    pub fn new(
        local_node: Node,
        signer: SigningKey,
        udp_socket: Arc<UdpSocket>,
        table: Arc<Mutex<KademliaTable>>,
    ) -> Self {
        Self {
            local_node,
            signer,
            udp_socket,
            table,
            lookup_interval_minutes: PEERS_RANDOM_LOOKUP_TIME_IN_MIN,
            seen_peers: HashSet::new(),
            asked_peers: HashSet::new(),
        }
    }

    pub async fn start_lookup_task(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(self.lookup_interval_minutes));

        loop {
            // Notice that the first tick is immediate,
            // so as soon as the server starts we'll do a lookup with the seeder nodes.
            interval.tick().await;

            debug!("Starting lookup");

            let mut handlers = vec![];

            // lookup closest to our pub key
            let self_clone = self.clone();
            handlers.push(tokio::spawn(async move {
                self_clone
                    .recursive_lookup(self_clone.local_node.node_id)
                    .await;
            }));

            // lookup closest to 3 random keys
            for _ in 0..3 {
                let random_pub_key = SigningKey::random(&mut OsRng);
                let self_clone = self.clone();
                handlers.push(tokio::spawn(async move {
                    self_clone
                        .recursive_lookup(node_id_from_signing_key(&random_pub_key))
                        .await
                }))
            }

            for handle in handlers {
                let _ = try_join!(handle);
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
            let (nodes_found, queries) = self.lookup(target, &mut asked_peers, &peers_to_ask).await;

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

                let mut found_nodes = find_node_and_wait_for_response(
                    &self.udp_socket,
                    SocketAddr::new(node.ip, node.udp_port),
                    &self.signer,
                    target,
                    &mut rx.unwrap(),
                )
                .await;
                nodes.append(&mut found_nodes);
            }

            if queries == alpha {
                break;
            }
        }

        (nodes, queries)
    }
}
