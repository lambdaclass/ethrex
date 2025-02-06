use super::{
    helpers::{
        current_unix_time, elapsed_time_since, get_msg_expiration_from_seconds, is_msg_expired,
    },
    lookup::Discv4LookupHandler,
    messages::{
        ENRRequestMessage, ENRResponseMessage, Message, NeighborsMessage, Packet, PingMessage,
        PongMessage,
    },
};
use crate::{
    handle_peer_as_initiator,
    kademlia::MAX_NODES_PER_BUCKET,
    types::{Endpoint, Node, NodeRecord},
    KademliaTable, P2PContext,
};
use ethrex_core::H256;
use k256::ecdsa::{signature::hazmat::PrehashVerifier, Signature, VerifyingKey};
use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{net::UdpSocket, sync::MutexGuard};
use tracing::{debug, error};

const MAX_DISC_PACKET_SIZE: usize = 1280;
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

/// Implements the discv4 protocol see: https://github.com/ethereum/devp2p/blob/master/discv4.md
#[derive(Debug, Clone)]
pub struct Discv4Server {
    pub(super) ctx: P2PContext,
    pub(super) udp_socket: Arc<UdpSocket>,
    pub(super) revalidation_interval_seconds: u64,
    pub(super) lookup_interval_minutes: u64,
}

impl Discv4Server {
    /// Initializes a Discv4 UDP socket and creates a new `Discv4Server` instance.  
    /// Returns an error if the socket binding fails.  
    pub async fn try_new(ctx: P2PContext) -> Result<Self, DiscoveryError> {
        let udp_socket = UdpSocket::bind(ctx.local_node.udp_addr())
            .await
            .map_err(DiscoveryError::BindSocket)?;

        Ok(Self {
            ctx,
            udp_socket: Arc::new(udp_socket),
            revalidation_interval_seconds: REVALIDATION_INTERVAL_IN_SECONDS,
            lookup_interval_minutes: PEERS_RANDOM_LOOKUP_TIME_IN_MIN,
        })
    }

    /// Initializes the discovery server. It:
    /// - Spawns tasks to handle incoming messages and revalidate known nodes.
    /// - Loads bootnodes to establish initial peer connections.
    /// - Starts the lookup handler via [`Discv4LookupHandler`] to periodically search for new peers.
    pub async fn start(&self, bootnodes: Vec<Node>) -> Result<(), DiscoveryError> {
        let lookup_handler = Discv4LookupHandler::new(
            self.ctx.clone(),
            self.udp_socket.clone(),
            self.lookup_interval_minutes,
        );

        self.ctx.tracker.spawn({
            let self_clone = self.clone();
            async move { self_clone.receive().await }
        });
        self.ctx.tracker.spawn({
            let self_clone = self.clone();
            async move { self_clone.start_revalidation().await }
        });
        self.load_bootnodes(bootnodes).await;
        lookup_handler.start(10);

        Ok(())
    }

    async fn load_bootnodes(&self, bootnodes: Vec<Node>) {
        for node in bootnodes {
            if let Err(e) = self
                .try_add_peer_and_ping(node, self.ctx.table.lock().await)
                .await
            {
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
                Err(e) => debug!("Could not decode packet: {:?}", e),
                Ok(packet) => {
                    let msg = packet.get_message();
                    let msg_name = msg.to_string();
                    debug!("Message: {:?} from {}", msg, packet.get_node_id());
                    if let Err(e) = self.handle_message(packet, from).await {
                        debug!("Error while processing {} message: {:?}", msg_name, e);
                    };
                }
            }
        }
    }

    async fn handle_message(&self, packet: Packet, from: SocketAddr) -> Result<(), DiscoveryError> {
        match packet.get_message() {
            Message::Ping(msg) => {
                if is_msg_expired(msg.expiration) {
                    return Err(DiscoveryError::MessageExpired);
                };

                let node = Node {
                    ip: from.ip(),
                    udp_port: from.port(),
                    tcp_port: msg.from.tcp_port,
                    node_id: packet.get_node_id(),
                };
                self.pong(packet.get_hash(), node).await?;

                let peer = {
                    let table = self.ctx.table.lock().await;
                    table.get_by_node_id(packet.get_node_id()).cloned()
                };

                let Some(peer) = peer else {
                    self.try_add_peer_and_ping(node, self.ctx.table.lock().await)
                        .await?;
                    return Ok(());
                };

                // if peer was in the table and last ping was 12 hs ago
                //  we need to re ping to re-validate the endpoint proof
                if elapsed_time_since(peer.last_ping) / 3600 >= PROOF_EXPIRATION_IN_HS {
                    self.ping(node, self.ctx.table.lock().await).await?;
                }
                if let Some(enr_seq) = msg.enr_seq {
                    if enr_seq > peer.record.seq && peer.is_proven {
                        debug!("Found outdated enr-seq, sending an enr_request");
                        self.send_enr_request(peer.node, self.ctx.table.lock().await)
                            .await?;
                    }
                }

                Ok(())
            }
            Message::Pong(msg) => {
                if is_msg_expired(msg.expiration) {
                    return Err(DiscoveryError::MessageExpired);
                }

                let peer = {
                    let table = self.ctx.table.lock().await;
                    table.get_by_node_id(packet.get_node_id()).cloned()
                };
                let Some(peer) = peer else {
                    return Err(DiscoveryError::InvalidMessage("not known node".into()));
                };

                let Some(ping_hash) = peer.last_ping_hash else {
                    return Err(DiscoveryError::InvalidMessage(
                        "node did not send a previous ping".into(),
                    ));
                };
                if ping_hash != msg.ping_hash {
                    return Err(DiscoveryError::InvalidMessage(
                        "hash did not match the last corresponding ping".into(),
                    ));
                }

                // all validations went well, mark as answered and start a rlpx connection
                self.ctx
                    .table
                    .lock()
                    .await
                    .pong_answered(peer.node.node_id, current_unix_time());
                if let Some(enr_seq) = msg.enr_seq {
                    if enr_seq > peer.record.seq {
                        debug!("Found outdated enr-seq, send an enr_request");
                        self.send_enr_request(peer.node, self.ctx.table.lock().await)
                            .await?;
                    }
                }

                // We won't initiate a connection if we are already connected.
                // This will typically be the case when revalidating a node.
                if peer.is_connected {
                    return Ok(());
                }

                let ctx = self.ctx.clone();
                self.ctx
                    .tracker
                    .spawn(async move { handle_peer_as_initiator(ctx, peer.node).await });
                Ok(())
            }
            Message::FindNode(msg) => {
                if is_msg_expired(msg.expiration) {
                    return Err(DiscoveryError::MessageExpired);
                };
                let node = {
                    let table = self.ctx.table.lock().await;
                    table.get_by_node_id(packet.get_node_id()).cloned()
                };

                let Some(node) = node else {
                    return Err(DiscoveryError::InvalidMessage("not a known node".into()));
                };
                if !node.is_proven {
                    return Err(DiscoveryError::InvalidMessage("node isn't proven".into()));
                }

                let nodes = {
                    let table = self.ctx.table.lock().await;
                    table.get_closest_nodes(msg.target)
                };
                let nodes_chunks = nodes.chunks(4);
                let expiration = get_msg_expiration_from_seconds(20);

                debug!("Sending neighbors!");
                // we are sending the neighbors in 4 different messages as not to exceed the
                // maximum packet size
                for nodes in nodes_chunks {
                    let neighbors =
                        Message::Neighbors(NeighborsMessage::new(nodes.to_vec(), expiration));
                    let mut buf = Vec::new();
                    neighbors.encode_with_header(&mut buf, &self.ctx.signer);

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
            }
            Message::Neighbors(neighbors_msg) => {
                if is_msg_expired(neighbors_msg.expiration) {
                    return Err(DiscoveryError::MessageExpired);
                };

                let mut table_lock = self.ctx.table.lock().await;

                let Some(node) = table_lock.get_by_node_id_mut(packet.get_node_id()) else {
                    return Err(DiscoveryError::InvalidMessage("not a known node".into()));
                };

                let Some(req) = &mut node.find_node_request else {
                    return Err(DiscoveryError::InvalidMessage(
                        "find node request not sent".into(),
                    ));
                };
                if current_unix_time().saturating_sub(req.sent_at) >= 60 {
                    node.find_node_request = None;
                    return Err(DiscoveryError::InvalidMessage(
                        "find_node request expired after one minute".into(),
                    ));
                }

                let nodes = &neighbors_msg.nodes;
                let total_nodes_sent = req.nodes_sent + nodes.len();

                if total_nodes_sent > MAX_NODES_PER_BUCKET {
                    node.find_node_request = None;
                    return Err(DiscoveryError::InvalidMessage(
                        "sent more than allowed nodes".into(),
                    ));
                }

                // update the number of node_sent
                // and forward the nodes sent if a channel is attached
                req.nodes_sent = total_nodes_sent;
                if let Some(tx) = &req.tx {
                    let _ = tx.send(nodes.clone());
                }

                if total_nodes_sent == MAX_NODES_PER_BUCKET {
                    debug!("Neighbors request has been fulfilled");
                    node.find_node_request = None;
                }

                // release the lock early
                // as we might be a long time pinging all the new nodes
                drop(table_lock);

                debug!("Storing neighbors in our table!");
                for node in nodes {
                    let _ = self
                        .try_add_peer_and_ping(*node, self.ctx.table.lock().await)
                        .await;
                }

                Ok(())
            }
            Message::ENRRequest(msg) => {
                if is_msg_expired(msg.expiration) {
                    return Err(DiscoveryError::MessageExpired);
                }
                let Ok(node_record) =
                    NodeRecord::from_node(self.ctx.local_node, self.ctx.enr_seq, &self.ctx.signer)
                else {
                    return Err(DiscoveryError::InvalidMessage(
                        "could not build local node record".into(),
                    ));
                };
                let msg =
                    Message::ENRResponse(ENRResponseMessage::new(packet.get_hash(), node_record));
                let mut buf = vec![];
                msg.encode_with_header(&mut buf, &self.ctx.signer);

                let bytes_sent = self
                    .udp_socket
                    .send_to(&buf, from)
                    .await
                    .map_err(DiscoveryError::MessageSendFailure)?;

                if bytes_sent != buf.len() {
                    return Err(DiscoveryError::PartialMessageSent);
                }

                Ok(())
            }
            Message::ENRResponse(msg) => {
                let mut table_lock = self.ctx.table.lock().await;
                let peer = table_lock.get_by_node_id_mut(packet.get_node_id());
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
    async fn start_revalidation(&self) {
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
                let mut table_lock = self.ctx.table.lock().await;
                let Some(peer) = table_lock.get_by_node_id_mut(node_id) else {
                    continue;
                };

                if let Some(has_answered) = peer.revalidation {
                    if has_answered {
                        peer.increment_liveness();
                    } else {
                        peer.decrement_liveness();
                    }
                }

                peer.revalidation = None;

                if peer.liveness == 0 {
                    let new_peer = table_lock.replace_peer(node_id);
                    if let Some(new_peer) = new_peer {
                        let _ = self.ping(new_peer.node, table_lock).await;
                    }
                }
            }

            // now send a ping to the least recently pinged peers
            // this might be too expensive to run if our table is filled
            // maybe we could just pick them randomly
            let peers = self
                .ctx
                .table
                .lock()
                .await
                .get_least_recently_pinged_peers(3);
            previously_pinged_peers = HashSet::default();
            for peer in peers {
                debug!("Pinging peer {:?} to re-validate!", peer.node.node_id);
                let _ = self.ping(peer.node, self.ctx.table.lock().await).await;
                previously_pinged_peers.insert(peer.node.node_id);
                let mut table = self.ctx.table.lock().await;
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
    async fn try_add_peer_and_ping<'a>(
        &self,
        node: Node,
        mut table_lock: MutexGuard<'a, KademliaTable>,
    ) -> Result<(), DiscoveryError> {
        // sanity check to make sure we are not storing ourselves
        // a case that may happen in a neighbor message for example
        if node.node_id == self.ctx.local_node.node_id {
            return Ok(());
        }

        if let (Some(peer), true) = table_lock.insert_node(node) {
            self.ping(peer.node, table_lock).await?;
        };
        Ok(())
    }

    async fn ping<'a>(
        &self,
        node: Node,
        mut table_lock: MutexGuard<'a, KademliaTable>,
    ) -> Result<(), DiscoveryError> {
        let mut buf = Vec::new();
        let expiration: u64 = get_msg_expiration_from_seconds(20);
        let from = Endpoint {
            ip: self.ctx.local_node.ip,
            udp_port: self.ctx.local_node.udp_port,
            tcp_port: self.ctx.local_node.tcp_port,
        };
        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let ping =
            Message::Ping(PingMessage::new(from, to, expiration).with_enr_seq(self.ctx.enr_seq));
        ping.encode_with_header(&mut buf, &self.ctx.signer);
        let bytes_sent = self
            .udp_socket
            .send_to(&buf, node.udp_addr())
            .await
            .map_err(DiscoveryError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryError::PartialMessageSent);
        }

        let hash = H256::from_slice(&buf[0..32]);
        table_lock.update_peer_ping(node.node_id, Some(hash), current_unix_time());

        Ok(())
    }

    async fn pong(&self, ping_hash: H256, node: Node) -> Result<(), DiscoveryError> {
        let mut buf = Vec::new();
        let expiration: u64 = get_msg_expiration_from_seconds(20);
        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let pong = Message::Pong(
            PongMessage::new(to, ping_hash, expiration).with_enr_seq(self.ctx.enr_seq),
        );
        pong.encode_with_header(&mut buf, &self.ctx.signer);

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, node.udp_addr())
            .await
            .map_err(DiscoveryError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            Err(DiscoveryError::PartialMessageSent)
        } else {
            Ok(())
        }
    }

    async fn send_enr_request<'a>(
        &self,
        node: Node,
        mut table_lock: MutexGuard<'a, KademliaTable>,
    ) -> Result<(), DiscoveryError> {
        let mut buf = Vec::new();
        let expiration: u64 = get_msg_expiration_from_seconds(20);
        let enr_req = Message::ENRRequest(ENRRequestMessage::new(expiration));
        enr_req.encode_with_header(&mut buf, &self.ctx.signer);

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, node.udp_addr())
            .await
            .map_err(DiscoveryError::MessageSendFailure)?;
        if bytes_sent != buf.len() {
            return Err(DiscoveryError::PartialMessageSent);
        }

        let hash = H256::from_slice(&buf[0..32]);
        if let Some(peer) = table_lock.get_by_node_id_mut(node.node_id) {
            peer.enr_request_hash = Some(hash);
        };

        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::{
        node_id_from_signing_key, rlpx::message::Message as RLPxMessage, serve_p2p_requests,
        MAX_MESSAGES_TO_BROADCAST,
    };
    use ethrex_storage::{EngineType, Store};
    use k256::ecdsa::SigningKey;
    use rand::rngs::OsRng;
    use std::net::{IpAddr, Ipv4Addr};
    use tokio::{sync::Mutex, time::sleep};

    pub async fn insert_random_node_on_custom_bucket(
        table: Arc<Mutex<KademliaTable>>,
        bucket_idx: usize,
    ) {
        let node_id = node_id_from_signing_key(&SigningKey::random(&mut OsRng));
        let node = Node {
            ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            tcp_port: 0,
            udp_port: 0,
            node_id,
        };
        table
            .lock()
            .await
            .insert_node_on_custom_bucket(node, bucket_idx);
    }

    pub async fn fill_table_with_random_nodes(table: Arc<Mutex<KademliaTable>>) {
        for i in 0..256 {
            for _ in 0..16 {
                insert_random_node_on_custom_bucket(table.clone(), i).await;
            }
        }
    }

    pub async fn start_discovery_server(
        udp_port: u16,
        should_start_server: bool,
    ) -> Result<Discv4Server, DiscoveryError> {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), udp_port);
        let signer = SigningKey::random(&mut OsRng);
        let node_id = node_id_from_signing_key(&signer);
        let local_node = Node {
            ip: addr.ip(),
            node_id,
            udp_port,
            tcp_port: udp_port,
        };

        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let table = Arc::new(Mutex::new(KademliaTable::new(node_id)));
        let (broadcast, _) = tokio::sync::broadcast::channel::<(tokio::task::Id, Arc<RLPxMessage>)>(
            MAX_MESSAGES_TO_BROADCAST,
        );
        let tracker = tokio_util::task::TaskTracker::new();

        let ctx = P2PContext {
            local_node,
            enr_seq: current_unix_time(),
            tracker: tracker.clone(),
            signer,
            table,
            storage,
            broadcast,
        };

        let discv4 = Discv4Server::try_new(ctx.clone()).await?;

        if should_start_server {
            tracker.spawn({
                let discv4 = discv4.clone();
                async move {
                    discv4.receive().await;
                }
            });
            // we need to spawn the p2p service, as the nodes will try to connect each other via tcp once bonded
            // if that connection fails, then they are remove themselves from the table, we want them to be bonded for these tests
            ctx.tracker.spawn(serve_p2p_requests(ctx.clone()));
        }

        Ok(discv4)
    }

    /// connects two mock servers by pinging a to b
    pub async fn connect_servers(
        server_a: &mut Discv4Server,
        server_b: &mut Discv4Server,
    ) -> Result<(), DiscoveryError> {
        server_a
            .try_add_peer_and_ping(server_b.ctx.local_node, server_a.ctx.table.lock().await)
            .await?;

        // mark them as connected so that they are not replaced when trying to init tcp connection
        sleep(Duration::from_secs(1)).await;

        Ok(())
    }

    #[tokio::test]
    /** This is a end to end test on the discovery server, the idea is as follows:
     * - We'll start two discovery servers (`a` & `b`) to ping between each other
     * - We'll make `b` ping `a`, and validate that the connection is right
     * - Then we'll wait for a revalidation where we expect everything to be the same
     * - We'll do this five 5 more times
     * - Then we'll stop server `a` so that it doesn't respond to re-validations
     * - We expect server `b` to remove node `a` from its table after 3 re-validations
     * To make this run faster, we'll change the revalidation time to be every 2secs
     */
    async fn discovery_server_revalidation() -> Result<(), DiscoveryError> {
        let mut server_a = start_discovery_server(7998, true).await?;
        let mut server_b = start_discovery_server(7999, true).await?;

        connect_servers(&mut server_a, &mut server_b).await?;

        server_b.revalidation_interval_seconds = 2;

        // start revalidation server
        server_b.ctx.tracker.spawn({
            let server_b = server_b.clone();
            async move { server_b.start_revalidation().await }
        });

        for _ in 0..5 {
            sleep(Duration::from_millis(2500)).await;
            // by now, b should've send a revalidation to a
            let table = server_b.ctx.table.lock().await;
            let node = table.get_by_node_id(server_a.ctx.local_node.node_id);
            assert!(node.is_some_and(|n| n.revalidation.is_some()));
        }

        // make sure that `a` has responded too all the re-validations
        // we can do that by checking the liveness
        {
            let table = server_b.ctx.table.lock().await;
            let node = table.get_by_node_id(server_a.ctx.local_node.node_id);
            assert_eq!(node.map_or(0, |n| n.liveness), 6);
        }

        // now, stopping server `a` is not trivial
        // so we'll instead change its port, so that no one responds
        {
            let mut table = server_b.ctx.table.lock().await;
            let node = table.get_by_node_id_mut(server_a.ctx.local_node.node_id);
            if let Some(node) = node {
                node.node.udp_port = 0
            };
        }

        // now the liveness field should start decreasing until it gets to 0
        // which should happen in 3 re-validations
        for _ in 0..2 {
            sleep(Duration::from_millis(2500)).await;
            let table = server_b.ctx.table.lock().await;
            let node = table.get_by_node_id(server_a.ctx.local_node.node_id);
            assert!(node.is_some_and(|n| n.revalidation.is_some()));
        }
        sleep(Duration::from_millis(2500)).await;

        // finally, `a`` should not exist anymore
        let table = server_b.ctx.table.lock().await;
        assert!(table
            .get_by_node_id(server_a.ctx.local_node.node_id)
            .is_none());
        Ok(())
    }

    #[tokio::test]
    /**
     * This test verifies the exchange and update of ENR (Ethereum Node Record) messages.
     * The test follows these steps:
     *
     * 1. Start two nodes.
     * 2. Wait until they establish a connection.
     * 3. Assert that they exchange their records and store them
     * 3. Modify the ENR (node record) of one of the nodes.
     * 4. Send a new ping message and check that an ENR request was triggered.
     * 5. Verify that the updated node record has been correctly received and stored.
     */
    async fn discovery_enr_message() -> Result<(), DiscoveryError> {
        let mut server_a = start_discovery_server(8006, true).await?;
        let mut server_b = start_discovery_server(8007, true).await?;

        connect_servers(&mut server_a, &mut server_b).await?;

        // wait some time for the enr request-response finishes
        sleep(Duration::from_millis(2500)).await;

        let expected_record = NodeRecord::from_node(
            server_b.ctx.local_node,
            current_unix_time(),
            &server_b.ctx.signer,
        )
        .expect("Node record is created from node");

        let server_a_peer_b = server_a
            .ctx
            .table
            .lock()
            .await
            .get_by_node_id(server_b.ctx.local_node.node_id)
            .cloned()
            .unwrap();

        // we only match the pairs, as the signature and seq will change
        // because they are calculated with the current time
        assert!(server_a_peer_b.record.decode_pairs() == expected_record.decode_pairs());

        // Modify server_a's record of server_b with an incorrect TCP port.
        // This simulates an outdated or incorrect entry in the node table.
        server_a
            .ctx
            .table
            .lock()
            .await
            .get_by_node_id_mut(server_b.ctx.local_node.node_id)
            .unwrap()
            .node
            .tcp_port = 10;

        // update the enr_seq of server_b so that server_a notices it is outdated
        // and sends a request to update it
        server_b.ctx.enr_seq = current_unix_time();

        // Send a ping from server_b to server_a.
        // server_a should notice the enr_seq is outdated
        // and trigger a enr-request to server_b to update the record.
        server_b
            .ping(server_a.ctx.local_node, server_b.ctx.table.lock().await)
            .await?;

        // Wait for the update to propagate.
        sleep(Duration::from_millis(2500)).await;

        // Verify that server_a has updated its record of server_b with the correct TCP port.
        let table_lock = server_a.ctx.table.lock().await;
        let server_a_node_b_record = table_lock
            .get_by_node_id(server_b.ctx.local_node.node_id)
            .unwrap();

        assert!(server_a_node_b_record.node.tcp_port == server_b.ctx.local_node.tcp_port);

        Ok(())
    }
}
