use crate::{
    bootnode::BootNode,
    discv4::{
        helpers::{get_expiration, is_expired, time_now_unix, time_since_in_hs},
        messages::{Message, NeighborsMessage, Packet},
        requests::{ping, pong},
    },
    handle_peer_as_initiator,
    kademlia::MAX_NODES_PER_BUCKET,
    rlpx::message::Message as RLPxMessage,
    types::Node,
    KademliaTable, MAX_DISC_PACKET_SIZE,
};
use ethrex_core::H512;
use ethrex_storage::Store;
use futures::try_join;
use k256::ecdsa::SigningKey;
use std::{collections::HashSet, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::UdpSocket,
    sync::{broadcast, Mutex},
};
use tracing::debug;

use super::lookup::DiscoveryLookupHandler;

#[derive(Debug, Clone)]
pub struct Discv4 {
    local_node: Node,
    udp_addr: SocketAddr,
    udp_socket: Arc<UdpSocket>,
    signer: SigningKey,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    tx_broadcaster_send: broadcast::Sender<(tokio::task::Id, Arc<RLPxMessage>)>,
    revalidation_interval_seconds: u64,
}

pub enum DiscoveryError {
    UnexpectedError,
}

const REVALIDATION_INTERVAL_IN_SECONDS: u64 = 30; // this is just an arbitrary number, maybe we should get this from a cfg or cli param
const PROOF_EXPIRATION_IN_HS: u16 = 12;

impl Discv4 {
    pub fn new(
        local_node: Node,
        signer: SigningKey,
        storage: Store,
        table: Arc<Mutex<KademliaTable>>,
        tx_broadcaster_send: broadcast::Sender<(tokio::task::Id, Arc<RLPxMessage>)>,
        udp_socket: Arc<UdpSocket>,
    ) -> Self {
        Self {
            local_node,
            signer,
            storage,
            table,
            tx_broadcaster_send,
            udp_addr: SocketAddr::new(local_node.ip, local_node.udp_port),
            udp_socket,
            revalidation_interval_seconds: REVALIDATION_INTERVAL_IN_SECONDS,
        }
    }

    pub fn with_revalidation_interval_of(self, seconds: u64) -> Self {
        Self {
            revalidation_interval_seconds: seconds,
            ..self
        }
    }

    pub fn with_lookup_interval_of(self, minutes: u64) -> Self {
        Self {
            revalidation_interval_seconds: minutes,
            ..self
        }
    }

    pub async fn start_discovery_service(
        self: Arc<Self>,
        bootnodes: Vec<BootNode>,
    ) -> Result<(), DiscoveryError> {
        let server_handle = tokio::spawn(self.clone().receive());
        self.load_bootnodes(bootnodes).await;

        let revalidation_handle = tokio::spawn(self.clone().start_revalidation_task());

        // a first initial lookup runs without waiting for the interval
        // so we need to allow some time to the pinged peers to ping us back and acknowledge us
        let lookup_handler = DiscoveryLookupHandler::new(
            self.local_node,
            self.signer.clone(),
            self.udp_socket.clone(),
            self.table.clone(),
        );
        let lookup_handle = tokio::spawn(async move { lookup_handler.start_lookup_task().await });

        let result = try_join!(server_handle, revalidation_handle, lookup_handle);

        if result.is_ok() {
            Ok(())
        } else {
            Err(DiscoveryError::UnexpectedError)
        }
    }

    async fn load_bootnodes(&self, bootnodes: Vec<BootNode>) {
        for bootnode in bootnodes {
            self.table.lock().await.insert_node(Node {
                ip: bootnode.socket_address.ip(),
                udp_port: bootnode.socket_address.port(),
                // TODO: udp port can differ from tcp port.
                // see https://github.com/lambdaclass/ethrex/issues/905
                tcp_port: bootnode.socket_address.port(),
                node_id: bootnode.node_id,
            });
            let ping_hash = ping(
                &self.udp_socket,
                self.udp_addr,
                bootnode.socket_address,
                &self.signer,
            )
            .await;
            self.table
                .lock()
                .await
                .update_peer_ping(bootnode.node_id, ping_hash);
        }
    }

    async fn receive(self: Arc<Self>) {
        let mut buf = vec![0; MAX_DISC_PACKET_SIZE];

        loop {
            let (read, from) = self.udp_socket.recv_from(&mut buf).await.unwrap();
            debug!("Received {read} bytes from {from}");

            let packet = Packet::decode(&buf[..read]);
            if packet.is_err() {
                debug!("Could not decode packet: {:?}", packet.err().unwrap());
                continue;
            }
            let packet = packet.unwrap();

            self.handle_message(packet, from, read, &buf).await;
        }
    }

    async fn handle_message(
        &self,
        packet: Packet,
        from: SocketAddr,
        msg_len: usize,
        msg_bytes: &[u8],
    ) {
        let msg = packet.get_message();
        debug!("Message: {:?} from {}", msg, packet.get_node_id());
        match msg {
            Message::Ping(msg) => {
                if is_expired(msg.expiration) {
                    debug!("Ignoring ping as it is expired.");
                    return;
                };
                let ping_hash = packet.get_hash();
                pong(&self.udp_socket, from, ping_hash, &self.signer).await;
                let node = {
                    let table = self.table.lock().await;
                    table.get_by_node_id(packet.get_node_id()).cloned()
                };
                if let Some(peer) = node {
                    // send a a ping to get an endpoint proof
                    if time_since_in_hs(peer.last_ping) >= PROOF_EXPIRATION_IN_HS as u64 {
                        let hash = ping(&self.udp_socket, self.udp_addr, from, &self.signer).await;
                        if let Some(hash) = hash {
                            self.table
                                .lock()
                                .await
                                .update_peer_ping(peer.node.node_id, Some(hash));
                        }
                    }
                } else {
                    // send a ping to get the endpoint proof from our end
                    let (peer, inserted_to_table) = {
                        let mut table = self.table.lock().await;
                        table.insert_node(Node {
                            ip: from.ip(),
                            udp_port: msg.from.udp_port,
                            tcp_port: msg.from.tcp_port,
                            node_id: packet.get_node_id(),
                        })
                    };
                    let hash = ping(&self.udp_socket, self.udp_addr, from, &self.signer).await;
                    if let Some(hash) = hash {
                        if inserted_to_table && peer.is_some() {
                            let peer = peer.unwrap();
                            self.table
                                .lock()
                                .await
                                .update_peer_ping(peer.node.node_id, Some(hash));
                        }
                    }
                }
            }
            Message::Pong(msg) => {
                let table = self.table.clone();
                if is_expired(msg.expiration) {
                    debug!("Ignoring pong as it is expired.");
                    return;
                }
                let peer = {
                    let table = table.lock().await;
                    table.get_by_node_id(packet.get_node_id()).cloned()
                };
                if let Some(peer) = peer {
                    if peer.last_ping_hash.is_none() {
                        debug!("Discarding pong as the node did not send a previous ping");
                        return;
                    }
                    if peer.last_ping_hash.unwrap() == msg.ping_hash {
                        table.lock().await.pong_answered(peer.node.node_id);

                        let mut msg_buf = vec![0; msg_len - 32];
                        msg_bytes[32..msg_len].clone_into(&mut msg_buf);
                        let signer = self.signer.clone();
                        let storage = self.storage.clone();
                        let broadcaster = self.tx_broadcaster_send.clone();
                        tokio::spawn(async move {
                            handle_peer_as_initiator(
                                signer,
                                &msg_buf,
                                &peer.node,
                                storage,
                                table,
                                broadcaster,
                            )
                            .await;
                        });
                    } else {
                        debug!(
                            "Discarding pong as the hash did not match the last corresponding ping"
                        );
                    }
                } else {
                    debug!("Discarding pong as it is not a known node");
                }
            }
            Message::FindNode(msg) => {
                if is_expired(msg.expiration) {
                    debug!("Ignoring find node msg as it is expired.");
                    return;
                };
                let node = {
                    let table = self.table.lock().await;
                    table.get_by_node_id(packet.get_node_id()).cloned()
                };
                if let Some(node) = node {
                    if node.is_proven {
                        let nodes = {
                            let table = self.table.lock().await;
                            table.get_closest_nodes(msg.target)
                        };
                        let nodes_chunks = nodes.chunks(4);
                        let expiration = get_expiration(20);
                        debug!("Sending neighbors!");
                        // we are sending the neighbors in 4 different messages as not to exceed the
                        // maximum packet size
                        for nodes in nodes_chunks {
                            let neighbors = Message::Neighbors(NeighborsMessage::new(
                                nodes.to_vec(),
                                expiration,
                            ));
                            let mut buf = Vec::new();
                            neighbors.encode_with_header(&mut buf, &self.signer);
                            let _ = self.udp_socket.send_to(&buf, from).await;
                        }
                    } else {
                        debug!("Ignoring find node message as the node isn't proven!");
                    }
                } else {
                    debug!("Ignoring find node message as it is not a known node");
                }
            }
            Message::Neighbors(neighbors_msg) => {
                if is_expired(neighbors_msg.expiration) {
                    debug!("Ignoring neighbor msg as it is expired.");
                    return;
                };

                let mut nodes_to_insert = None;
                let mut table = self.table.lock().await;
                if let Some(node) = table.get_by_node_id_mut(packet.get_node_id()) {
                    if let Some(req) = &mut node.find_node_request {
                        if time_now_unix().saturating_sub(req.sent_at) >= 60 {
                            debug!("Ignoring neighbors message as the find_node request expires after one minute");
                            node.find_node_request = None;
                            return;
                        }
                        let nodes = &neighbors_msg.nodes;
                        let nodes_sent = req.nodes_sent + nodes.len();

                        if nodes_sent <= MAX_NODES_PER_BUCKET {
                            debug!("Storing neighbors in our table!");
                            req.nodes_sent = nodes_sent;
                            nodes_to_insert = Some(nodes.clone());
                            if let Some(tx) = &req.tx {
                                let _ = tx.send(nodes.clone());
                            }
                        } else {
                            debug!("Ignoring neighbors message as the client sent more than the allowed nodes");
                        }

                        if nodes_sent == MAX_NODES_PER_BUCKET {
                            debug!("Neighbors request has been fulfilled");
                            node.find_node_request = None;
                        }
                    }
                } else {
                    debug!("Ignoring neighbor msg as it is not a known node");
                }

                if let Some(nodes) = nodes_to_insert {
                    for node in nodes {
                        let (peer, inserted_to_table) = table.insert_node(node);
                        if inserted_to_table && peer.is_some() {
                            let peer = peer.unwrap();
                            let node_addr = SocketAddr::new(peer.node.ip, peer.node.udp_port);
                            let ping_hash =
                                ping(&self.udp_socket, self.udp_addr, node_addr, &self.signer)
                                    .await;
                            table.update_peer_ping(peer.node.node_id, ping_hash);
                        };
                    }
                }
            }
            _ => {}
        }
    }

    async fn start_revalidation_task(self: Arc<Self>) {
        let mut interval =
            tokio::time::interval(Duration::from_secs(self.revalidation_interval_seconds));
        // peers we have pinged in the previous iteration
        let mut previously_pinged_peers: HashSet<H512> = HashSet::default();

        // first tick starts immediately
        interval.tick().await;

        loop {
            interval.tick().await;
            debug!("Running peer revalidation");

            // first check that the peers we ping have responded
            for node_id in previously_pinged_peers {
                let mut table = self.table.lock().await;
                let peer = table.get_by_node_id_mut(node_id).unwrap();

                if let Some(has_answered) = peer.revalidation {
                    if has_answered {
                        peer.increment_liveness();
                    } else {
                        peer.decrement_liveness();
                    }
                }

                peer.revalidation = None;

                if peer.liveness == 0 {
                    let new_peer = table.replace_peer(node_id);
                    if let Some(new_peer) = new_peer {
                        let ping_hash = ping(
                            &self.udp_socket,
                            self.udp_addr,
                            SocketAddr::new(new_peer.node.ip, new_peer.node.udp_port),
                            &self.signer,
                        )
                        .await;
                        table.update_peer_ping(new_peer.node.node_id, ping_hash);
                    }
                }
            }

            // now send a ping to the least recently pinged peers
            // this might be too expensive to run if our table is filled
            // maybe we could just pick them randomly
            let peers = self.table.lock().await.get_least_recently_pinged_peers(3);
            previously_pinged_peers = HashSet::default();
            for peer in peers {
                let ping_hash = ping(
                    &self.udp_socket,
                    self.udp_addr,
                    SocketAddr::new(peer.node.ip, peer.node.udp_port),
                    &self.signer,
                )
                .await;
                let mut table = self.table.lock().await;
                table.update_peer_ping_with_revalidation(peer.node.node_id, ping_hash);
                previously_pinged_peers.insert(peer.node.node_id);

                debug!("Pinging peer {:?} to re-validate!", peer.node.node_id);
            }

            debug!("Peer revalidation finished");
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::{kademlia::bucket_number, node_id_from_signing_key, MAX_MESSAGES_TO_BROADCAST};
//     use ethrex_storage::EngineType;
//     use rand::rngs::OsRng;
//     use std::{
//         collections::HashSet,
//         net::{IpAddr, Ipv4Addr},
//     };
//     use tokio::time::sleep;

//     async fn insert_random_node_on_custom_bucket(
//         table: Arc<Mutex<KademliaTable>>,
//         bucket_idx: usize,
//     ) {
//         let node_id = node_id_from_signing_key(&SigningKey::random(&mut OsRng));
//         let node = Node {
//             ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
//             tcp_port: 0,
//             udp_port: 0,
//             node_id,
//         };
//         table
//             .lock()
//             .await
//             .insert_node_on_custom_bucket(node, bucket_idx);
//     }

//     async fn fill_table_with_random_nodes(table: Arc<Mutex<KademliaTable>>) {
//         for i in 0..256 {
//             for _ in 0..16 {
//                 insert_random_node_on_custom_bucket(table.clone(), i).await;
//             }
//         }
//     }

//     async fn start_mock_discovery_server(udp_port: u16, should_start_server: bool) -> Discv4 {
//         let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), udp_port);
//         let signer = SigningKey::random(&mut OsRng);
//         let udp_socket = Arc::new(UdpSocket::bind(addr).await.unwrap());
//         let node_id = node_id_from_signing_key(&signer);
//         let storage =
//             Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
//         let table = Arc::new(Mutex::new(KademliaTable::new(node_id)));
//         let (channel_broadcast_send_end, _) = tokio::sync::broadcast::channel::<(
//             tokio::task::Id,
//             Arc<RLPxMessage>,
//         )>(MAX_MESSAGES_TO_BROADCAST);

//         let discv4 = Discv4::new();
//         if should_start_server {
//             tokio::spawn(disv4.handle_messages());
//         }
//     }

//     /// connects two mock servers by pinging a to b
//     async fn connect_servers(server_a: &mut MockServer, server_b: &mut MockServer) {
//         let ping_hash = ping(
//             &server_a.udp_socket,
//             server_a.addr,
//             server_b.addr,
//             &server_a.signer,
//         )
//         .await;
//         {
//             let mut table = server_a.table.lock().await;
//             table.insert_node(Node {
//                 ip: server_b.addr.ip(),
//                 udp_port: server_b.addr.port(),
//                 tcp_port: 0,
//                 node_id: server_b.node_id,
//             });
//             table.update_peer_ping(server_b.node_id, ping_hash);
//         }
//         // allow some time for the server to respond
//         sleep(Duration::from_secs(1)).await;
//     }

//     #[tokio::test]
//     /** This is a end to end test on the discovery server, the idea is as follows:
//      * - We'll start two discovery servers (`a` & `b`) to ping between each other
//      * - We'll make `b` ping `a`, and validate that the connection is right
//      * - Then we'll wait for a revalidation where we expect everything to be the same
//      * - We'll do this five 5 more times
//      * - Then we'll stop server `a` so that it doesn't respond to re-validations
//      * - We expect server `b` to remove node `a` from its table after 3 re-validations
//      * To make this run faster, we'll change the revalidation time to be every 2secs
//      */
//     async fn discovery_server_revalidation() {
//         let mut server_a = start_mock_discovery_server(7998, true).await;
//         let mut server_b = start_mock_discovery_server(7999, true).await;

//         connect_servers(&mut server_a, &mut server_b).await;

//         // start revalidation server
//         tokio::spawn(peers_revalidation(
//             server_b.addr,
//             server_b.udp_socket.clone(),
//             server_b.table.clone(),
//             server_b.signer.clone(),
//             2,
//         ));

//         for _ in 0..5 {
//             sleep(Duration::from_millis(2500)).await;
//             // by now, b should've send a revalidation to a
//             let table = server_b.table.lock().await;
//             let node = table.get_by_node_id(server_a.node_id).unwrap();
//             assert!(node.revalidation.is_some());
//         }

//         // make sure that `a` has responded too all the re-validations
//         // we can do that by checking the liveness
//         {
//             let table = server_b.table.lock().await;
//             let node = table.get_by_node_id(server_a.node_id).unwrap();
//             assert_eq!(node.liveness, 6);
//         }

//         // now, stopping server `a` is not trivial
//         // so we'll instead change its port, so that no one responds
//         {
//             let mut table = server_b.table.lock().await;
//             let node = table.get_by_node_id_mut(server_a.node_id).unwrap();
//             node.node.udp_port = 0;
//         }

//         // now the liveness field should start decreasing until it gets to 0
//         // which should happen in 3 re-validations
//         for _ in 0..2 {
//             sleep(Duration::from_millis(2500)).await;
//             let table = server_b.table.lock().await;
//             let node = table.get_by_node_id(server_a.node_id).unwrap();
//             assert!(node.revalidation.is_some());
//         }
//         sleep(Duration::from_millis(2500)).await;

//         // finally, `a`` should not exist anymore
//         let table = server_b.table.lock().await;
//         assert!(table.get_by_node_id(server_a.node_id).is_none());
//     }

//     #[tokio::test]
//     /** This test tests the lookup function, the idea is as follows:
//      * - We'll start two discovery servers (`a` & `b`) that will connect between each other
//      * - We'll insert random nodes to the server `a`` to fill its table
//      * - We'll forcedly run `lookup` and validate that a `find_node` request was sent
//      *   by checking that new nodes have been inserted to the table
//      *
//      * This test for only one lookup, and not recursively.
//      */
//     async fn discovery_server_lookup() {
//         let mut server_a = start_mock_discovery_server(8000, true).await;
//         let mut server_b = start_mock_discovery_server(8001, true).await;

//         fill_table_with_random_nodes(server_a.table.clone()).await;

//         // before making the connection, remove a node from the `b` bucket. Otherwise it won't be added
//         let b_bucket = bucket_number(server_a.node_id, server_b.node_id);
//         let node_id_to_remove = server_a.table.lock().await.buckets()[b_bucket].peers[0]
//             .node
//             .node_id;
//         server_a
//             .table
//             .lock()
//             .await
//             .replace_peer_on_custom_bucket(node_id_to_remove, b_bucket);

//         connect_servers(&mut server_a, &mut server_b).await;

//         // now we are going to run a lookup with us as the target
//         let closets_peers_to_b_from_a = server_a
//             .table
//             .lock()
//             .await
//             .get_closest_nodes(server_b.node_id);
//         let nodes_to_ask = server_b
//             .table
//             .lock()
//             .await
//             .get_closest_nodes(server_b.node_id);

//         lookup(
//             server_b.udp_socket.clone(),
//             server_b.table.clone(),
//             &server_b.signer,
//             server_b.node_id,
//             &mut HashSet::default(),
//             &nodes_to_ask,
//         )
//         .await;

//         // find_node sent, allow some time for `a` to respond
//         sleep(Duration::from_secs(2)).await;

//         // now all peers should've been inserted
//         for peer in closets_peers_to_b_from_a {
//             let table = server_b.table.lock().await;
//             assert!(table.get_by_node_id(peer.node_id).is_some());
//         }
//     }

//     #[tokio::test]
//     /** This test tests the lookup function, the idea is as follows:
//      * - We'll start four discovery servers (`a`, `b`, `c` & `d`)
//      * - `a` will be connected to `b`, `b` will be connected to `c` and `c` will be connected to `d`.
//      * - The server `d` will have its table filled with mock nodes
//      * - We'll run a recursive lookup on server `a` and we expect to end with `b`, `c`, `d` and its mock nodes
//      */
//     async fn discovery_server_recursive_lookup() {
//         let mut server_a = start_mock_discovery_server(8002, true).await;
//         let mut server_b = start_mock_discovery_server(8003, true).await;
//         let mut server_c = start_mock_discovery_server(8004, true).await;
//         let mut server_d = start_mock_discovery_server(8005, true).await;

//         connect_servers(&mut server_a, &mut server_b).await;
//         connect_servers(&mut server_b, &mut server_c).await;
//         connect_servers(&mut server_c, &mut server_d).await;

//         // now we fill the server_d table with 3 random nodes
//         // the reason we don't put more is because this nodes won't respond (as they don't are not real servers)
//         // and so we will have to wait for the timeout on each node, which will only slow down the test
//         for _ in 0..3 {
//             insert_random_node_on_custom_bucket(server_d.table.clone(), 0).await;
//         }

//         let mut expected_peers = vec![];
//         expected_peers.extend(
//             server_b
//                 .table
//                 .lock()
//                 .await
//                 .get_closest_nodes(server_a.node_id),
//         );
//         expected_peers.extend(
//             server_c
//                 .table
//                 .lock()
//                 .await
//                 .get_closest_nodes(server_a.node_id),
//         );
//         expected_peers.extend(
//             server_d
//                 .table
//                 .lock()
//                 .await
//                 .get_closest_nodes(server_a.node_id),
//         );

//         // we'll run a recursive lookup closest to the server itself
//         recursive_lookup(
//             server_a.udp_socket.clone(),
//             server_a.table.clone(),
//             server_a.signer.clone(),
//             server_a.node_id,
//             server_a.node_id,
//         )
//         .await;

//         for peer in expected_peers {
//             assert!(server_a
//                 .table
//                 .lock()
//                 .await
//                 .get_by_node_id(peer.node_id)
//                 .is_some());
//         }
//     }
// }
