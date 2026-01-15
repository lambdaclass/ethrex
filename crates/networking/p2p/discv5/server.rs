use crate::{
    discv5::{
        codec::Discv5Codec,
        messages::{
            DISTANCES_PER_FIND_NODE_MSG, FindNodeMessage, Handshake, Message, NodesMessage,
            Ordinary, Packet, PacketCodecError, PacketHeader, PacketTrait as _, PingMessage,
            PongMessage, WhoAreYou,
        },
        session::{build_challenge_data, create_id_signature, derive_session_keys},
    },
    metrics::METRICS,
    peer_table::{PeerTable, PeerTableError},
    rlpx::utils::compress_pubkey,
    types::{Node, NodeRecord},
    utils::distance,
};
use bytes::{Bytes, BytesMut};
use ethrex_common::H256;
use ethrex_storage::{Store, error::StoreError};
use futures::StreamExt;
use indexmap::IndexMap;
use rand::{Rng, RngCore, rngs::OsRng};
use secp256k1::{SecretKey, ecdsa::Signature};
use spawned_concurrency::{
    messages::Unused,
    tasks::{
        CastResponse, GenServer, GenServerHandle, InitResult::Success, send_after, send_interval,
        send_message_on, spawn_listener,
    },
};
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;
use tokio_util::udp::UdpFramed;
use tracing::{debug, error, info, trace};

/// Interval between revalidation checks (how often we run the revalidation loop).
const REVALIDATION_CHECK_INTERVAL: Duration = Duration::from_secs(10);
/// Nodes not validated within this interval are candidates for revalidation.
const REVALIDATION_INTERVAL: Duration = Duration::from_secs(30);
/// The initial interval between peer lookups, until the number of peers reaches
/// [target_peers](DiscoverySideCarState::target_peers), or the number of
/// contacts reaches [target_contacts](DiscoverySideCarState::target_contacts).
pub const INITIAL_LOOKUP_INTERVAL_MS: f64 = 100.0; // 10 per second
pub const LOOKUP_INTERVAL_MS: f64 = 600.0; // 100 per minute
const PRUNE_INTERVAL: Duration = Duration::from_secs(5);
/// Timeout for pending messages awaiting WhoAreYou response.
/// Per spec, good timeout is 500ms for single requests, 1s for handshakes.
/// Using 2s to be conservative.
const MESSAGE_CACHE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryServerError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Failed to decode packet")]
    DecodeError(#[from] PacketCodecError),
    #[error("Only partial message was sent")]
    PartialMessageSent,
    #[error("Unknown or invalid contact")]
    InvalidContact,
    #[error(transparent)]
    PeerTable(#[from] PeerTableError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("Internal error {0}")]
    InternalError(String),
    #[error("Cryptography Error {0}")]
    CryptographyError(String),
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Message(Box<Discv5Message>),
    Revalidate,
    Lookup,
    Prune,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

#[derive(Debug)]
pub struct DiscoveryServer {
    local_node: Node,
    local_node_record: NodeRecord,
    signer: SecretKey,
    udp_socket: Arc<UdpSocket>,
    peer_table: PeerTable,
    initial_lookup_interval: f64,
    /// Outgoing message count, used for nonce generation as per the spec.
    counter: u32,
    messages_by_nonce: IndexMap<[u8; 12], (Node, Message, Instant)>,
}

impl DiscoveryServer {
    pub async fn spawn(
        storage: Store,
        local_node: Node,
        signer: SecretKey,
        udp_socket: UdpSocket,
        mut peer_table: PeerTable,
        bootnodes: Vec<Node>,
        // Sending part of the UdpFramed to send messages to remote nodes
        initial_lookup_interval: f64,
    ) -> Result<(), DiscoveryServerError> {
        info!("Starting Discovery Server");

        let mut local_node_record = NodeRecord::from_node(&local_node, 1, &signer)
            .expect("Failed to create local node record");
        if let Ok(fork_id) = storage.get_fork_id().await {
            local_node_record
                .set_fork_id(fork_id, &signer)
                .expect("Failed to set fork_id on local node record");
        }

        let discovery_server = Self {
            local_node: local_node.clone(),
            local_node_record,
            signer,
            udp_socket: Arc::new(udp_socket),
            peer_table: peer_table.clone(),
            initial_lookup_interval,
            counter: 0,
            messages_by_nonce: Default::default(),
        };

        info!(count = bootnodes.len(), "Adding bootnodes");
        peer_table
            .new_contacts(bootnodes, local_node.node_id())
            .await?;

        discovery_server.start();
        Ok(())
    }

    async fn handle_packet(
        &mut self,
        Discv5Message { packet, from }: Discv5Message,
    ) -> Result<(), DiscoveryServerError> {
        // TODO retrieve session info
        match packet.header.flag {
            0x00 => self.handle_ordinary(packet, from).await,
            0x01 => self.handle_who_are_you(packet, from).await,
            0x02 => {
                // Handshake handling not yet implemented
                tracing::info!("Received handsake message");
                Ok(())
            }
            f => {
                tracing::info!("Unexpected flag {f}");
                Err(PacketCodecError::MalformedData)?
            }
        }
    }
    async fn handle_ordinary(
        &mut self,
        packet: Packet,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let src_id = H256::from_slice(&packet.header.authdata);
        let decrypt_key = self
            .peer_table
            .get_session_info(src_id)
            .await?
            .map_or([0; 16], |s| s.inbound_key);

        let ordinary = Ordinary::decode(&packet, &decrypt_key)?;

        tracing::trace!(received = %ordinary.message, from = %src_id, %addr);

        self.handle_message(ordinary, addr).await
    }

    async fn handle_who_are_you(
        &mut self,
        packet: Packet,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let nonce = packet.header.nonce;
        let Some((node, message, _)) = self.messages_by_nonce.swap_remove(&nonce) else {
            tracing::trace!("Received unexpected WhoAreYou packet. Ignoring it");
            return Ok(());
        };
        tracing::trace!(received = "WhoAreYou", from = %node.node_id(), %addr);

        // challenge-data     = masking-iv || static-header || authdata
        let challenge_data = build_challenge_data(
            &packet.masking_iv,
            &packet.header.static_header,
            &packet.header.authdata,
        );

        // ephemeral-key      = random private key generated by node A
        // ephemeral-pubkey   = public key corresponding to ephemeral-key
        let ephemeral_key = SecretKey::new(&mut rand::thread_rng());
        let ephemeral_pubkey = ephemeral_key.public_key(secp256k1::SECP256K1).serialize();

        // dest-pubkey        = public key corresponding to node B's static private key
        let Some(dest_pubkey) = compress_pubkey(node.public_key) else {
            return Err(DiscoveryServerError::CryptographyError(
                "Invalid public key".to_string(),
            ));
        };

        let session = derive_session_keys(
            &ephemeral_key,
            &dest_pubkey,
            &self.local_node.node_id(),
            &node.node_id(),
            &challenge_data,
        );

        // Create the signature included in the message.
        let signature = create_id_signature(
            &self.signer,
            &challenge_data,
            &ephemeral_pubkey,
            &node.node_id(),
        );

        self.peer_table
            .set_session_info(node.node_id(), session)
            .await?;

        // Check enr-seq to decide if we have to send the local ENR in the handshake.
        let whoareyou = WhoAreYou::decode(&packet)?;
        let record = (self.local_node_record.seq != whoareyou.enr_seq)
            .then_some(self.local_node_record.clone());
        self.send_handshake(&message, signature, &ephemeral_pubkey, &node, record)
            .await
    }

    async fn revalidate(&mut self) -> Result<(), DiscoveryServerError> {
        let contacts = self
            .peer_table
            .get_contacts_to_revalidate(REVALIDATION_INTERVAL)
            .await?;

        for contact in contacts {
            if let Err(e) = self.send_ping(&contact.node).await {
                trace!(node = %contact.node.node_id(), err = ?e, "Failed to send revalidation PING");
            }
        }
        Ok(())
    }

    async fn lookup(&mut self) -> Result<(), DiscoveryServerError> {
        if let Some(contact) = self.peer_table.get_contact_for_lookup().await? {
            if let Err(e) = self
                .send_ordinary(
                    &self.get_random_find_node_message(&contact.node),
                    &contact.node,
                )
                .await
            {
                error!(sending = "FindNode", addr = ?&contact.node.udp_addr(), err=?e, "Error sending message");
                self.peer_table
                    .set_disposable(&contact.node.node_id())
                    .await?;
                METRICS.record_new_discarded_node();
            }

            self.peer_table
                .increment_find_node_sent(&contact.node.node_id())
                .await?;
        }
        Ok(())
    }

    fn get_random_find_node_message(&self, node: &Node) -> Message {
        let mut rng = OsRng;
        let target = rng.r#gen();
        let distance = distance(&target, &node.node_id()) as u8;
        let mut distances = Vec::new();
        distances.push(distance as u32);
        for i in 0..DISTANCES_PER_FIND_NODE_MSG / 2 {
            if let Some(d) = distance.checked_add(i + 1) {
                distances.push(d as u32)
            }
            if let Some(d) = distance.checked_sub(i + 1) {
                distances.push(d as u32)
            }
        }
        Message::FindNode(FindNodeMessage {
            req_id: Bytes::from(rng.r#gen::<u64>().to_be_bytes().to_vec()),
            distances,
        })
    }

    async fn prune(&mut self) -> Result<(), DiscoveryServerError> {
        self.peer_table.prune().await?;
        Ok(())
    }

    async fn get_lookup_interval(&mut self) -> Duration {
        let peer_completion = self
            .peer_table
            .target_peers_completion()
            .await
            .unwrap_or_default();
        lookup_interval_function(
            peer_completion,
            self.initial_lookup_interval,
            LOOKUP_INTERVAL_MS,
        )
    }

    async fn handle_ping(
        &mut self,
        ping_message: PingMessage,
        sender_id: H256,
        sender_addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        trace!(from = %sender_id, enr_seq = ping_message.enr_seq, "Received PING");

        // Build PONG response
        let pong = Message::Pong(PongMessage {
            req_id: ping_message.req_id,
            enr_seq: self.local_node_record.seq,
            recipient_addr: sender_addr,
        });

        // Get sender node for sending response (need public key for encryption)
        if let Some(contact) = self.peer_table.get_contact(sender_id).await? {
            self.send_ordinary(&pong, &contact.node).await?;
        } else {
            trace!(from = %sender_id, "Received PING from unknown node, cannot respond");
        }

        Ok(())
    }

    async fn handle_pong(
        &mut self,
        pong_message: PongMessage,
        sender_id: H256,
    ) -> Result<(), DiscoveryServerError> {
        // Validate and record PONG (clears ping_req_id if matches)
        self.peer_table
            .record_pong_received(&sender_id, pong_message.req_id)
            .await?;

        // TODO: If sender's enr_seq > our cached version, request updated ENR

        Ok(())
    }

    async fn handle_find_node(
        &mut self,
        _find_node_message: FindNodeMessage,
    ) -> Result<(), DiscoveryServerError> {
        // TODO: Handle FindNode requests
        // (https://github.com/lambdaclass/ethrex/issues/5779)
        Ok(())
    }

    async fn handle_nodes_message(
        &mut self,
        nodes_message: NodesMessage,
    ) -> Result<(), DiscoveryServerError> {
        // TODO(#3746): check that we requested neighbors from the node
        self.peer_table
            .new_contact_records(nodes_message.nodes, self.local_node.node_id())
            .await?;
        Ok(())
    }

    async fn send_ping(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
        let mut rng = OsRng;
        let req_id = Bytes::from(rng.r#gen::<u64>().to_be_bytes().to_vec());

        let ping = Message::Ping(PingMessage {
            req_id: req_id.clone(),
            enr_seq: self.local_node_record.seq,
        });

        self.send_ordinary(&ping, node).await?;

        // Record ping sent for later PONG verification
        self.peer_table
            .record_ping_sent(&node.node_id(), req_id)
            .await?;

        Ok(())
    }

    async fn send_ordinary(
        &mut self,
        message: &Message,
        node: &Node,
    ) -> Result<(), DiscoveryServerError> {
        let ordinary = Ordinary {
            src_id: self.local_node.node_id(),
            message: message.clone(),
        };
        let encrypt_key = self
            .peer_table
            .get_session_info(node.node_id())
            .await?
            .map_or([0; 16], |s| s.outbound_key);

        let mut rng = OsRng;
        let masking_iv: u128 = rng.r#gen();
        let nonce = self.next_nonce(&mut rng);

        let packet = ordinary.encode(&nonce, masking_iv.to_be_bytes(), &encrypt_key)?;

        let mut buf = BytesMut::new();
        packet.encode(&mut buf, &node.node_id())?;

        let addr = node.udp_addr();
        let _ = self.udp_socket.send_to(&buf, addr).await.inspect_err(
            |e| error!(sending = ?message, addr = ?addr, err=?e, "Error sending message"),
        )?;
        trace!(msg = %message, node = %node.public_key, address= %addr, "Discv5 ordinary message sent");
        self.messages_by_nonce
            .insert(nonce, (node.clone(), message.clone(), Instant::now()));
        Ok(())
    }

    async fn send_handshake(
        &mut self,
        message: &Message,
        signature: Signature,
        eph_pubkey: &[u8],
        node: &Node,
        record: Option<NodeRecord>,
    ) -> Result<(), DiscoveryServerError> {
        let handshake = Handshake {
            src_id: self.local_node.node_id(),
            id_signature: signature.serialize_compact().to_vec(),
            eph_pubkey: eph_pubkey.to_vec(),
            record,
            message: message.clone(),
        };
        let encrypt_key = self
            .peer_table
            .get_session_info(node.node_id())
            .await?
            .map_or([0; 16], |s| s.outbound_key);

        let mut rng = OsRng;
        let masking_iv: u128 = rng.r#gen();
        let nonce = self.next_nonce(&mut rng);

        let packet = handshake.encode(&nonce, masking_iv.to_be_bytes(), &encrypt_key)?;

        let mut buf = BytesMut::new();
        packet.encode(&mut buf, &node.node_id())?;

        let addr = node.udp_addr();
        let _ = self.udp_socket.send_to(&buf, addr).await.inspect_err(
            |e| error!(sending = ?message, addr = ?addr, err=?e, "Error sending message"),
        )?;
        trace!(msg = %message, "Discv5 handshake message sent");
        self.messages_by_nonce
            .insert(nonce, (node.clone(), message.clone(), Instant::now()));
        Ok(())
    }

    /// Generates a 96-bit AES-GCM nonce
    /// ## Spec Recommendation
    /// Encode the current outgoing message count into the first 32 bits of the nonce and fill the remaining 64 bits with random data generated
    /// by a cryptographically secure random number generator.
    fn next_nonce<R: RngCore>(&mut self, rng: &mut R) -> [u8; 12] {
        let counter = self.counter;
        self.counter = self.counter.wrapping_add(1);

        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&counter.to_be_bytes());
        rng.fill_bytes(&mut nonce[4..]);
        nonce
    }

    /// Remove stale entries from the messages_by_nonce cache.
    /// Called periodically to prevent unbounded growth.
    fn cleanup_message_cache(&mut self) {
        let now = Instant::now();
        let before = self.messages_by_nonce.len();
        self.messages_by_nonce
            .retain(|_nonce, (_node, _message, timestamp)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });
        let removed = before - self.messages_by_nonce.len();
        if removed > 0 {
            trace!("Cleaned up {} stale entries from message cache", removed);
        }
    }

    async fn handle_message(
        &mut self,
        ordinary: Ordinary,
        sender_addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        // Ignore packets sent by ourselves
        let sender_id = ordinary.src_id;
        if sender_id == self.local_node.node_id() {
            return Ok(());
        }
        match ordinary.message {
            Message::Ping(ping_message) => {
                self.handle_ping(ping_message, sender_id, sender_addr)
                    .await?
            }
            Message::Pong(pong_message) => {
                self.handle_pong(pong_message, sender_id).await?;
            }
            Message::FindNode(find_node_message) => {
                self.handle_find_node(find_node_message).await?;
            }
            Message::Nodes(nodes_message) => {
                self.handle_nodes_message(nodes_message).await?;
            }
            // We are ignoring these messages currently
            Message::TalkReq(_talk_req_message) => (),
            Message::TalkRes(_talk_res_message) => (),
            Message::Ticket(_ticket_message) => (),
        }
        Ok(())
    }
}

impl GenServer for DiscoveryServer {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = DiscoveryServerError;

    async fn init(
        self,
        handle: &GenServerHandle<Self>,
    ) -> Result<spawned_concurrency::tasks::InitResult<Self>, Self::Error> {
        let stream = UdpFramed::new(
            self.udp_socket.clone(),
            Discv5Codec::new(self.local_node.node_id()),
        );

        spawn_listener(
            handle.clone(),
            stream.filter_map(|result| async move {
                match result {
                    Ok((packet, addr)) => Some(InMessage::Message(Box::new(Discv5Message::from(
                        packet, addr,
                    )))),
                    Err(e) => {
                        debug!(error=?e, "Error receiving Discv5 message");
                        // Skipping invalid data
                        None
                    }
                }
            }),
        );
        send_interval(
            REVALIDATION_CHECK_INTERVAL,
            handle.clone(),
            InMessage::Revalidate,
        );
        send_interval(PRUNE_INTERVAL, handle.clone(), InMessage::Prune);
        let _ = handle.clone().cast(InMessage::Lookup).await;
        send_message_on(handle.clone(), tokio::signal::ctrl_c(), InMessage::Shutdown);

        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::Message(message) => {
                let _ = self
                    .handle_packet(*message)
                    .await
                    // log level trace as we don't want to spam decoding errors from bad peers.
                    .inspect_err(|e| trace!(err=%e, "Error Handling Discovery message"));
            }
            Self::CastMsg::Revalidate => {
                trace!(received = "Revalidate");
                let _ = self
                    .revalidate()
                    .await
                    .inspect_err(|e| error!(err=%e, "Error revalidating discovered peers"));
            }
            Self::CastMsg::Lookup => {
                trace!(received = "Lookup");
                let _ = self
                    .lookup()
                    .await
                    .inspect_err(|e| error!(err=%e, "Error performing Discovery lookup"));

                let interval = self.get_lookup_interval().await;
                send_after(interval, handle.clone(), Self::CastMsg::Lookup);
            }
            Self::CastMsg::Prune => {
                trace!(received = "Prune");
                let _ = self
                    .prune()
                    .await
                    .inspect_err(|e| error!(err=?e, "Error Pruning peer table"));
                self.cleanup_message_cache();
            }
            Self::CastMsg::Shutdown => return CastResponse::Stop,
        }
        CastResponse::NoReply
    }
}

#[derive(Debug, Clone)]
pub struct Discv5Message {
    from: SocketAddr,
    packet: Packet,
}

impl Discv5Message {
    pub fn from(packet: Packet, from: SocketAddr) -> Self {
        Self { from, packet }
    }
}

pub fn lookup_interval_function(progress: f64, lower_limit: f64, upper_limit: f64) -> Duration {
    // Smooth progression curve
    // See https://easings.net/#easeInOutCubic
    let ease_in_out_cubic = if progress < 0.5 {
        4.0 * progress.powf(3.0)
    } else {
        1.0 - ((-2.0 * progress + 2.0).powf(3.0)) / 2.0
    };
    Duration::from_micros(
        // Use `progress` here instead of `ease_in_out_cubic` for a linear function.
        (1000f64 * (ease_in_out_cubic * (upper_limit - lower_limit) + lower_limit)).round() as u64,
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        discv5::server::DiscoveryServer,
        peer_table::PeerTable,
        types::{Node, NodeRecord},
    };
    use rand::{SeedableRng, rngs::StdRng};
    use secp256k1::SecretKey;
    use std::sync::Arc;
    use tokio::net::UdpSocket;

    #[tokio::test]
    async fn test_next_nonce_counter() {
        let mut rng = StdRng::seed_from_u64(7);
        let local_node = Node::from_enode_url(
            "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
        ).expect("Bad enode url");
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();
        let mut server = DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket: Arc::new(UdpSocket::bind("127.0.0.1:30303").await.unwrap()),
            peer_table: PeerTable::spawn(10),
            initial_lookup_interval: 1000.0,
            counter: 0,
            messages_by_nonce: Default::default(),
        };

        let n1 = server.next_nonce(&mut rng);
        let n2 = server.next_nonce(&mut rng);

        assert_eq!(&n1[..4], &[0, 0, 0, 0]);
        assert_eq!(&n2[..4], &[0, 0, 0, 1]);
        assert_ne!(&n1[4..], &n2[4..]);
    }
}
