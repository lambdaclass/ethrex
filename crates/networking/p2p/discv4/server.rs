use crate::network::MAX_MESSAGES_TO_BROADCAST;
use crate::network::P2PContext;
use crate::rlpx::message::RLPxMessage;
use crate::utils::public_key_from_signing_key;
use crate::utils::{is_msg_expired, unmap_ipv4in6_address};
use crate::{
    discv4::messages::{
        ENRRequestMessage, ENRResponseMessage, FindNodeMessage, Message, NeighborsMessage, Packet,
        PacketDecodeErr, PingMessage, PongMessage,
    },
    kademlia::{Contact, Kademlia},
    metrics::METRICS,
    types::{Endpoint, Node, NodeRecord},
    utils::{get_msg_expiration_from_seconds, node_id},
};
use ethrex_blockchain::Blockchain;
use ethrex_common::H32;
use ethrex_common::types::{BlockHeader, ChainConfig};
use ethrex_common::{H512, types::ForkId};
use ethrex_storage::EngineType;
use ethrex_storage::Store;
use ethrex_storage::error::StoreError;
use keccak_hash::H256;
use rand::{rngs::OsRng, seq::IteratorRandom};
use secp256k1::SecretKey;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle},
};
use std::{collections::btree_map::Entry, net::SocketAddr, sync::Arc};
use tokio::time::Duration;
use tokio::{net::UdpSocket, sync::Mutex};
use tracing::{debug, error, info, trace, warn};

use std::net::{IpAddr, Ipv4Addr};
use tokio::time::sleep;

const MAX_DISC_PACKET_SIZE: usize = 1280;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryServerError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Failed to spawn connection handler")]
    ConnectionError(#[from] ConnectionHandlerError),
    #[error("Failed to decode packet")]
    InvalidPacket(#[from] PacketDecodeErr),
    #[error("Failed to send message")]
    MessageSendFailure(std::io::Error),
    #[error("Only partial message was sent")]
    PartialMessageSent,
}

#[derive(Debug, Clone)]
pub struct DiscoveryServer {
    local_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SecretKey,
    udp_socket: Arc<UdpSocket>,
    kademlia: Kademlia,
}

impl DiscoveryServer {
    pub fn new(
        local_node: Node,
        local_node_record: Arc<Mutex<NodeRecord>>,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
    ) -> Self {
        Self {
            local_node,
            local_node_record,
            signer,
            udp_socket,
            kademlia,
        }
    }

    async fn handle_listens(&self) -> Result<(), DiscoveryServerError> {
        let mut buf = vec![0; MAX_DISC_PACKET_SIZE];
        loop {
            let (read, from) = self.udp_socket.recv_from(&mut buf).await?;
            let Ok(packet) = Packet::decode(&buf[..read])
                .inspect_err(|e| warn!(err = ?e, "Failed to decode packet"))
            else {
                continue;
            };
            if packet.get_node_id() == self.local_node.node_id() {
                // Ignore packets sent by ourselves
                continue;
            }
            let mut conn_handle = ConnectionHandler::spawn(self.clone()).await;
            let _ = conn_handle
                .cast(ConnectionHandlerInMessage::from(packet, from))
                .await;
        }
    }

    async fn ping(&self, node: &Node) -> Result<H256, DiscoveryServerError> {
        let mut buf = Vec::new();

        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(20);

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

        let enr_seq = self.local_node_record.lock().await.seq;

        let ping = Message::Ping(PingMessage::new(from, to, expiration).with_enr_seq(enr_seq));

        ping.encode_with_header(&mut buf, &self.signer);

        let ping_hash: [u8; 32] = buf[..32]
            .try_into()
            .expect("first 32 bytes are the message hash");

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, node.udp_addr())
            .await
            .map_err(DiscoveryServerError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

        debug!(sent = "Ping", to = %format!("{:#x}", node.public_key));

        Ok(H256::from(ping_hash))
    }

    async fn pong(&self, ping_hash: H256, node: &Node) -> Result<(), DiscoveryServerError> {
        let mut buf = Vec::new();

        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let enr_seq = self.local_node_record.lock().await.seq;

        let pong = Message::Pong(PongMessage::new(to, ping_hash, expiration).with_enr_seq(enr_seq));

        pong.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self.udp_socket.send_to(&buf, node.udp_addr()).await?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

        debug!(sent = "Pong", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn send_neighbors(
        &self,
        neighbors: Vec<Node>,
        node: &Node,
    ) -> Result<(), DiscoveryServerError> {
        let mut buf = Vec::new();

        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let msg = Message::Neighbors(NeighborsMessage::new(neighbors, expiration));

        msg.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self.udp_socket.send_to(&buf, node.udp_addr()).await?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

        debug!(sent = "Neighbors", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn send_enr_response(
        &self,
        request_hash: H256,
        from: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let node_record = self.local_node_record.lock().await;

        let msg = Message::ENRResponse(ENRResponseMessage::new(request_hash, node_record.clone()));

        let mut buf = vec![];

        msg.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, from)
            .await
            .map_err(DiscoveryServerError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

        Ok(())
    }

    async fn handle_ping(&self, hash: H256, node: Node) -> Result<(), DiscoveryServerError> {
        self.pong(hash, &node).await?;

        let mut table = self.kademlia.table.lock().await;

        match table.entry(node.node_id()) {
            Entry::Occupied(_) => (),
            Entry::Vacant(entry) => {
                let ping_hash = self.ping(&node).await?;
                let contact = entry.insert(Contact::from(node));
                contact.record_sent_ping(ping_hash);
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Listen,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

impl DiscoveryServer {
    pub async fn spawn(
        local_node: Node,
        signer: SecretKey,
        fork_id: &ForkId,
        udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
        bootnodes: Vec<Node>,
    ) -> Result<(), DiscoveryServerError> {
        info!("Starting Discovery Server");

        let local_node_record = Arc::new(Mutex::new(
            NodeRecord::from_node(&local_node, 1, &signer, fork_id.clone())
                .expect("Failed to create local node record"),
        ));

        let state = Self::new(
            local_node,
            local_node_record,
            signer,
            udp_socket,
            kademlia.clone(),
        );

        let mut server = DiscoveryServer::start(state.clone());

        let _ = server.cast(InMessage::Listen).await;

        info!("Pinging {} bootnodes", bootnodes.len());

        let mut table = kademlia.table.lock().await;

        for bootnode in bootnodes {
            let _ = state.ping(&bootnode).await.inspect_err(|e| {
                error!("Failed to ping bootnode: {e}");
            });

            table.insert(bootnode.node_id(), bootnode.into());
        }

        Ok(())
    }
}

impl GenServer for DiscoveryServer {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = DiscoveryServerError;

    async fn handle_cast(
        self,
        message: Self::CastMsg,
        _handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::Listen => {
                let _ = self.handle_listens().await.inspect_err(|e| {
                    error!("Failed to handle listens: {e}");
                });
                CastResponse::Stop
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionHandlerError {}

#[derive(Debug, Clone)]
pub enum ConnectionHandlerInMessage {
    Ping {
        from: SocketAddr,
        message: PingMessage,
        hash: H256,
        sender_public_key: H512,
    },
    Pong {
        message: PongMessage,
        sender_public_key: H512,
    },
    FindNode {
        from: SocketAddr,
        message: FindNodeMessage,
        sender_public_key: H512,
    },
    Neighbors {
        message: NeighborsMessage,
        sender_public_key: H512,
    },
    ENRResponse {
        message: ENRResponseMessage,
        sender_public_key: H512,
    },
    ENRRequest {
        message: ENRRequestMessage,
        from: SocketAddr,
        hash: H256,
        sender_public_key: H512,
    },
}

impl ConnectionHandlerInMessage {
    pub fn from(packet: Packet, from: SocketAddr) -> Self {
        match packet.get_message() {
            Message::Ping(msg) => Self::Ping {
                from,
                message: msg.clone(),
                hash: packet.get_hash(),
                sender_public_key: packet.get_public_key(),
            },
            Message::Pong(msg) => Self::Pong {
                message: *msg,
                sender_public_key: packet.get_public_key(),
            },
            Message::FindNode(msg) => Self::FindNode {
                from,
                message: msg.clone(),
                sender_public_key: packet.get_public_key(),
            },
            Message::Neighbors(msg) => Self::Neighbors {
                message: msg.clone(),
                sender_public_key: packet.get_public_key(),
            },
            Message::ENRResponse(msg) => Self::ENRResponse {
                message: msg.clone(),
                sender_public_key: packet.get_public_key(),
            },
            Message::ENRRequest(msg) => Self::ENRRequest {
                message: *msg,
                from,
                hash: packet.get_hash(),
                sender_public_key: packet.get_public_key(),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConnectionHandlerOutMessage {
    Done,
}

#[derive(Debug, Clone)]
pub struct ConnectionHandler {
    inner_state: DiscoveryServer,
}

impl ConnectionHandler {
    pub fn new(inner_state: DiscoveryServer) -> Self {
        Self { inner_state }
    }

    async fn handle_pong(&self, message: PongMessage, node_id: H256) {
        let mut contacts = self.inner_state.kademlia.table.lock().await;

        // Received a pong from a node we don't know about
        let Some(contact) = contacts.get_mut(&node_id) else {
            return;
        };
        // Received a pong for an unknown ping
        if !contact
            .ping_hash
            .map(|ph| ph == message.ping_hash)
            .unwrap_or(false)
        {
            return;
        }
        contact.ping_hash = None;
    }
}

impl ConnectionHandler {
    pub async fn spawn(inner_state: DiscoveryServer) -> GenServerHandle<Self> {
        ConnectionHandler::new(inner_state).start()
    }
}

impl GenServer for ConnectionHandler {
    type CallMsg = Unused;
    type CastMsg = ConnectionHandlerInMessage;
    type OutMsg = ConnectionHandlerOutMessage;
    type Error = ConnectionHandlerError;

    async fn handle_cast(
        mut self,
        message: Self::CastMsg,
        _handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::Ping {
                from,
                message: msg,
                hash,
                sender_public_key,
            } => {
                trace!(received = "Ping", msg = ?msg, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(msg.expiration) {
                    trace!("Ping expired");
                    return CastResponse::Stop;
                }

                let sender_ip = unmap_ipv4in6_address(from.ip());
                let node = Node::new(sender_ip, from.port(), msg.from.tcp_port, sender_public_key);

                let _ = self.inner_state.handle_ping(hash, node).await.inspect_err(|e| {
                    error!(sent = "Ping", to = %format!("{sender_public_key:#x}"), err = ?e);
                });
            }
            Self::CastMsg::Pong {
                message,
                sender_public_key,
            } => {
                trace!(received = "Pong", msg = ?message, from = %format!("{:#x}", sender_public_key));

                let node_id = node_id(&sender_public_key);

                self.handle_pong(message, node_id).await;
            }
            Self::CastMsg::FindNode {
                from,
                message,
                sender_public_key,
            } => {
                trace!(received = "FindNode", msg = ?message, from = %format!("{:#x}", sender_public_key));

                if is_msg_expired(message.expiration) {
                    trace!("FindNode expired");
                    return CastResponse::Stop;
                }
                let node_id = node_id(&sender_public_key);

                let table = self.inner_state.kademlia.table.lock().await;

                let Some(contact) = table.get(&node_id) else {
                    return CastResponse::Stop;
                };
                if !contact.was_validated() {
                    debug!(received = "FindNode", to = %format!("{sender_public_key:#x}"), "Contact not validated, skipping");
                    return CastResponse::Stop;
                }
                let node = contact.node.clone();

                // Check that the IP address from which we receive the request matches the one we have stored to prevent amplification attacks
                // This prevents an attack vector where the discovery protocol could be used to amplify traffic in a DDOS attack.
                // A malicious actor would send a findnode request with the IP address and UDP port of the target as the source address.
                // The recipient of the findnode packet would then send a neighbors packet (which is a much bigger packet than findnode) to the victim.
                if from.ip() != node.ip {
                    debug!(received = "FindNode", to = %format!("{sender_public_key:#x}"), "IP address mismatch, skipping");
                    return CastResponse::Stop;
                }

                let neighbors = table
                    .iter()
                    .map(|(_, c)| c.node.clone())
                    .choose_multiple(&mut OsRng, 16);

                drop(table);

                // we are sending the neighbors in 2 different messages to avoid exceeding the
                // maximum packet size
                for chunk in neighbors.chunks(8) {
                    let _ = self.inner_state.send_neighbors(chunk.to_vec(), &node).await.inspect_err(|e| {
                        error!(sent = "Neighbors", to = %format!("{sender_public_key:#x}"), err = ?e);
                    });
                }
            }
            Self::CastMsg::Neighbors {
                message: msg,
                sender_public_key,
            } => {
                trace!(received = "Neighbors", msg = ?msg, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(msg.expiration) {
                    trace!("Neighbors expired");
                    return CastResponse::Stop;
                }

                // TODO(#3746): check that we requested neighbors from the node

                let mut contacts = self.inner_state.kademlia.table.lock().await;
                let discarded_contacts = self.inner_state.kademlia.discarded_contacts.lock().await;

                for node in msg.nodes {
                    let node_id = node.node_id();
                    if let Entry::Vacant(vacant_entry) = contacts.entry(node_id) {
                        if !discarded_contacts.contains(&node_id)
                            && node_id != self.inner_state.local_node.node_id()
                        {
                            vacant_entry.insert(Contact::from(node));
                            METRICS.record_new_discovery().await;
                        }
                    };
                }
            }
            Self::CastMsg::ENRRequest {
                message: msg,
                from,
                hash,
                sender_public_key,
            } => {
                trace!(received = "ENRRequest", msg = ?msg, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(msg.expiration) {
                    trace!("ENRRequest expired");
                    return CastResponse::Stop;
                }
                let node_id = node_id(&sender_public_key);

                let mut table = self.inner_state.kademlia.table.lock().await;

                let Some(contact) = table.get(&node_id) else {
                    return CastResponse::Stop;
                };
                if !contact.was_validated() {
                    debug!(received = "ENRRequest", to = %format!("{sender_public_key:#x}"), "Contact not validated, skipping");
                    return CastResponse::Stop;
                }

                if let Err(err) = self.inner_state.send_enr_response(hash, from).await {
                    error!(sent = "ENRResponse", to = %format!("{from}"), err = ?err);
                    return CastResponse::Stop;
                }

                table.entry(node_id).and_modify(|c| c.knows_us = true);
            }
            Self::CastMsg::ENRResponse {
                message: msg,
                sender_public_key,
            } => {
                /*
                    - Look up in kademlia the peer associated with this message
                    - Check that the request hash sent matches the one we sent previously (this requires setting it on enrrequest)
                    - Check that the seq number matches the one we have in our table (this requires setting it).
                    - Check valid signature
                    - Take the `eth` part of the record. If it's None, this peer is garbage; if it's set
                */
                trace!(received = "ENRResponse", msg = ?msg, from = %format!("{sender_public_key:#x}"));
            }
        }
        CastResponse::Stop
    }
}

// pub async fn insert_random_node_on_custom_bucket(
//     table: Arc<Mutex<KademliaTable>>,
//     bucket_idx: usize,
// ) {
//     let public_key = public_key_from_signing_key(&SecretKey::new(&mut OsRng));
//     let node = Node::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0, 0, public_key);
//     table
//         .lock()
//         .await
//         .insert_node_on_custom_bucket(node, bucket_idx);
// }

// pub async fn fill_table_with_random_nodes(table: Arc<Mutex<KademliaTable>>) {
//     for i in 0..256 {
//         for _ in 0..16 {
//             insert_random_node_on_custom_bucket(table.clone(), i).await;
//         }
//     }
// }

// pub async fn start_discovery_server(
//     udp_port: u16,
//     initial_blocks: u64,
//     should_start_server: bool,
// ) -> Result<Discv4Server, DiscoveryError> {
//     let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), udp_port);
//     let signer = SecretKey::new(&mut OsRng);
//     let public_key = public_key_from_signing_key(&signer);
//     let local_node = Node::new(addr.ip(), udp_port, udp_port, public_key);

//     let storage = match initial_blocks {
//         0 => Store::new("temp.db", EngineType::InMemory).expect("Failed to create test DB"),
//         blocks => setup_storage(blocks).await.expect("Storage setup"),
//     };

//     let blockchain = Arc::new(Blockchain::default_with_store(storage.clone()));
//     let table = Arc::new(Mutex::new(KademliaTable::new(local_node.node_id())));
//     let (broadcast, _) = tokio::sync::broadcast::channel::<(tokio::task::Id, Arc<RLPxMessage>)>(
//         MAX_MESSAGES_TO_BROADCAST,
//     );
//     let tracker = tokio_util::task::TaskTracker::new();
//     let local_node_record = Arc::new(Mutex::new(
//         NodeRecord::from_node(&local_node, 1, &signer)
//                        .expect("Node record could not be created from local node"),
//     ));
//     let ctx = P2PContext {
//         local_node,
//         local_node_record,
//         tracker: tracker.clone(),
//         signer,
//         table,
//         storage,
//         blockchain,
//         broadcast,
//         client_version: "ethrex/test".to_string(),
//         based_context: None,
//     };

//     let discv4 = Discv4Server::try_new(ctx.clone()).await?;

//     if should_start_server {
//         tracker.spawn({
//             let discv4 = discv4.clone();
//             async move {
//                 discv4.receive().await;
//             }
//         });
//         // we need to spawn the p2p service, as the nodes will try to connect each other via tcp once bonded
//         // if that connection fails, then they are remove themselves from the table, we want them to be bonded for these tests
//         ctx.tracker.spawn(serve_p2p_requests(ctx.clone()));
//     }

//     Ok(discv4)
// }

// /// connects two mock servers by pinging a to b
// pub async fn connect_servers(
//     server_a: &mut Discv4Server,
//     server_b: &mut Discv4Server,
// ) -> Result<(), DiscoveryError> {
//     server_a
//         .try_add_peer_and_ping(server_b.ctx.local_node.clone())
//         .await?;

//     // allow some time for the server to respond
//     sleep(Duration::from_secs(1)).await;
//     Ok(())
// }

// async fn setup_storage(blocks: u64) -> Result<Store, StoreError> {
//     let store = Store::new("test", EngineType::InMemory)?;

//     let config = ChainConfig {
//         shanghai_time: Some(1),
//         istanbul_block: Some(1),
//         ..Default::default()
//     };
//     store.set_chain_config(&config).await?;

//     for i in 0..blocks {
//         let header = BlockHeader {
//             number: 0,
//             timestamp: i * 5,
//             gas_limit: 100_000_000,
//             gas_used: 0,
//             ..Default::default()
//         };
//         let block_hash = header.hash();
//         store.add_block_header(block_hash, header).await?;
//         store.set_canonical_block(i, block_hash).await?;
//     }
//     store.update_latest_block_number(blocks - 1).await?;
//     Ok(store)
// }

// #[tokio::test]
// /** This is a end to end test on the discovery server, the idea is as follows:
//  * - We'll start two discovery servers (`a` & `b`) to ping between each other
//  * - We'll make `b` ping `a`, and validate that the connection is right
//  * - Then we'll wait for a revalidation where we expect everything to be the same
//  * - We'll do this five 5 more times
//  * - Then we'll stop server `a` so that it doesn't respond to re-validations
//  * - We expect server `b` to remove node `a` from its table after 3 re-validations
//  * To make this run faster, we'll change the revalidation time to be every 2secs
//  */
// async fn discovery_server_revalidation() -> Result<(), DiscoveryError> {
//     let mut server_a = start_discovery_server(7998, 1, true).await?;
//     let mut server_b = start_discovery_server(7999, 1, true).await?;

//     connect_servers(&mut server_a, &mut server_b).await?;

//     server_b.revalidation_interval_seconds = 2;

//     // start revalidation server
//     server_b.ctx.tracker.spawn({
//         let server_b = server_b.clone();
//         async move { server_b.start_revalidation().await }
//     });

//     for _ in 0..5 {
//         sleep(Duration::from_millis(2500)).await;
//         // by now, b should've send a revalidation to a
//         let table = server_b.ctx.table.lock().await;
//         let node = table.get_by_node_id(server_a.ctx.local_node.node_id());
//         assert!(node.is_some_and(|n| n.revalidation.is_some()));
//     }

//     // make sure that `a` has responded too all the re-validations
//     // we can do that by checking the liveness
//     {
//         let table = server_b.ctx.table.lock().await;
//         let node = table.get_by_node_id(server_a.ctx.local_node.node_id());
//         assert_eq!(node.map_or(0, |n| n.liveness), 6);
//     }

//     // now, stopping server `a` is not trivial
//     // so we'll instead change its port, so that no one responds
//     {
//         let mut table = server_b.ctx.table.lock().await;
//         let node = table.get_by_node_id_mut(server_a.ctx.local_node.node_id());
//         if let Some(node) = node {
//             node.node.udp_port = 0
//         };
//     }

//     // now the liveness field should start decreasing until it gets to 0
//     // which should happen in 3 re-validations
//     for _ in 0..2 {
//         sleep(Duration::from_millis(2500)).await;
//         let table = server_b.ctx.table.lock().await;
//         let node = table.get_by_node_id(server_a.ctx.local_node.node_id());
//         assert!(node.is_some_and(|n| n.revalidation.is_some()));
//     }
//     sleep(Duration::from_millis(2500)).await;

//     // finally, `a`` should not exist anymore
//     let table = server_b.ctx.table.lock().await;
//     assert!(
//         table
//             .get_by_node_id(server_a.ctx.local_node.node_id())
//             .is_none()
//     );
//     Ok(())
// }

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
// async fn discovery_enr_message() -> Result<(), DiscoveryError> {
//     let mut server_a = start_discovery_server(8006, 1, true).await?;
//     let mut server_b = start_discovery_server(8007, 1, true).await?;

//     connect_servers(&mut server_a, &mut server_b).await?;

//     // wait some time for the enr request-response finishes
//     sleep(Duration::from_millis(2500)).await;

//     let expected_record = server_b.ctx.local_node_record.lock().await.clone();

//     let server_a_peer_b = server_a
//         .ctx
//         .table
//         .lock()
//         .await
//         .get_by_node_id(server_b.ctx.local_node.node_id())
//         .cloned()
//         .unwrap();

//     // we only match the pairs, as the signature and seq will change
//     // because they are calculated with the current time
//     assert!(server_a_peer_b.record.decode_pairs() == expected_record.decode_pairs());

//     // Modify server_a's record of server_b with an incorrect TCP port.
//     // This simulates an outdated or incorrect entry in the node table.
//     server_a
//         .ctx
//         .table
//         .lock()
//         .await
//         .get_by_node_id_mut(server_b.ctx.local_node.node_id())
//         .unwrap()
//         .node
//         .tcp_port = 10;

//     // update the enr_seq of server_b so that server_a notices it is outdated
//     // and sends a request to update it
//     server_b
//         .ctx
//         .local_node_record
//         .lock()
//         .await
//         .update_seq(&server_b.ctx.signer)
//         .unwrap();

//     // Send a ping from server_b to server_a.
//     // server_a should notice the enr_seq is outdated
//     // and trigger a enr-request to server_b to update the record.
//     server_b.ping(&server_a.ctx.local_node).await?;

//     // Wait for the update to propagate.
//     sleep(Duration::from_millis(2500)).await;

//     // Verify that server_a has updated its record of server_b with the correct TCP port.
//     let table_lock = server_a.ctx.table.lock().await;
//     let server_a_node_b_record = table_lock
//         .get_by_node_id(server_b.ctx.local_node.node_id())
//         .unwrap();

//     assert!(server_a_node_b_record.node.tcp_port == server_b.ctx.local_node.tcp_port);

//     Ok(())
// }

// TODO: SNAP SYNC: reenable this test
// #[tokio::test]
// /**
//  * This test verifies the exchange and validation of eth pairs in the ENR (Ethereum Node Record) messages.
//  * The test follows these steps:
//  *
//  * 1. Start three nodes.
//  * 2. Add a valid fork_id to the nodes a and b
//  * 3. Add a invalid fork_id to the node c
//  * 4. Wait until they establish a connection.
//  * 5. Validate they have exchanged the pairs and validated them
//  * 6. node a and b should be connected
//  * 7. node a and c shouldn't be connected
//  */
// async fn discovery_eth_pair_validation() -> Result<(), DiscoveryError> {
//     let mut server_a = start_discovery_server(8086, 10, true).await?;
//     let mut server_b = start_discovery_server(8087, 10, true).await?;
//     let mut server_c = start_discovery_server(8088, 0, true).await?;

//     let config = ChainConfig {
//         ..Default::default()
//     };
//     server_c
//         .ctx
//         .storage
//         .set_chain_config(&config)
//         .await
//         .unwrap();

//     let fork_id_valid = ForkId {
//         fork_hash: H32::zero(),
//         fork_next: u64::MAX,
//     };

//     let fork_id_invalid = ForkId {
//         fork_hash: H32::zero(),
//         fork_next: 1,
//     };

//     server_a
//         .ctx
//         .local_node_record
//         .lock()
//         .await
//         .set_fork_id(&fork_id_valid, &server_a.ctx.signer)
//         .unwrap();

//     server_b
//         .ctx
//         .local_node_record
//         .lock()
//         .await
//         .set_fork_id(&fork_id_valid, &server_b.ctx.signer)
//         .unwrap();

//     server_c
//         .ctx
//         .local_node_record
//         .lock()
//         .await
//         .set_fork_id(&fork_id_invalid, &server_c.ctx.signer)
//         .unwrap();

//     connect_servers(&mut server_a, &mut server_b).await?;
//     connect_servers(&mut server_a, &mut server_c).await?;

//     // wait some time for the enr request-response finishes
//     sleep(Duration::from_millis(2500)).await;

//     assert!(
//         server_a
//             .ctx
//             .table
//             .lock()
//             .await
//             .get_by_node_id(server_b.ctx.local_node.node_id())
//             .is_some()
//     );

//     assert!(
//         server_a
//             .ctx
//             .table
//             .lock()
//             .await
//             .get_by_node_id(server_c.ctx.local_node.node_id())
//             .is_none()
//     );

//     Ok(())
// }
