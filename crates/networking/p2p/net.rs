use std::{
    collections::HashSet,
    io,
    net::SocketAddr,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bootnode::BootNode;
use discv4::{
    get_expiration, is_expired, time_now_unix, time_since_in_hs, ENRRequestMessage,
    ENRResponseMessage, FindNodeMessage, Message, NeighborsMessage, Packet, PingMessage,
    PongMessage,
};
use ethrex_core::{H256, H512};
use ethrex_storage::Store;
use k256::{
    ecdsa::{signature::hazmat::PrehashVerifier, Signature, SigningKey, VerifyingKey},
    elliptic_curve::{sec1::ToEncodedPoint, PublicKey},
};
pub use kademlia::KademliaTable;
use kademlia::{bucket_number, MAX_NODES_PER_BUCKET};
use rand::rngs::OsRng;
use rlpx::{connection::RLPxConnection, message::Message as RLPxMessage};
use tokio::{
    net::{TcpListener, TcpSocket, TcpStream, UdpSocket},
    sync::{broadcast, Mutex},
};
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info};
use types::{Endpoint, Node, NodeRecord};

pub mod bootnode;
pub(crate) mod discv4;
pub(crate) mod kademlia;
pub mod peer_channels;
pub mod rlpx;
pub(crate) mod snap;
pub mod sync;
pub mod types;

const MAX_DISC_PACKET_SIZE: usize = 1280;

// Totally arbitrary limit on how
// many messages the connections can queue,
// if we miss messages to broadcast, maybe
// we should bump this limit.
const MAX_MESSAGES_TO_BROADCAST: usize = 1000;

pub fn peer_table(signer: SigningKey) -> Arc<Mutex<KademliaTable>> {
    let local_node_id = node_id_from_signing_key(&signer);
    Arc::new(Mutex::new(KademliaTable::new(local_node_id)))
}

#[allow(clippy::too_many_arguments)]
pub async fn start_network(
    local_node: Node,
    tracker: TaskTracker,
    udp_addr: SocketAddr,
    tcp_addr: SocketAddr,
    bootnodes: Vec<BootNode>,
    signer: SigningKey,
    peer_table: Arc<Mutex<KademliaTable>>,
    storage: Store,
) {
    info!("Starting discovery service at {udp_addr}");
    info!("Listening for requests at {tcp_addr}");
    let (channel_broadcast_send_end, _) = tokio::sync::broadcast::channel::<(
        tokio::task::Id,
        Arc<RLPxMessage>,
    )>(MAX_MESSAGES_TO_BROADCAST);

    tracker.spawn(discover_peers(
        local_node,
        tracker.clone(),
        udp_addr,
        signer.clone(),
        storage.clone(),
        peer_table.clone(),
        bootnodes,
        channel_broadcast_send_end.clone(),
    ));

    tracker.spawn(serve_p2p_requests(
        tracker.clone(),
        tcp_addr,
        signer.clone(),
        storage.clone(),
        peer_table.clone(),
        channel_broadcast_send_end,
    ));
}

#[allow(clippy::too_many_arguments)]
async fn discover_peers(
    local_node: Node,
    tracker: TaskTracker,
    udp_addr: SocketAddr,
    signer: SigningKey,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    bootnodes: Vec<BootNode>,
    connection_broadcast: broadcast::Sender<(tokio::task::Id, Arc<RLPxMessage>)>,
) {
    let udp_socket = match UdpSocket::bind(udp_addr).await {
        Ok(socket) => Arc::new(socket),
        Err(e) => {
            error!("Error binding udp socket {udp_addr}: {e}. Stopping discover peers task");
            return;
        }
    };

    tracker.spawn(discover_peers_server(
        local_node,
        tracker.clone(),
        udp_addr,
        udp_socket.clone(),
        storage,
        table.clone(),
        signer.clone(),
        connection_broadcast,
    ));

    tracker.spawn(peers_revalidation(
        local_node,
        udp_socket.clone(),
        table.clone(),
        signer.clone(),
        REVALIDATION_INTERVAL_IN_SECONDS as u64,
    ));

    discovery_startup(
        local_node,
        udp_socket.clone(),
        table.clone(),
        signer.clone(),
        bootnodes,
    )
    .await;

    // a first initial lookup runs without waiting for the interval
    // so we need to allow some time to the pinged peers to ping us back and acknowledge us
    tokio::time::sleep(Duration::from_secs(10)).await;
    tracker.spawn(peers_lookup(
        tracker.clone(),
        udp_socket.clone(),
        table.clone(),
        signer.clone(),
        node_id_from_signing_key(&signer),
        PEERS_RANDOM_LOOKUP_TIME_IN_MIN as u64 * 60,
    ));
}

#[allow(clippy::too_many_arguments)]
async fn discover_peers_server(
    local_node: Node,
    tracker: TaskTracker,
    udp_addr: SocketAddr,
    udp_socket: Arc<UdpSocket>,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    signer: SigningKey,
    tx_broadcaster_send: broadcast::Sender<(tokio::task::Id, Arc<RLPxMessage>)>,
) {
    let mut buf = vec![0; MAX_DISC_PACKET_SIZE];

    loop {
        let (read, from) = match udp_socket.recv_from(&mut buf).await {
            Ok(result) => result,
            Err(e) => {
                error!(
                    "Error receiving data from socket {udp_addr}: {e}. Stopping discovery server"
                );
                return;
            }
        };
        debug!("Received {read} bytes from {from}");

        match Packet::decode(&buf[..read]) {
            Err(e) => error!("Could not decode packet: {:?}", e),
            Ok(packet) => {
                let msg = packet.get_message();
                debug!("Message: {:?} from {}", msg, packet.get_node_id());

                match msg {
                    Message::Ping(msg) => {
                        if is_expired(msg.expiration) {
                            debug!("Ignoring ping as it is expired.");
                            continue;
                        };
                        let node = Node {
                            ip: from.ip(),
                            udp_port: from.port(),
                            tcp_port: msg.from.tcp_port,
                            node_id: packet.get_node_id(),
                        };
                        let ping_hash = packet.get_hash();
                        pong(&udp_socket, node, ping_hash, &signer).await;
                        let peer = {
                            let table = table.lock().await;
                            table.get_by_node_id(packet.get_node_id()).cloned()
                        };
                        if let Some(peer) = peer {
                            // send a a ping to get an endpoint proof
                            if time_since_in_hs(peer.last_ping) >= PROOF_EXPIRATION_IN_HS as u64 {
                                let hash = ping(&udp_socket, local_node, peer.node, &signer).await;
                                if let Some(hash) = hash {
                                    table
                                        .lock()
                                        .await
                                        .update_peer_ping(peer.node.node_id, Some(hash));
                                }
                            }

                            // if it has updated its record, send a request to update it
                            if let Some(enr_seq) = msg.enr_seq {
                                if enr_seq > peer.record.seq {
                                    debug!("enr-seq outdated, send an enr_request");
                                    let req_hash =
                                        send_enr_request(&udp_socket, from, &signer).await;
                                    table.lock().await.update_peer_enr_seq(
                                        peer.node.node_id,
                                        enr_seq,
                                        req_hash,
                                    );
                                }
                            }
                        } else {
                            let mut table = table.lock().await;
                            if let (Some(peer), true) = table.insert_node(node) {
                                // send a ping to get the endpoint proof from our end
                                let hash = ping(&udp_socket, local_node, node, &signer).await;
                                table.update_peer_ping(peer.node.node_id, hash);
                            }
                        }
                    }
                    Message::Pong(msg) => {
                        let table = table.clone();
                        if is_expired(msg.expiration) {
                            debug!("Ignoring pong as it is expired.");
                            continue;
                        }
                        let peer = {
                            let table = table.lock().await;
                            table.get_by_node_id(packet.get_node_id()).cloned()
                        };
                        if let Some(peer) = peer {
                            if peer.last_ping_hash.is_none() {
                                debug!("Discarding pong as the node did not send a previous ping");
                                continue;
                            }
                            if peer
                                .last_ping_hash
                                .is_some_and(|hash| hash == msg.ping_hash)
                            {
                                table.lock().await.pong_answered(peer.node.node_id);
                                // if it has updated its record, send a request to update it
                                if let Some(enr_seq) = msg.enr_seq {
                                    if enr_seq > peer.record.seq {
                                        debug!("enr-seq outdated, send an enr_request");
                                        let req_hash =
                                            send_enr_request(&udp_socket, from, &signer).await;
                                        table.lock().await.update_peer_enr_seq(
                                            peer.node.node_id,
                                            enr_seq,
                                            req_hash,
                                        );
                                    }
                                }

                                let mut msg_buf = vec![0; read - 32];
                                buf[32..read].clone_into(&mut msg_buf);
                                let signer = signer.clone();
                                let storage = storage.clone();
                                let broadcaster = tx_broadcaster_send.clone();
                                tracker.spawn(async move {
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
                            continue;
                        };
                        let node = {
                            let table = table.lock().await;
                            table.get_by_node_id(packet.get_node_id()).cloned()
                        };
                        if let Some(node) = node {
                            if node.is_proven {
                                let nodes = {
                                    let table = table.lock().await;
                                    table.get_closest_nodes(msg.target)
                                };
                                let nodes_chunks = nodes.chunks(4);
                                let expiration = get_expiration(20);
                                debug!("Sending neighbors!");
                                // we are sending the neighbors in 4 different messages as not to exceed the
                                // maximum packet size
                                for nodes in nodes_chunks {
                                    let neighbors = discv4::Message::Neighbors(
                                        NeighborsMessage::new(nodes.to_vec(), expiration),
                                    );
                                    let mut buf = Vec::new();
                                    neighbors.encode_with_header(&mut buf, &signer);
                                    if let Err(e) = udp_socket.send_to(&buf, from).await {
                                        error!("Could not send Neighbors message {e}");
                                    }
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
                            continue;
                        };

                        let mut nodes_to_insert = None;
                        let mut table = table.lock().await;
                        if let Some(node) = table.get_by_node_id_mut(packet.get_node_id()) {
                            if let Some(req) = &mut node.find_node_request {
                                if time_now_unix().saturating_sub(req.sent_at) >= 60 {
                                    debug!("Ignoring neighbors message as the find_node request expires after one minute");
                                    node.find_node_request = None;
                                    continue;
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
                                if let (Some(peer), true) = table.insert_node(node) {
                                    let ping_hash =
                                        ping(&udp_socket, local_node, peer.node, &signer).await;
                                    table.update_peer_ping(peer.node.node_id, ping_hash);
                                }
                            }
                        }
                    }
                    Message::ENRRequest(msg) => {
                        if is_expired(msg.expiration) {
                            debug!("Ignoring enr-request msg as it is expired.");
                            continue;
                        }
                        // Note we are passing the current timestamp as the sequence number
                        // This is because we are not storing our local_node updates in the db
                        let Ok(node_record) =
                            NodeRecord::from_node(local_node, time_now_unix(), &signer)
                        else {
                            debug!("Ignoring enr-request msg could not build local node record.");
                            continue;
                        };
                        let msg = discv4::Message::ENRResponse(ENRResponseMessage::new(
                            packet.get_hash(),
                            node_record,
                        ));
                        let mut buf = vec![];
                        msg.encode_with_header(&mut buf, &signer);
                        let _ = udp_socket.send_to(&buf, from).await;
                    }
                    Message::ENRResponse(msg) => {
                        let mut table = table.lock().await;
                        let peer = table.get_by_node_id_mut(packet.get_node_id());
                        let Some(peer) = peer else {
                            debug!("Discarding enr-response as we don't know the peer");
                            continue;
                        };

                        let Some(req_hash) = peer.enr_request_hash else {
                            debug!("Discarding enr-response as it wasn't requested");
                            continue;
                        };
                        if req_hash != msg.request_hash {
                            debug!("Discarding enr-response as the request hash did not match");
                            continue;
                        }
                        peer.enr_request_hash = None;

                        if msg.node_record.seq < peer.record.seq {
                            debug!(
                        "Discarding enr-response as the record seq is lower than the one we have"
                    );
                            continue;
                        }

                        let record = msg.node_record.decode_pairs();
                        let Some(id) = record.id else {
                            debug!(
                                "Discarding enr-response as record does not have the `id` field"
                            );
                            continue;
                        };

                        // https://github.com/ethereum/devp2p/blob/master/enr.md#v4-identity-scheme
                        let signature_valid = match id.as_str() {
                            "v4" => {
                                let digest = msg.node_record.get_signature_digest();
                                let Some(public_key) = record.secp256k1 else {
                                    debug!("Discarding enr-response as signature could not be verified because public key was not provided");
                                    continue;
                                };
                                let signature_bytes = msg.node_record.signature.as_bytes();
                                let Ok(signature) = Signature::from_slice(&signature_bytes[0..64])
                                else {
                                    debug!("Discarding enr-response as signature could not be build from msg signature bytes");
                                    continue;
                                };
                                let Ok(verifying_key) =
                                    VerifyingKey::from_sec1_bytes(public_key.as_bytes())
                                else {
                                    debug!("Discarding enr-response as public key could no be built from msg pub key bytes");
                                    continue;
                                };
                                verifying_key.verify_prehash(&digest, &signature).is_ok()
                            }
                            _ => false,
                        };
                        if !signature_valid {
                            debug!(
                                "Discarding enr-response as the signature verification was invalid"
                            );
                            continue;
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
                    }
                }
            }
        }
    }
}

// this is just an arbitrary number, maybe we should get this from some kind of cfg
/// This is a really basic startup and should be improved when we have the nodes stored in the db
/// currently, since we are not storing nodes, the only way to have startup nodes is by providing
/// an array of bootnodes.
async fn discovery_startup(
    local_node: Node,
    udp_socket: Arc<UdpSocket>,
    table: Arc<Mutex<KademliaTable>>,
    signer: SigningKey,
    bootnodes: Vec<BootNode>,
) {
    for bootnode in bootnodes {
        let node = Node {
            ip: bootnode.socket_address.ip(),
            udp_port: bootnode.socket_address.port(),
            // TODO: udp port can differ from tcp port.
            // see https://github.com/lambdaclass/ethrex/issues/905
            tcp_port: bootnode.socket_address.port(),
            node_id: bootnode.node_id,
        };
        table.lock().await.insert_node(node);
        let ping_hash = ping(&udp_socket, local_node, node, &signer).await;
        table
            .lock()
            .await
            .update_peer_ping(bootnode.node_id, ping_hash);
    }
}

const REVALIDATION_INTERVAL_IN_SECONDS: usize = 30; // this is just an arbitrary number, maybe we should get this from some kind of cfg
const PROOF_EXPIRATION_IN_HS: usize = 12;

/// Starts a tokio scheduler that:
/// - performs periodic revalidation of the current nodes (sends a ping to the old nodes). Currently this is configured to happen every [`REVALIDATION_INTERVAL_IN_MINUTES`]
///
/// **Peer revalidation**
///
/// Peers revalidation works in the following manner:
/// 1. Every `REVALIDATION_INTERVAL_IN_SECONDS` we ping the 3 least recently pinged peers
/// 2. In the next iteration we check if they have answered
///    - if they have: we increment the liveness field by one
///    - otherwise we decrement it by the current value / 3.
/// 3. If the liveness field is 0, then we delete it and insert a new one from the replacements table
///
/// See more https://github.com/ethereum/devp2p/blob/master/discv4.md#kademlia-table
async fn peers_revalidation(
    local_node: Node,
    udp_socket: Arc<UdpSocket>,
    table: Arc<Mutex<KademliaTable>>,
    signer: SigningKey,
    interval_time_in_seconds: u64,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(interval_time_in_seconds));
    // peers we have pinged in the previous iteration
    let mut previously_pinged_peers: HashSet<H512> = HashSet::default();

    // first tick starts immediately
    interval.tick().await;

    loop {
        interval.tick().await;
        debug!("Running peer revalidation");

        // first check that the peers we ping have responded
        for node_id in previously_pinged_peers {
            let mut table = table.lock().await;
            if let Some(peer) = table.get_by_node_id_mut(node_id) {
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
                        let ping_hash = ping(&udp_socket, local_node, new_peer.node, &signer).await;
                        table.update_peer_ping(new_peer.node.node_id, ping_hash);
                    }
                }
            }
        }

        // now send a ping to the least recently pinged peers
        // this might be too expensive to run if our table is filled
        // maybe we could just pick them randomly
        let peers = table.lock().await.get_least_recently_pinged_peers(3);
        previously_pinged_peers = HashSet::default();
        for peer in peers {
            let ping_hash = ping(&udp_socket, local_node, peer.node, &signer).await;
            let mut table = table.lock().await;
            table.update_peer_ping_with_revalidation(peer.node.node_id, ping_hash);
            previously_pinged_peers.insert(peer.node.node_id);

            debug!("Pinging peer {:?} to re-validate!", peer.node.node_id);
        }

        debug!("Peer revalidation finished");
    }
}

const PEERS_RANDOM_LOOKUP_TIME_IN_MIN: usize = 30;

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
async fn peers_lookup(
    tracker: TaskTracker,
    udp_socket: Arc<UdpSocket>,
    table: Arc<Mutex<KademliaTable>>,
    signer: SigningKey,
    local_node_id: H512,
    interval_time_in_seconds: u64,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(interval_time_in_seconds));

    loop {
        // Notice that the first tick is immediate,
        // so as soon as the server starts we'll do a lookup with the seeder nodes.
        interval.tick().await;

        debug!("Starting lookup");

        // lookup closest to our pub key
        tracker.spawn(recursive_lookup(
            udp_socket.clone(),
            table.clone(),
            signer.clone(),
            local_node_id,
            local_node_id,
        ));

        // lookup closest to 3 random keys
        for _ in 0..3 {
            let random_pub_key = &SigningKey::random(&mut OsRng);
            tracker.spawn(recursive_lookup(
                udp_socket.clone(),
                table.clone(),
                signer.clone(),
                node_id_from_signing_key(random_pub_key),
                local_node_id,
            ));
        }

        debug!("Lookup finished");
    }
}

async fn recursive_lookup(
    udp_socket: Arc<UdpSocket>,
    table: Arc<Mutex<KademliaTable>>,
    signer: SigningKey,
    target: H512,
    local_node_id: H512,
) {
    let mut asked_peers = HashSet::default();
    // lookups start with the closest from our table
    let closest_nodes = table.lock().await.get_closest_nodes(target);
    let mut seen_peers: HashSet<H512> = HashSet::default();

    seen_peers.insert(local_node_id);
    for node in &closest_nodes {
        seen_peers.insert(node.node_id);
    }

    let mut peers_to_ask: Vec<Node> = closest_nodes;

    loop {
        let (nodes_found, queries) = lookup(
            udp_socket.clone(),
            table.clone(),
            &signer,
            target,
            &mut asked_peers,
            &peers_to_ask,
        )
        .await;

        // only push the peers that have not been seen
        // that is those who have not been yet pushed, which also accounts for
        // those peers that were in the array but have been replaced for closer peers
        for node in nodes_found {
            if !seen_peers.contains(&node.node_id) {
                seen_peers.insert(node.node_id);
                peers_to_ask_push(&mut peers_to_ask, target, node);
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
    udp_socket: Arc<UdpSocket>,
    table: Arc<Mutex<KademliaTable>>,
    signer: &SigningKey,
    target: H512,
    asked_peers: &mut HashSet<H512>,
    nodes_to_ask: &Vec<Node>,
) -> (Vec<Node>, u32) {
    let alpha = 3;
    let mut queries = 0;
    let mut nodes = vec![];

    for node in nodes_to_ask {
        if !asked_peers.contains(&node.node_id) {
            let mut locked_table = table.lock().await;
            if let Some(peer) = locked_table.get_by_node_id_mut(node.node_id) {
                // if the peer has an ongoing find_node request, don't query
                if peer.find_node_request.is_none() {
                    let (tx, mut receiver) = tokio::sync::mpsc::unbounded_channel::<Vec<Node>>();
                    peer.new_find_node_request_with_sender(tx);

                    // Release the lock
                    drop(locked_table);

                    queries += 1;
                    asked_peers.insert(node.node_id);
                    let mut found_nodes = find_node_and_wait_for_response(
                        &udp_socket,
                        SocketAddr::new(node.ip, node.udp_port),
                        signer,
                        target,
                        &mut receiver,
                    )
                    .await;
                    nodes.append(&mut found_nodes)
                }
            }
        }

        if queries == alpha {
            break;
        }
    }

    (nodes, queries)
}

fn peers_to_ask_push(peers_to_ask: &mut Vec<Node>, target: H512, node: Node) {
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

/// Sends a ping to the addr
/// # Returns
/// an optional hash corresponding to the message header hash to account if the send was successful
async fn ping(
    socket: &UdpSocket,
    local_node: Node,
    node: Node,
    signer: &SigningKey,
) -> Option<H256> {
    let mut buf = Vec::new();

    let expiration: u64 = (SystemTime::now() + Duration::from_secs(20))
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let from = Endpoint {
        ip: local_node.ip,
        udp_port: local_node.udp_port,
        tcp_port: local_node.tcp_port,
    };
    let to = Endpoint {
        ip: node.ip,
        udp_port: node.udp_port,
        tcp_port: node.tcp_port,
    };

    let ping =
        discv4::Message::Ping(PingMessage::new(from, to, expiration).with_enr_seq(time_now_unix()));
    ping.encode_with_header(&mut buf, signer);

    // Send ping and log if error
    match socket
        .send_to(&buf, SocketAddr::new(to.ip, to.udp_port))
        .await
    {
        Ok(bytes_sent) => {
            // sanity check to make sure the ping was well sent
            // though idk if this is actually needed or if it might break other stuff
            if bytes_sent == buf.len() {
                return Some(H256::from_slice(&buf[0..32]));
            }
        }
        Err(e) => error!("Unable to send ping: {e}"),
    }

    None
}

async fn find_node_and_wait_for_response(
    socket: &UdpSocket,
    to_addr: SocketAddr,
    signer: &SigningKey,
    target_node_id: H512,
    request_receiver: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<Node>>,
) -> Vec<Node> {
    let expiration: u64 = (SystemTime::now() + Duration::from_secs(20))
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let msg: discv4::Message =
        discv4::Message::FindNode(FindNodeMessage::new(target_node_id, expiration));

    let mut buf = Vec::new();
    msg.encode_with_header(&mut buf, signer);
    let mut nodes = vec![];

    if socket.send_to(&buf, to_addr).await.is_err() {
        return nodes;
    }

    loop {
        // wait as much as 5 seconds for the response
        match tokio::time::timeout(Duration::from_secs(5), request_receiver.recv()).await {
            Ok(Some(mut found_nodes)) => {
                nodes.append(&mut found_nodes);
                if nodes.len() == MAX_NODES_PER_BUCKET {
                    return nodes;
                };
            }
            Ok(None) => {
                return nodes;
            }
            Err(_) => {
                // timeout expired
                return nodes;
            }
        }
    }
}

async fn pong(socket: &UdpSocket, node: Node, ping_hash: H256, signer: &SigningKey) {
    let mut buf = Vec::new();

    let expiration: u64 = (SystemTime::now() + Duration::from_secs(20))
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let to = Endpoint {
        ip: node.ip,
        udp_port: node.udp_port,
        tcp_port: node.tcp_port,
    };
    let pong: discv4::Message = discv4::Message::Pong(
        PongMessage::new(to, ping_hash, expiration).with_enr_seq(time_now_unix()),
    );

    pong.encode_with_header(&mut buf, signer);

    // Send pong and log if error
    if let Err(e) = socket
        .send_to(&buf, SocketAddr::new(node.ip, node.udp_port))
        .await
    {
        error!("Unable to send pong: {e}")
    }
}

async fn send_enr_request(
    socket: &UdpSocket,
    to_addr: SocketAddr,
    signer: &SigningKey,
) -> Option<H256> {
    let mut buf = Vec::new();

    let expiration: u64 = (SystemTime::now() + Duration::from_secs(20))
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let enr_req = discv4::Message::ENRRequest(ENRRequestMessage::new(expiration));

    enr_req.encode_with_header(&mut buf, signer);

    let bytes_sent = socket.send_to(&buf, to_addr).await.ok()?;
    if bytes_sent != buf.len() {
        debug!(
            "ENR request message partially sent: {} out of {} bytes.",
            bytes_sent,
            buf.len()
        );
        return None;
    }

    Some(H256::from_slice(&buf[0..32]))
}

async fn serve_p2p_requests(
    tracker: TaskTracker,
    tcp_addr: SocketAddr,
    signer: SigningKey,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    connection_broadcast: broadcast::Sender<(tokio::task::Id, Arc<RLPxMessage>)>,
) {
    let listener = match listener(tcp_addr) {
        Ok(result) => result,
        Err(e) => {
            error!("Error opening tcp socket at {tcp_addr}: {e}. Stopping p2p server");
            return;
        }
    };
    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(result) => result,
            Err(e) => {
                error!("Error receiving data from tcp socket {tcp_addr}: {e}. Stopping p2p server");
                return;
            }
        };

        tracker.spawn(handle_peer_as_receiver(
            peer_addr,
            signer.clone(),
            stream,
            storage.clone(),
            table.clone(),
            connection_broadcast.clone(),
        ));
    }
}

fn listener(tcp_addr: SocketAddr) -> Result<TcpListener, io::Error> {
    let tcp_socket = TcpSocket::new_v4()?;
    tcp_socket.bind(tcp_addr)?;
    tcp_socket.listen(50)
}

async fn handle_peer_as_receiver(
    peer_addr: SocketAddr,
    signer: SigningKey,
    stream: TcpStream,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    connection_broadcast: broadcast::Sender<(tokio::task::Id, Arc<RLPxMessage>)>,
) {
    let mut conn = RLPxConnection::receiver(signer, stream, storage, connection_broadcast);
    conn.start_peer(peer_addr, table).await;
}

async fn handle_peer_as_initiator(
    signer: SigningKey,
    msg: &[u8],
    node: &Node,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    connection_broadcast: broadcast::Sender<(tokio::task::Id, Arc<RLPxMessage>)>,
) {
    let addr = SocketAddr::new(node.ip, node.tcp_port);
    let stream = match tcp_stream(addr).await {
        Ok(result) => result,
        Err(e) => {
            // TODO We should remove the peer from the table if connection failed
            // but currently it will make the tests fail
            // table.lock().await.replace_peer(node.node_id);
            error!("Error establishing tcp connection with peer at {addr}: {e}");
            return;
        }
    };
    match RLPxConnection::initiator(signer, msg, stream, storage, connection_broadcast) {
        Ok(mut conn) => {
            conn.start_peer(SocketAddr::new(node.ip, node.udp_port), table)
                .await
        }
        Err(e) => {
            // TODO We should remove the peer from the table if connection failed
            // but currently it will make the tests fail
            // table.lock().await.replace_peer(node.node_id);
            error!("Error creating tcp connection with peer at {addr}: {e}")
        }
    };
}

async fn tcp_stream(addr: SocketAddr) -> Result<TcpStream, io::Error> {
    TcpSocket::new_v4()?.connect(addr).await
}

pub fn node_id_from_signing_key(signer: &SigningKey) -> H512 {
    let public_key = PublicKey::from(signer.verifying_key());
    let encoded = public_key.to_encoded_point(false);
    H512::from_slice(&encoded.as_bytes()[1..])
}

/// Shows the amount of connected peers, active peers, and peers suitable for snap sync on a set interval
pub async fn periodically_show_peer_stats(peer_table: Arc<Mutex<KademliaTable>>) {
    const INTERVAL_DURATION: tokio::time::Duration = tokio::time::Duration::from_secs(60);
    let mut interval = tokio::time::interval(INTERVAL_DURATION);
    loop {
        peer_table.lock().await.show_peer_stats();
        interval.tick().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_storage::EngineType;
    use kademlia::bucket_number;
    use rand::rngs::OsRng;
    use std::{
        collections::HashSet,
        net::{IpAddr, Ipv4Addr},
    };
    use tokio::time::sleep;

    async fn insert_random_node_on_custom_bucket(
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

    async fn fill_table_with_random_nodes(table: Arc<Mutex<KademliaTable>>) {
        for i in 0..256 {
            for _ in 0..16 {
                insert_random_node_on_custom_bucket(table.clone(), i).await;
            }
        }
    }

    struct MockServer {
        pub local_node: Node,
        pub addr: SocketAddr,
        pub signer: SigningKey,
        pub table: Arc<Mutex<KademliaTable>>,
        pub node_id: H512,
        pub udp_socket: Arc<UdpSocket>,
    }

    async fn start_mock_discovery_server(
        udp_port: u16,
        should_start_server: bool,
    ) -> Result<MockServer, io::Error> {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), udp_port);
        let signer = SigningKey::random(&mut OsRng);
        let udp_socket = Arc::new(UdpSocket::bind(addr).await?);
        let node_id = node_id_from_signing_key(&signer);
        let storage =
            Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB");
        let table = Arc::new(Mutex::new(KademliaTable::new(node_id)));
        let (channel_broadcast_send_end, _) = tokio::sync::broadcast::channel::<(
            tokio::task::Id,
            Arc<RLPxMessage>,
        )>(MAX_MESSAGES_TO_BROADCAST);
        let local_node = Node {
            ip: addr.ip(),
            tcp_port: addr.port(),
            udp_port: addr.port(),
            node_id,
        };
        let tracker = TaskTracker::new();
        if should_start_server {
            tracker.spawn(discover_peers_server(
                local_node,
                tracker.clone(),
                addr,
                udp_socket.clone(),
                storage.clone(),
                table.clone(),
                signer.clone(),
                channel_broadcast_send_end,
            ));
        }

        Ok(MockServer {
            local_node,
            addr,
            signer,
            table,
            node_id,
            udp_socket,
        })
    }

    /// connects two mock servers by pinging a to b
    async fn connect_servers(server_a: &mut MockServer, server_b: &mut MockServer) {
        let ping_hash = ping(
            &server_a.udp_socket,
            server_a.local_node,
            server_b.local_node,
            &server_a.signer,
        )
        .await;
        {
            let mut table = server_a.table.lock().await;
            table.insert_node(Node {
                ip: server_b.local_node.ip,
                udp_port: server_b.local_node.udp_port,
                tcp_port: server_b.local_node.tcp_port,
                node_id: server_b.node_id,
            });
            table.update_peer_ping(server_b.node_id, ping_hash);
        }
        // allow some time for the server to respond
        sleep(Duration::from_secs(1)).await;
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
    async fn discovery_server_revalidation() -> Result<(), io::Error> {
        let mut server_a = start_mock_discovery_server(7998, true).await?;
        let mut server_b = start_mock_discovery_server(7999, true).await?;

        connect_servers(&mut server_a, &mut server_b).await;

        // start revalidation server
        tokio::spawn(peers_revalidation(
            server_b.local_node,
            server_b.udp_socket.clone(),
            server_b.table.clone(),
            server_b.signer.clone(),
            2,
        ));

        for _ in 0..5 {
            sleep(Duration::from_millis(2500)).await;
            // by now, b should've send a revalidation to a
            let table = server_b.table.lock().await;
            let node = table.get_by_node_id(server_a.node_id);
            assert!(node.is_some_and(|n| n.revalidation.is_some()));
        }

        // make sure that `a` has responded too all the re-validations
        // we can do that by checking the liveness
        {
            let table = server_b.table.lock().await;
            let node = table.get_by_node_id(server_a.node_id);
            assert_eq!(node.map_or(0, |n| n.liveness), 6);
        }

        // now, stopping server `a` is not trivial
        // so we'll instead change its port, so that no one responds
        {
            let mut table = server_b.table.lock().await;
            let node = table.get_by_node_id_mut(server_a.node_id);
            if let Some(node) = node {
                node.node.udp_port = 0
            };
        }

        // now the liveness field should start decreasing until it gets to 0
        // which should happen in 3 re-validations
        for _ in 0..2 {
            sleep(Duration::from_millis(2500)).await;
            let table = server_b.table.lock().await;
            let node = table.get_by_node_id(server_a.node_id);
            assert!(node.is_some_and(|n| n.revalidation.is_some()));
        }
        sleep(Duration::from_millis(2500)).await;

        // finally, `a`` should not exist anymore
        let table = server_b.table.lock().await;
        assert!(table.get_by_node_id(server_a.node_id).is_none());
        Ok(())
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
    async fn discovery_server_lookup() -> Result<(), io::Error> {
        let mut server_a = start_mock_discovery_server(8000, true).await?;
        let mut server_b = start_mock_discovery_server(8001, true).await?;

        fill_table_with_random_nodes(server_a.table.clone()).await;

        // before making the connection, remove a node from the `b` bucket. Otherwise it won't be added
        let b_bucket = bucket_number(server_a.node_id, server_b.node_id);
        let node_id_to_remove = server_a.table.lock().await.buckets()[b_bucket].peers[0]
            .node
            .node_id;
        server_a
            .table
            .lock()
            .await
            .replace_peer_on_custom_bucket(node_id_to_remove, b_bucket);

        connect_servers(&mut server_a, &mut server_b).await;

        // now we are going to run a lookup with us as the target
        let closets_peers_to_b_from_a = server_a
            .table
            .lock()
            .await
            .get_closest_nodes(server_b.node_id);
        let nodes_to_ask = server_b
            .table
            .lock()
            .await
            .get_closest_nodes(server_b.node_id);

        lookup(
            server_b.udp_socket.clone(),
            server_b.table.clone(),
            &server_b.signer,
            server_b.node_id,
            &mut HashSet::default(),
            &nodes_to_ask,
        )
        .await;

        // find_node sent, allow some time for `a` to respond
        sleep(Duration::from_secs(2)).await;

        // now all peers should've been inserted
        for peer in closets_peers_to_b_from_a {
            let table = server_b.table.lock().await;
            assert!(table.get_by_node_id(peer.node_id).is_some());
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
    async fn discovery_server_recursive_lookup() -> Result<(), io::Error> {
        let mut server_a = start_mock_discovery_server(8002, true).await?;
        let mut server_b = start_mock_discovery_server(8003, true).await?;
        let mut server_c = start_mock_discovery_server(8004, true).await?;
        let mut server_d = start_mock_discovery_server(8005, true).await?;

        connect_servers(&mut server_a, &mut server_b).await;
        connect_servers(&mut server_b, &mut server_c).await;
        connect_servers(&mut server_c, &mut server_d).await;

        // now we fill the server_d table with 3 random nodes
        // the reason we don't put more is because this nodes won't respond (as they don't are not real servers)
        // and so we will have to wait for the timeout on each node, which will only slow down the test
        for _ in 0..3 {
            insert_random_node_on_custom_bucket(server_d.table.clone(), 0).await;
        }

        let mut expected_peers = vec![];
        expected_peers.extend(
            server_b
                .table
                .lock()
                .await
                .get_closest_nodes(server_a.node_id),
        );
        expected_peers.extend(
            server_c
                .table
                .lock()
                .await
                .get_closest_nodes(server_a.node_id),
        );
        expected_peers.extend(
            server_d
                .table
                .lock()
                .await
                .get_closest_nodes(server_a.node_id),
        );

        // we'll run a recursive lookup closest to the server itself
        recursive_lookup(
            server_a.udp_socket.clone(),
            server_a.table.clone(),
            server_a.signer.clone(),
            server_a.node_id,
            server_a.node_id,
        )
        .await;

        for peer in expected_peers {
            assert!(server_a
                .table
                .lock()
                .await
                .get_by_node_id(peer.node_id)
                .is_some());
        }
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
    async fn discovery_enr_message() -> Result<(), io::Error> {
        let mut server_a = start_mock_discovery_server(8006, true).await?;
        let mut server_b = start_mock_discovery_server(8007, true).await?;

        connect_servers(&mut server_a, &mut server_b).await;

        // wait some time for the enr request-response finishes
        sleep(Duration::from_millis(2500)).await;

        let expected_record =
            NodeRecord::from_node(server_b.local_node, time_now_unix(), &server_b.signer)
                .expect("Node record is created from node");

        let server_a_peer_b = server_a
            .table
            .lock()
            .await
            .get_by_node_id(server_b.node_id)
            .cloned()
            .unwrap();

        // we only match the pairs, as the signature and seq will change
        // because they are calculated with the current time
        assert!(server_a_peer_b.record.decode_pairs() == expected_record.decode_pairs());

        // Modify server_a's record of server_b with an incorrect TCP port.
        // This simulates an outdated or incorrect entry in the node table.
        server_a
            .table
            .lock()
            .await
            .get_by_node_id_mut(server_b.node_id)
            .unwrap()
            .node
            .tcp_port = 10;

        // Send a ping from server_b to server_a.
        // server_a should notice the enr_seq is outdated
        // and trigger a enr-request to server_b to update the record.
        ping(
            &server_b.udp_socket,
            server_b.local_node,
            server_a.local_node,
            &server_b.signer,
        )
        .await;

        // Wait for the update to propagate.
        sleep(Duration::from_millis(2500)).await;

        // Verify that server_a has updated its record of server_b with the correct TCP port.
        let tcp_port = server_a
            .table
            .lock()
            .await
            .get_by_node_id(server_b.node_id)
            .unwrap()
            .node
            .tcp_port;

        assert!(tcp_port == server_b.addr.port());

        Ok(())
    }
}
