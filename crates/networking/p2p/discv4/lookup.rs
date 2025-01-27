use super::{helpers::get_expiration, DiscoveryError, Message};
use crate::{
    kademlia::{bucket_number, MAX_NODES_PER_BUCKET},
    node_id_from_signing_key,
    types::Node,
    P2PContext,
};
use ethrex_core::H512;
use k256::ecdsa::SigningKey;
use rand::rngs::OsRng;
use std::{collections::HashSet, net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::UdpSocket;
use tracing::debug;

#[derive(Clone, Debug)]
pub struct Discv4LookupHandler {
    ctx: P2PContext,
    udp_socket: Arc<UdpSocket>,
    interval_minutes: u64,
}

impl Discv4LookupHandler {
    pub fn new(ctx: P2PContext, udp_socket: Arc<UdpSocket>, interval_minutes: u64) -> Self {
        Self {
            ctx,
            udp_socket,
            interval_minutes,
        }
    }

    /// Starts a tokio scheduler that:
    /// - performs random lookups to discover new nodes.
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
    /// 4. We wait for the neighbors response and push or replace those that are closer to the potential peers array.
    /// 5. We select three other nodes from the potential peers vector and do the same until one lookup
    ///    doesn't have any node to ask.
    ///
    /// See more https://github.com/ethereum/devp2p/blob/master/discv4.md#recursive-lookup
    pub fn start(&self, initial_interval_wait_seconds: u64) {
        self.ctx.tracker.spawn({
            let self_clone = self.clone();
            async move {
                self_clone
                    .start_lookup_loop(initial_interval_wait_seconds)
                    .await;
            }
        });
    }

    async fn start_lookup_loop(&self, initial_interval_wait_seconds: u64) {
        let mut interval = tokio::time::interval(Duration::from_secs(self.interval_minutes));
        tokio::time::sleep(Duration::from_secs(initial_interval_wait_seconds)).await;

        loop {
            // first tick is immediate,
            interval.tick().await;

            debug!("Starting lookup");

            // lookup closest to our node_id
            self.ctx.tracker.spawn({
                let self_clone = self.clone();
                async move {
                    self_clone
                        .recursive_lookup(self_clone.ctx.local_node.node_id)
                        .await
                }
            });

            // lookup closest to 3 random keys
            for _ in 0..3 {
                let random_pub_key = SigningKey::random(&mut OsRng);
                self.ctx.tracker.spawn({
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
        // lookups start with the closest nodes to the target from our table
        let mut peers_to_ask: Vec<Node> = self.ctx.table.lock().await.get_closest_nodes(target);
        // stores the peers in peers_to_ask + the peers that were in peers_to_ask but were replaced by closer targets
        let mut seen_peers: HashSet<H512> = HashSet::default();
        let mut asked_peers = HashSet::default();

        seen_peers.insert(self.ctx.local_node.node_id);
        for node in &peers_to_ask {
            seen_peers.insert(node.node_id);
        }

        loop {
            let (nodes_found, queries) = self.lookup(target, &mut asked_peers, &peers_to_ask).await;

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
        // send FIND_NODE as much as three times
        let alpha = 3;
        let mut queries = 0;
        let mut nodes = vec![];

        for node in nodes_to_ask {
            if asked_peers.contains(&node.node_id) {
                continue;
            }
            let mut locked_table = self.ctx.table.lock().await;
            if let Some(peer) = locked_table.get_by_node_id_mut(node.node_id) {
                // if the peer has an ongoing find_node request, don't query
                if peer.find_node_request.is_none() {
                    let (tx, mut receiver) = tokio::sync::mpsc::unbounded_channel::<Vec<Node>>();
                    peer.new_find_node_request_with_sender(tx);

                    // Release the lock
                    drop(locked_table);

                    queries += 1;
                    asked_peers.insert(node.node_id);
                    if let Ok(mut found_nodes) = self
                        .find_node_and_wait_for_response(*node, target, &mut receiver)
                        .await
                    {
                        nodes.append(&mut found_nodes);
                    }

                    if let Some(peer) = self.ctx.table.lock().await.get_by_node_id_mut(node.node_id)
                    {
                        peer.find_node_request = None;
                    };
                }
            }

            if queries == alpha {
                break;
            }
        }

        (nodes, queries)
    }

    /// Adds a node to `peers_to_ask` if there's space; otherwise, replaces the farthest node
    /// from `target` if the new node is closer.
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
        msg.encode_with_header(&mut buf, &self.ctx.signer);
        let bytes_sent = self
            .udp_socket
            .send_to(&buf, SocketAddr::new(node.ip, node.udp_port))
            .await
            .map_err(DiscoveryError::MessageSendFailure)?;

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

#[cfg(test)]
mod tests {
    use tokio::time::sleep;

    use super::*;
    use crate::discv4::{
        tests::{
            connect_servers, fill_table_with_random_nodes, insert_random_node_on_custom_bucket,
            start_discovery_server,
        },
        Discv4Server,
    };

    fn lookup_handler_from_server(server: Discv4Server) -> Discv4LookupHandler {
        Discv4LookupHandler::new(
            server.ctx.clone(),
            server.udp_socket.clone(),
            server.lookup_interval_minutes,
        )
    }

    #[tokio::test]
    /** This test tests the lookup function, the idea is as follows:
     * - We'll start two discovery servers (`a` & `b`) that will connect between each other
     * - We'll insert random nodes to the server `a`` to fill its table
     * - We'll forcedly run `lookup` and validate that a `find_node` request was sent
     *   by checking that new nodes have been inserted to the table
     *
     * This test for only one lookup, and not recursively.
     */
    async fn discovery_server_lookup() -> Result<(), DiscoveryError> {
        let mut server_a = start_discovery_server(8000, true).await?;
        let mut server_b = start_discovery_server(8001, true).await?;

        fill_table_with_random_nodes(server_a.ctx.table.clone()).await;

        // because the table is filled, before making the connection, remove a node from the `b` bucket
        // otherwise it won't be added.
        let b_bucket = bucket_number(
            server_a.ctx.local_node.node_id,
            server_b.ctx.local_node.node_id,
        );
        let node_id_to_remove = server_a.ctx.table.lock().await.buckets()[b_bucket].peers[0]
            .node
            .node_id;
        server_a
            .ctx
            .table
            .lock()
            .await
            .replace_peer_on_custom_bucket(node_id_to_remove, b_bucket);

        connect_servers(&mut server_a, &mut server_b).await?;

        // now we are going to run a lookup with us as the target
        let closets_peers_to_b_from_a = server_a
            .ctx
            .table
            .lock()
            .await
            .get_closest_nodes(server_b.ctx.local_node.node_id);
        let nodes_to_ask = server_b
            .ctx
            .table
            .lock()
            .await
            .get_closest_nodes(server_b.ctx.local_node.node_id);

        let lookup_handler = lookup_handler_from_server(server_b.clone());
        lookup_handler
            .lookup(
                server_b.ctx.local_node.node_id,
                &mut HashSet::default(),
                &nodes_to_ask,
            )
            .await;

        // find_node sent, allow some time for `a` to respond
        sleep(Duration::from_secs(2)).await;

        // now all peers should've been inserted
        for peer in closets_peers_to_b_from_a {
            let table = server_b.ctx.table.lock().await;
            let node = table.get_by_node_id(peer.node_id);
            // sometimes nodes can send ourselves as a neighbor
            // make sure we don't add it
            if peer.node_id == server_b.ctx.local_node.node_id {
                assert!(node.is_none());
            } else {
                assert!(node.is_some());
            }
        }
        Ok(())
    }

    #[tokio::test]
    /** This test tests the lookup function, the idea is as follows:
     * - We'll start four discovery servers (`a`, `b`, `c` & `d`)
     * - `a` will be connected to `b`, `b` will be connected to `c` and `c` will be connected to `d`.
     * - The server `d` will have its table filled with mock nodes
     * - We'll run a recursive lookup on server `a` and we expect to end with `b`, `c`, `d` and its mock nodes
     */
    async fn discovery_server_recursive_lookup() -> Result<(), DiscoveryError> {
        let mut server_a = start_discovery_server(8002, true).await?;
        let mut server_b = start_discovery_server(8003, true).await?;
        let mut server_c = start_discovery_server(8004, true).await?;
        let mut server_d = start_discovery_server(8005, true).await?;

        connect_servers(&mut server_a, &mut server_b).await?;
        connect_servers(&mut server_b, &mut server_c).await?;
        connect_servers(&mut server_c, &mut server_d).await?;

        // now we fill the server_d table with 3 random nodes
        // the reason we don't put more is because this nodes won't respond (as they don't are not real servers)
        // and so we will have to wait for the timeout on each node, which will only slow down the test
        for _ in 0..3 {
            insert_random_node_on_custom_bucket(server_d.ctx.table.clone(), 0).await;
        }

        let mut expected_peers = vec![];
        expected_peers.extend(
            server_b
                .ctx
                .table
                .lock()
                .await
                .get_closest_nodes(server_a.ctx.local_node.node_id),
        );
        expected_peers.extend(
            server_c
                .ctx
                .table
                .lock()
                .await
                .get_closest_nodes(server_a.ctx.local_node.node_id),
        );
        expected_peers.extend(
            server_d
                .ctx
                .table
                .lock()
                .await
                .get_closest_nodes(server_a.ctx.local_node.node_id),
        );

        let lookup_handler = lookup_handler_from_server(server_a.clone());

        // we'll run a recursive lookup closest to the server itself
        lookup_handler
            .recursive_lookup(server_a.ctx.local_node.node_id)
            .await;

        // sometimes nodes can send ourselves as a neighbor
        // make sure we don't add it
        for peer in expected_peers {
            let table = server_a.ctx.table.lock().await;
            let node = table.get_by_node_id(peer.node_id);

            if peer.node_id == server_a.ctx.local_node.node_id {
                assert!(node.is_none());
            } else {
                assert!(node.is_some());
            }
        }

        Ok(())
    }
}
