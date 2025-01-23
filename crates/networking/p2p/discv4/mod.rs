pub(super) mod helpers;
mod lookup;
pub(super) mod messages;

use crate::{
    bootnode::BootNode,
    handle_peer_as_initiator,
    kademlia::MAX_NODES_PER_BUCKET,
    rlpx::connection::RLPxConnBroadcastSender,
    types::{Endpoint, Node, NodeRecord},
    KademliaTable,
};
use ethrex_core::H256;
use ethrex_storage::Store;
use helpers::{get_expiration, is_expired, time_now_unix, time_since_in_hs};
use k256::ecdsa::{signature::hazmat::PrehashVerifier, Signature, SigningKey, VerifyingKey};
use lookup::Discv4LookupHandler;
use messages::{
    ENRRequestMessage, ENRResponseMessage, FindNodeMessage, Message, NeighborsMessage, Packet,
    PingMessage, PongMessage,
};
use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{net::UdpSocket, sync::Mutex};
use tokio_util::task::TaskTracker;
use tracing::{debug, error};

pub const MAX_DISC_PACKET_SIZE: usize = 1280;
const PROOF_EXPIRATION_IN_HS: u64 = 12;

// These interval times are arbitrary numbers, maybe we should read them from a cfg or a cli param
const REVALIDATION_INTERVAL_IN_SECONDS: u64 = 30;
const PEERS_RANDOM_LOOKUP_TIME_IN_MIN: u64 = 30;

#[derive(Debug)]
#[allow(dead_code)]
pub enum DiscoveryError {
    BindSocket(std::io::Error),
    MessageSendFailure(std::io::Error),
    PartialMessageSent,
    MessageExpired,
    InvalidMessage(String),
}

#[derive(Debug, Clone)]
pub struct Discv4 {
    local_node: Node,
    udp_socket: Arc<UdpSocket>,
    signer: SigningKey,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    tracker: TaskTracker,
    rlxp_conn_sender: RLPxConnBroadcastSender,
    revalidation_interval_seconds: u64,
    lookup_interval_minutes: u64,
}

impl Discv4 {
    pub async fn try_new(
        local_node: Node,
        signer: SigningKey,
        storage: Store,
        table: Arc<Mutex<KademliaTable>>,
        rlpx_conn_sender: RLPxConnBroadcastSender,
        tracker: TaskTracker,
    ) -> Result<Self, DiscoveryError> {
        let udp_socket = UdpSocket::bind(SocketAddr::new(local_node.ip, local_node.udp_port))
            .await
            .map_err(DiscoveryError::BindSocket)?;

        Ok(Self {
            local_node,
            signer,
            storage,
            table,
            rlxp_conn_sender: rlpx_conn_sender,
            udp_socket: Arc::new(udp_socket),
            revalidation_interval_seconds: REVALIDATION_INTERVAL_IN_SECONDS,
            lookup_interval_minutes: PEERS_RANDOM_LOOKUP_TIME_IN_MIN,
            tracker,
        })
    }

    #[allow(unused)]
    pub fn with_revalidation_interval_of(self, seconds: u64) -> Self {
        Self {
            revalidation_interval_seconds: seconds,
            ..self
        }
    }

    #[allow(unused)]
    pub fn with_lookup_interval_of(self, minutes: u64) -> Self {
        Self {
            revalidation_interval_seconds: minutes,
            ..self
        }
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.local_node.ip, self.local_node.udp_port)
    }

    pub async fn start(&self, bootnodes: Vec<BootNode>) -> Result<(), DiscoveryError> {
        let lookup_handler = Discv4LookupHandler::new(
            self.local_node,
            self.signer.clone(),
            self.udp_socket.clone(),
            self.table.clone(),
            self.lookup_interval_minutes,
            self.tracker.clone(),
        );

        self.tracker.spawn({
            let self_clone = self.clone();
            async move { self_clone.receive().await }
        });
        self.tracker.spawn({
            let self_clone = self.clone();
            async move { self_clone.start_revalidation().await }
        });
        self.load_bootnodes(bootnodes).await;
        lookup_handler.start(10);

        Ok(())
    }

    async fn load_bootnodes(&self, bootnodes: Vec<BootNode>) {
        for bootnode in bootnodes {
            let node = Node {
                ip: bootnode.socket_address.ip(),
                udp_port: bootnode.socket_address.port(),
                // TODO: udp port can differ from tcp port.
                // see https://github.com/lambdaclass/ethrex/issues/905
                tcp_port: bootnode.socket_address.port(),
                node_id: bootnode.node_id,
            };
            if let Err(e) = self.try_add_peer_and_ping(node).await {
                debug!("Error while adding bootnode to table: {:?}", e);
            };
        }
    }

    pub async fn receive(&self) {
        let mut buf = vec![0; MAX_DISC_PACKET_SIZE];

        loop {
            let (read, from) = match self.udp_socket.recv_from(&mut buf).await {
                Ok(result) => result,
                Err(e) => {
                    error!("Error receiving data from socket: {e}. Stopping discovery server");
                    return;
                }
            };
            debug!("Received {read} bytes from {from}");

            match Packet::decode(&buf[..read]) {
                Err(e) => error!("Could not decode packet: {:?}", e),
                Ok(packet) => {
                    let msg = packet.get_message();
                    let msg_name = msg.to_string();
                    debug!("Message: {:?} from {}", msg, packet.get_node_id());
                    if let Err(e) = self.handle_message(packet, from, read, &buf).await {
                        debug!("Error while processing {} message: {:?}", msg_name, e);
                    };
                }
            }
        }
    }

    async fn handle_message(
        &self,
        packet: Packet,
        from: SocketAddr,
        msg_len: usize,
        msg_bytes: &[u8],
    ) -> Result<(), DiscoveryError> {
        match packet.get_message() {
            Message::Ping(msg) => {
                if is_expired(msg.expiration) {
                    return Err(DiscoveryError::MessageExpired);
                };
                let node = Node {
                    ip: from.ip(),
                    udp_port: msg.from.udp_port,
                    tcp_port: msg.from.tcp_port,
                    node_id: packet.get_node_id(),
                };
                self.pong(packet.get_hash(), node).await?;
                let peer = {
                    let table = self.table.lock().await;
                    table.get_by_node_id(packet.get_node_id()).cloned()
                };
                // if peer was already inserted, and last ping was 12 hs ago
                //  we need to re ping to re-validate the endpoint proof
                if let Some(peer) = peer {
                    if time_since_in_hs(peer.last_ping) >= PROOF_EXPIRATION_IN_HS {
                        self.ping(node).await?;
                    }
                    if let Some(enr_seq) = msg.enr_seq {
                        if enr_seq > peer.record.seq {
                            debug!("Found outdated enr-seq, send an enr_request");
                            self.send_enr_request(peer.node, enr_seq).await?;
                        }
                    }
                } else {
                    // otherwise add to the table
                    let mut table = self.table.lock().await;
                    if let (Some(peer), true) = table.insert_node(node) {
                        // it was inserted, send ping to bond
                        self.ping(peer.node).await?;
                    }
                }

                Ok(())
            }
            Message::Pong(msg) => {
                let table = self.table.clone();
                if is_expired(msg.expiration) {
                    return Err(DiscoveryError::MessageExpired);
                }
                let peer = {
                    let table = table.lock().await;
                    table.get_by_node_id(packet.get_node_id()).cloned()
                };
                if let Some(peer) = peer {
                    if peer.last_ping_hash.is_none() {
                        return Err(DiscoveryError::InvalidMessage(
                            "node did not send a previous ping".into(),
                        ));
                    }
                    if peer
                        .last_ping_hash
                        .is_some_and(|hash| hash == msg.ping_hash)
                    {
                        table.lock().await.pong_answered(peer.node.node_id);
                        if let Some(enr_seq) = msg.enr_seq {
                            if enr_seq > peer.record.seq {
                                debug!("Found outdated enr-seq, send an enr_request");
                                self.send_enr_request(peer.node, enr_seq).await?;
                            }
                        }
                        let mut msg_buf = vec![0; msg_len - 32];
                        msg_bytes[32..msg_len].clone_into(&mut msg_buf);
                        let signer = self.signer.clone();
                        let storage = self.storage.clone();
                        let broadcaster = self.rlxp_conn_sender.clone();
                        self.tracker.spawn(async move {
                            handle_peer_as_initiator(
                                signer,
                                &msg_buf,
                                &peer.node,
                                storage,
                                table,
                                broadcaster,
                            )
                            .await
                        });
                        Ok(())
                    } else {
                        Err(DiscoveryError::InvalidMessage(
                            "pong as the hash did not match the last corresponding ping".into(),
                        ))
                    }
                } else {
                    Err(DiscoveryError::InvalidMessage(
                        "pong from a not known node".into(),
                    ))
                }
            }
            Message::FindNode(msg) => {
                if is_expired(msg.expiration) {
                    return Err(DiscoveryError::MessageExpired);
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
                            let bytes_sent = self
                                .udp_socket
                                .send_to(&buf, from)
                                .await
                                .map_err(DiscoveryError::MessageSendFailure)?;

                            if bytes_sent != buf.len() {
                                return Err(DiscoveryError::PartialMessageSent);
                            }
                        }
                        Ok(())
                    } else {
                        Err(DiscoveryError::InvalidMessage("Node isn't proven.".into()))
                    }
                } else {
                    Err(DiscoveryError::InvalidMessage("Node is not known".into()))
                }
            }
            Message::Neighbors(neighbors_msg) => {
                if is_expired(neighbors_msg.expiration) {
                    return Err(DiscoveryError::MessageExpired);
                };

                let mut nodes_to_insert = None;
                let mut table = self.table.lock().await;
                if let Some(node) = table.get_by_node_id_mut(packet.get_node_id()) {
                    if let Some(req) = &mut node.find_node_request {
                        if time_now_unix().saturating_sub(req.sent_at) >= 60 {
                            node.find_node_request = None;
                            return Err(DiscoveryError::InvalidMessage(
                                "find_node request expired after one minute".into(),
                            ));
                        }
                        let nodes = &neighbors_msg.nodes;
                        let nodes_sent = req.nodes_sent + nodes.len();

                        if nodes_sent <= MAX_NODES_PER_BUCKET {
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
                    return Err(DiscoveryError::InvalidMessage("Unknown node".into()));
                }

                if let Some(nodes) = nodes_to_insert {
                    debug!("Storing neighbors in our table!");
                    for node in nodes {
                        let _ = self.try_add_peer_and_ping(node).await;
                    }
                }

                Ok(())
            }
            Message::ENRRequest(msg) => {
                if is_expired(msg.expiration) {
                    return Err(DiscoveryError::MessageExpired);
                }
                // Note we are passing the current timestamp as the sequence number
                // This is because we are not storing our local_node updates in the db
                let Ok(node_record) =
                    NodeRecord::from_node(self.local_node, time_now_unix(), &self.signer)
                else {
                    return Err(DiscoveryError::InvalidMessage(
                        "Could not build local node record".into(),
                    ));
                };
                let msg =
                    Message::ENRResponse(ENRResponseMessage::new(packet.get_hash(), node_record));
                let mut buf = vec![];
                msg.encode_with_header(&mut buf, &self.signer);
                match self.udp_socket.send_to(&buf, from).await {
                    Ok(bytes_sent) => {
                        if bytes_sent == buf.len() {
                            Ok(())
                        } else {
                            Err(DiscoveryError::PartialMessageSent)
                        }
                    }
                    Err(e) => Err(DiscoveryError::MessageSendFailure(e)),
                }
            }
            Message::ENRResponse(msg) => {
                let mut table = self.table.lock().await;
                let peer = table.get_by_node_id_mut(packet.get_node_id());
                let Some(peer) = peer else {
                    return Err(DiscoveryError::InvalidMessage("Peer not known".into()));
                };

                let Some(req_hash) = peer.enr_request_hash else {
                    return Err(DiscoveryError::InvalidMessage(
                        "Discarding enr-response as enr-request wasn't sent".into(),
                    ));
                };
                if req_hash != msg.request_hash {
                    return Err(DiscoveryError::InvalidMessage(
                        "Discarding enr-response did not match enr-request hash".into(),
                    ));
                }
                peer.enr_request_hash = None;

                if msg.node_record.seq < peer.record.seq {
                    return Err(DiscoveryError::InvalidMessage(
                        "msg node record is lower than the one we have".into(),
                    ));
                }

                let record = msg.node_record.decode_pairs();
                let Some(id) = record.id else {
                    return Err(DiscoveryError::InvalidMessage(
                        "msg node record does not have required `id` field".into(),
                    ));
                };

                // https://github.com/ethereum/devp2p/blob/master/enr.md#v4-identity-scheme
                let signature_valid = match id.as_str() {
                    "v4" => {
                        let digest = msg.node_record.get_signature_digest();
                        let Some(public_key) = record.secp256k1 else {
                            return Err(DiscoveryError::InvalidMessage(
                                "signature could not be verified because public key was not provided".into(),
                            ));
                        };
                        let signature_bytes = msg.node_record.signature.as_bytes();
                        let Ok(signature) = Signature::from_slice(&signature_bytes[0..64]) else {
                            return Err(DiscoveryError::InvalidMessage(
                                "signature could not be build from msg signature bytes".into(),
                            ));
                        };
                        let Ok(verifying_key) =
                            VerifyingKey::from_sec1_bytes(public_key.as_bytes())
                        else {
                            return Err(DiscoveryError::InvalidMessage(
                                "public key could no be built from msg pub key bytes".into(),
                            ));
                        };
                        verifying_key.verify_prehash(&digest, &signature).is_ok()
                    }
                    _ => false,
                };
                if !signature_valid {
                    return Err(DiscoveryError::InvalidMessage(
                        "Signature verification invalid".into(),
                    ));
                }

                if let Some(ip) = record.ip {
                    peer.node.ip = IpAddr::from(Ipv4Addr::from_bits(ip));
                }
                if let Some(tcp_port) = record.tcp_port {
                    peer.node.tcp_port = tcp_port;
                }
                if let Some(udp_port) = record.udp_port {
                    peer.node.udp_port = udp_port;
                }
                peer.record.seq = msg.node_record.seq;
                peer.record = msg.node_record.clone();
                debug!(
                    "Node with id {:?} record has been successfully updated",
                    peer.node.node_id
                );
                Ok(())
            }
        }
    }

    /// Starts a tokio scheduler that:
    /// - performs periodic revalidation of the current nodes (sends a ping to the old nodes).
    ///
    /// **Peer revalidation**
    ///
    /// Peers revalidation works in the following manner:
    /// 1. Every `revalidation_interval_seconds` we ping the 3 least recently pinged peers
    /// 2. In the next iteration we check if they have answered
    ///    - if they have: we increment the liveness field by one
    ///    - otherwise we decrement it by the current value / 3.
    /// 3. If the liveness field is 0, then we delete it and insert a new one from the replacements table
    ///
    /// See more https://github.com/ethereum/devp2p/blob/master/discv4.md#kademlia-table
    pub async fn start_revalidation(&self) {
        let mut interval =
            tokio::time::interval(Duration::from_secs(self.revalidation_interval_seconds));

        // first tick starts immediately
        interval.tick().await;

        let mut previously_pinged_peers = HashSet::new();
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
                        let _ = self.ping(new_peer.node).await;
                    }
                }
            }

            // now send a ping to the least recently pinged peers
            // this might be too expensive to run if our table is filled
            // maybe we could just pick them randomly
            let peers = self.table.lock().await.get_least_recently_pinged_peers(3);
            previously_pinged_peers = HashSet::default();
            for peer in peers {
                debug!("Pinging peer {:?} to re-validate!", peer.node.node_id);
                let _ = self.ping(peer.node).await;
                previously_pinged_peers.insert(peer.node.node_id);
                let mut table = self.table.lock().await;
                let peer = table.get_by_node_id_mut(peer.node.node_id);
                if let Some(peer) = peer {
                    peer.revalidation = Some(false);
                }
            }

            debug!("Peer revalidation finished");
        }
    }

    /// Attempts to add a node to the Kademlia table and send a ping if necessary.
    ///
    /// - If the node is **not found** in the table and there is enough space, it will be added,  
    ///   and a ping message will be sent to verify connectivity.
    /// - If the node is **already present**, no action is taken.
    async fn try_add_peer_and_ping(&self, node: Node) -> Result<(), DiscoveryError> {
        if let (Some(peer), true) = self.table.lock().await.insert_node(node) {
            self.ping(peer.node).await?;
        };
        Ok(())
    }

    async fn ping(&self, node: Node) -> Result<(), DiscoveryError> {
        let mut buf = Vec::new();
        let expiration: u64 = get_expiration(20);
        let from = Endpoint {
            ip: self.local_node.ip,
            udp_port: self.local_node.udp_port,
            tcp_port: self.local_node.tcp_port,
        };
        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let ping =
            Message::Ping(PingMessage::new(from, to, expiration).with_enr_seq(time_now_unix()));
        ping.encode_with_header(&mut buf, &self.signer);
        let bytes_sent = self
            .udp_socket
            .send_to(&buf, SocketAddr::new(node.ip, node.udp_port))
            .await
            .map_err(DiscoveryError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryError::PartialMessageSent);
        }

        let hash = H256::from_slice(&buf[0..32]);
        self.table
            .lock()
            .await
            .update_peer_ping(node.node_id, Some(hash));

        Ok(())
    }

    async fn pong(&self, ping_hash: H256, node: Node) -> Result<(), DiscoveryError> {
        let mut buf = Vec::new();
        let expiration: u64 = get_expiration(20);
        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let pong = Message::Pong(
            PongMessage::new(to, ping_hash, expiration).with_enr_seq(time_now_unix()),
        );
        pong.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, SocketAddr::new(node.ip, node.udp_port))
            .await
            .map_err(DiscoveryError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            Err(DiscoveryError::PartialMessageSent)
        } else {
            Ok(())
        }
    }

    async fn send_enr_request(&self, node: Node, enr_seq: u64) -> Result<(), DiscoveryError> {
        let mut buf = Vec::new();

        let expiration: u64 = get_expiration(20);
        let enr_req = Message::ENRRequest(ENRRequestMessage::new(expiration));
        enr_req.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, SocketAddr::new(node.ip, node.udp_port))
            .await
            .map_err(DiscoveryError::MessageSendFailure)?;
        if bytes_sent != buf.len() {
            return Err(DiscoveryError::PartialMessageSent);
        }

        let hash = H256::from_slice(&buf[0..32]);
        self.table
            .lock()
            .await
            .update_peer_enr_seq(node.node_id, enr_seq, Some(hash));

        Ok(())
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use ethrex_storage::EngineType;
//     use kademlia::bucket_number;
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

//     struct MockServer {
//         pub addr: SocketAddr,
//         pub signer: SigningKey,
//         pub table: Arc<Mutex<KademliaTable>>,
//         pub node_id: H512,
//         pub udp_socket: Arc<UdpSocket>,
//     }

//     async fn start_mock_discovery_server(
//         udp_port: u16,
//         should_start_server: bool,
//     ) -> Result<MockServer, io::Error> {
//         let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), udp_port);
//         let signer = SigningKey::random(&mut OsRng);
//         let udp_socket = Arc::new(UdpSocket::bind(addr).await?);
//         let node_id = node_id_from_signing_key(&signer);
//         let storage =
//             Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
//         let table = Arc::new(Mutex::new(KademliaTable::new(node_id)));
//         let (channel_broadcast_send_end, _) = tokio::sync::broadcast::channel::<(
//             tokio::task::Id,
//             Arc<RLPxMessage>,
//         )>(MAX_MESSAGES_TO_BROADCAST);
//         let tracker = TaskTracker::new();
//         if should_start_server {
//             tracker.spawn(discover_peers_server(
//                 tracker.clone(),
//                 addr,
//                 udp_socket.clone(),
//                 storage.clone(),
//                 table.clone(),
//                 signer.clone(),
//                 channel_broadcast_send_end,
//             ));
//         }

//         Ok(MockServer {
//             addr,
//             signer,
//             table,
//             node_id,
//             udp_socket,
//         })
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
//     async fn discovery_server_revalidation() -> Result<(), io::Error> {
//         let mut server_a = start_mock_discovery_server(7998, true).await?;
//         let mut server_b = start_mock_discovery_server(7999, true).await?;

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
//             let node = table.get_by_node_id(server_a.node_id);
//             assert!(node.is_some_and(|n| n.revalidation.is_some()));
//         }

//         // make sure that `a` has responded too all the re-validations
//         // we can do that by checking the liveness
//         {
//             let table = server_b.table.lock().await;
//             let node = table.get_by_node_id(server_a.node_id);
//             assert_eq!(node.map_or(0, |n| n.liveness), 6);
//         }

//         // now, stopping server `a` is not trivial
//         // so we'll instead change its port, so that no one responds
//         {
//             let mut table = server_b.table.lock().await;
//             let node = table.get_by_node_id_mut(server_a.node_id);
//             if let Some(node) = node {
//                 node.node.udp_port = 0
//             };
//         }

//         // now the liveness field should start decreasing until it gets to 0
//         // which should happen in 3 re-validations
//         for _ in 0..2 {
//             sleep(Duration::from_millis(2500)).await;
//             let table = server_b.table.lock().await;
//             let node = table.get_by_node_id(server_a.node_id);
//             assert!(node.is_some_and(|n| n.revalidation.is_some()));
//         }
//         sleep(Duration::from_millis(2500)).await;

//         // finally, `a`` should not exist anymore
//         let table = server_b.table.lock().await;
//         assert!(table.get_by_node_id(server_a.node_id).is_none());
//         Ok(())
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
//     async fn discovery_server_lookup() -> Result<(), io::Error> {
//         let mut server_a = start_mock_discovery_server(8000, true).await?;
//         let mut server_b = start_mock_discovery_server(8001, true).await?;

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
//         Ok(())
//     }

//     #[tokio::test]
//     /** This test tests the lookup function, the idea is as follows:
//      * - We'll start four discovery servers (`a`, `b`, `c` & `d`)
//      * - `a` will be connected to `b`, `b` will be connected to `c` and `c` will be connected to `d`.
//      * - The server `d` will have its table filled with mock nodes
//      * - We'll run a recursive lookup on server `a` and we expect to end with `b`, `c`, `d` and its mock nodes
//      */
//     async fn discovery_server_recursive_lookup() -> Result<(), io::Error> {
//         let mut server_a = start_mock_discovery_server(8002, true).await?;
//         let mut server_b = start_mock_discovery_server(8003, true).await?;
//         let mut server_c = start_mock_discovery_server(8004, true).await?;
//         let mut server_d = start_mock_discovery_server(8005, true).await?;

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
//         Ok(())
//     }
// #[tokio::test]
// /**
//  * This test verifies the exchange and update of ENR (Ethereum Node Record) messages.
//  * The test follows these steps:
//  *
//  * 1. Start two nodes.
//  * 2. Wait until they establish a connection.
//  * 3. Assert that they exchange their records and store them
//  * 3. Modify the ENR (node record) of one of the nodes.
//  * 4. Send a new ping message and check that an ENR request was triggered.
//  * 5. Verify that the updated node record has been correctly received and stored.
//  */
// async fn discovery_enr_message() -> Result<(), io::Error> {
//     let mut server_a = start_mock_discovery_server(8006, true).await?;
//     let mut server_b = start_mock_discovery_server(8007, true).await?;

//     connect_servers(&mut server_a, &mut server_b).await;

//     // wait some time for the enr request-response finishes
//     sleep(Duration::from_millis(2500)).await;

//     let expected_record =
//         NodeRecord::from_node(server_b.local_node, time_now_unix(), &server_b.signer)
//             .expect("Node record is created from node");

//     let server_a_peer_b = server_a
//         .table
//         .lock()
//         .await
//         .get_by_node_id(server_b.node_id)
//         .cloned()
//         .unwrap();

//     // we only match the pairs, as the signature and seq will change
//     // because they are calculated with the current time
//     assert!(server_a_peer_b.record.decode_pairs() == expected_record.decode_pairs());

//     // Modify server_a's record of server_b with an incorrect TCP port.
//     // This simulates an outdated or incorrect entry in the node table.
//     server_a
//         .table
//         .lock()
//         .await
//         .get_by_node_id_mut(server_b.node_id)
//         .unwrap()
//         .node
//         .tcp_port = 10;

//     // Send a ping from server_b to server_a.
//     // server_a should notice the enr_seq is outdated
//     // and trigger a enr-request to server_b to update the record.
//     ping(
//         &server_b.udp_socket,
//         server_b.addr,
//         server_a.addr,
//         &server_b.signer,
//     )
//     .await;

//     // Wait for the update to propagate.
//     sleep(Duration::from_millis(2500)).await;

//     // Verify that server_a has updated its record of server_b with the correct TCP port.
//     let tcp_port = server_a
//         .table
//         .lock()
//         .await
//         .get_by_node_id(server_b.node_id)
//         .unwrap()
//         .node
//         .tcp_port;

//     assert!(tcp_port == server_b.addr.port());

//     Ok(())
// }
// }
