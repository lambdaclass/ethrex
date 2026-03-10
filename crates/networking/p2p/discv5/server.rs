use crate::{
    discv5::{
        messages::{
            DISTANCES_PER_FIND_NODE_MSG, FindNodeMessage, Handshake, HandshakeAuthdata, Message,
            NodesMessage, Ordinary, Packet, PacketCodecError, PacketTrait as _, PingMessage,
            PongMessage, WhoAreYou, decrypt_message,
        },
        session::{
            build_challenge_data, create_id_signature, derive_session_keys, verify_id_signature,
        },
    },
    metrics::METRICS,
    peer_table::{DiscoveryProtocol, OutMessage as PeerTableOutMessage, PeerTable, PeerTableError},
    rlpx::utils::compress_pubkey,
    types::{Node, NodeRecord},
    utils::{distance, node_id},
};
use bytes::{Bytes, BytesMut};
use ethrex_common::{H256, H512};
use ethrex_storage::{Store, error::StoreError};
use rand::{Rng, RngCore, rngs::OsRng};
use rustc_hash::{FxHashMap, FxHashSet};
use secp256k1::{PublicKey, SecretKey, ecdsa::Signature};
use spawned_concurrency::{
    messages::Unused,
    tasks::{
        CastResponse, GenServer, GenServerHandle, InitResult::Success, send_after, send_interval,
        send_message_on,
    },
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;
use tracing::{error, info, trace};

/// Maximum number of ENRs per NODES message (limited by UDP packet size).
/// See: https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire.md#nodes-response-0x04
const MAX_ENRS_PER_MESSAGE: usize = 3;
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
/// Minimum interval between WHOAREYOU packets to the same IP address.
/// Prevents amplification attacks where attackers spoof source IPs.
const WHOAREYOU_RATE_LIMIT: Duration = Duration::from_secs(1);
/// Time window for collecting IP votes from PONG recipient_addr.
/// Votes older than this are discarded. Reference: nim-eth uses 5 minutes.
const IP_VOTE_WINDOW: Duration = Duration::from_secs(300);
/// Minimum number of agreeing votes required to update external IP.
const IP_VOTE_THRESHOLD: usize = 3;

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

impl From<ethrex_rlp::error::RLPDecodeError> for DiscoveryServerError {
    fn from(err: ethrex_rlp::error::RLPDecodeError) -> Self {
        DiscoveryServerError::DecodeError(PacketCodecError::from(err))
    }
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
    /// Pending outgoing messages awaiting WhoAreYou response, keyed by nonce.
    pending_by_nonce: FxHashMap<[u8; 12], (Node, Message, Instant)>,
    /// Pending WhoAreYou challenges awaiting Handshake response, keyed by src_id.
    pending_challenges: FxHashMap<H256, (Vec<u8>, Instant)>,
    /// Tracks last WHOAREYOU send time per source IP to prevent amplification attacks.
    whoareyou_rate_limit: FxHashMap<IpAddr, Instant>,
    /// Collects recipient_addr IPs from PONGs for external IP detection via majority voting.
    /// Key: reported IP, Value: set of voter node_ids (each peer votes once per round).
    ip_votes: FxHashMap<IpAddr, FxHashSet<H256>>,
    /// When the current IP voting period started. None if no votes received yet.
    ip_vote_period_start: Option<Instant>,
    /// Whether the first (fast) voting round has completed.
    first_ip_vote_round_completed: bool,
}

impl DiscoveryServer {
    /// Spawn the discv5 discovery server.
    ///
    /// The server receives packets from the multiplexer via GenServer casts.
    /// The `udp_socket` is shared with the multiplexer and used for sending only.
    pub async fn spawn(
        storage: Store,
        local_node: Node,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        mut peer_table: PeerTable,
        bootnodes: Vec<Node>,
        initial_lookup_interval: f64,
    ) -> Result<GenServerHandle<Self>, DiscoveryServerError> {
        info!(protocol = "discv5", "Starting discovery server");

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
            udp_socket,
            peer_table: peer_table.clone(),
            initial_lookup_interval,
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        };

        info!(
            protocol = "discv5",
            count = bootnodes.len(),
            "Adding bootnodes"
        );
        peer_table
            .new_contacts(bootnodes, local_node.node_id(), DiscoveryProtocol::Discv5)
            .await?;

        Ok(discovery_server.start())
    }

    async fn handle_packet(
        &mut self,
        Discv5Message { packet, from }: Discv5Message,
    ) -> Result<(), DiscoveryServerError> {
        // TODO retrieve session info
        match packet.header.flag {
            0x00 => self.handle_ordinary(packet, from).await,
            0x01 => self.handle_who_are_you(packet, from).await,
            0x02 => self.handle_handshake(packet, from).await,
            f => {
                tracing::info!(protocol = "discv5", "Unexpected flag {f}");
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

        // Try to decrypt with existing session key, or send WhoAreYou if no session or decryption fails
        let decrypt_key = self
            .peer_table
            .get_session_info(src_id)
            .await?
            .map(|s| s.inbound_key);

        let ordinary = match decrypt_key {
            Some(key) => match Ordinary::decode(&packet, &key) {
                Ok(ordinary) => ordinary,
                Err(_) => {
                    // Decryption failed - session might be stale, send WhoAreYou
                    trace!(protocol = "discv5", from = %src_id, %addr, "Decryption failed, sending WhoAreYou");
                    return self
                        .send_who_are_you(packet.header.nonce, src_id, addr)
                        .await;
                }
            },
            None => {
                // No session - send WhoAreYou challenge to initiate handshake
                trace!(protocol = "discv5", from = %src_id, %addr, "No session, sending WhoAreYou");
                return self
                    .send_who_are_you(packet.header.nonce, src_id, addr)
                    .await;
            }
        };

        tracing::trace!(protocol = "discv5", received = %ordinary.message, from = %src_id, %addr);

        self.handle_message(ordinary, addr).await
    }

    async fn handle_who_are_you(
        &mut self,
        packet: Packet,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let nonce = packet.header.nonce;
        let Some((node, message, _)) = self.pending_by_nonce.remove(&nonce) else {
            tracing::trace!(
                protocol = "discv5",
                "Received unexpected WhoAreYou packet. Ignoring it"
            );
            return Ok(());
        };
        tracing::trace!(protocol = "discv5", received = "WhoAreYou", from = %node.node_id(), %addr);

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
            true, // we are the initiator
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
            .then(|| self.local_node_record.clone());
        self.send_handshake(message, signature, &ephemeral_pubkey, node, record)
            .await
    }

    async fn handle_handshake(
        &mut self,
        packet: Packet,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        // Parse authdata to extract src_id, signature, ephemeral pubkey, and optional ENR
        let authdata = HandshakeAuthdata::decode(&packet.header.authdata)?;
        let src_id = authdata.src_id;

        // Look up the WhoAreYou challenge we sent, keyed by src_id
        let Some((challenge_data, _)) = self.pending_challenges.remove(&src_id) else {
            trace!(protocol = "discv5", from = %src_id, %addr, "Received unexpected Handshake packet");
            return Ok(());
        };

        // Parse the ephemeral public key
        let eph_pubkey = PublicKey::from_slice(&authdata.eph_pubkey).map_err(|_| {
            DiscoveryServerError::CryptographyError("Invalid ephemeral pubkey".into())
        })?;

        // Get sender's public key from contact or ENR in handshake
        let src_pubkey = if let Some(contact) = self.peer_table.get_contact(src_id).await? {
            compress_pubkey(contact.node.public_key)
        } else if let Some(record) = &authdata.record {
            // Validate ENR signature before trusting its contents
            if !record.verify_signature() {
                trace!(from = %src_id, "Handshake ENR signature verification failed");
                return Ok(());
            }
            let pairs = record.decode_pairs();
            let pubkey = pairs
                .secp256k1
                .and_then(|pk| PublicKey::from_slice(pk.as_bytes()).ok());

            // Verify that the ENR's public key matches the claimed src_id
            if let Some(pk) = &pubkey {
                let uncompressed = pk.serialize_uncompressed();
                let derived_node_id = node_id(&H512::from_slice(&uncompressed[1..]));
                if derived_node_id != src_id {
                    trace!(from = %src_id, "Handshake ENR node_id mismatch");
                    return Ok(());
                }
            }

            pubkey
        } else {
            None
        };

        let Some(src_pubkey) = src_pubkey else {
            trace!(protocol = "discv5", from = %src_id, "Cannot verify handshake: unknown sender public key");
            return Ok(());
        };

        // Parse and verify the id-signature
        let signature = Signature::from_compact(&authdata.id_signature).map_err(|_| {
            DiscoveryServerError::CryptographyError("Invalid signature format".into())
        })?;

        if !verify_id_signature(
            &src_pubkey,
            &challenge_data,
            &authdata.eph_pubkey,
            &self.local_node.node_id(),
            &signature,
        ) {
            trace!(protocol = "discv5", from = %src_id, "Handshake signature verification failed");
            return Ok(());
        }

        // Add the peer to the peer table
        if let Some(record) = &authdata.record {
            self.peer_table
                .new_contact_records(vec![record.clone()], self.local_node.node_id())
                .await?;
        }

        // Derive session keys (we are the recipient, node B)
        let session = derive_session_keys(
            &self.signer,
            &eph_pubkey,
            &src_id,
            &self.local_node.node_id(),
            &challenge_data,
            false, // we are the recipient
        );

        // Store the session
        self.peer_table
            .set_session_info(src_id, session.clone())
            .await?;

        // Decrypt and handle the contained message
        let mut encrypted = packet.encrypted_message.clone();
        decrypt_message(&session.inbound_key, &packet, &mut encrypted)?;
        let message = Message::decode(&encrypted)?;
        trace!(protocol = "discv5", received = %message, from = %src_id, %addr, "Handshake completed");

        // Handle the contained message
        let ordinary = Ordinary { src_id, message };
        self.handle_message(ordinary, addr).await
    }

    async fn revalidate(&mut self) -> Result<(), DiscoveryServerError> {
        let contacts = self
            .peer_table
            .get_contacts_to_revalidate(REVALIDATION_INTERVAL, DiscoveryProtocol::Discv5)
            .await?;

        for contact in contacts {
            if let Err(e) = self.send_ping(&contact.node).await {
                trace!(protocol = "discv5", node = %contact.node.node_id(), err = ?e, "Failed to send revalidation PING");
            }
        }
        Ok(())
    }

    async fn lookup(&mut self) -> Result<(), DiscoveryServerError> {
        if let Some(contact) = self
            .peer_table
            .get_contact_for_lookup(DiscoveryProtocol::Discv5)
            .await?
        {
            let find_node_msg = self.get_random_find_node_message(&contact.node);
            if let Err(e) = self.send_ordinary(find_node_msg, &contact.node).await {
                error!(protocol = "discv5", sending = "FindNode", addr = ?&contact.node.udp_addr(), err=?e, "Error sending message");
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
            req_id: generate_req_id(),
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
        trace!(protocol = "discv5", from = %sender_id, enr_seq = ping_message.enr_seq, "Received PING");

        // Build PONG response
        let pong = Message::Pong(PongMessage {
            req_id: ping_message.req_id,
            enr_seq: self.local_node_record.seq,
            recipient_addr: sender_addr,
        });

        // Get sender node for sending response (need public key for encryption)
        if let Some(contact) = self.peer_table.get_contact(sender_id).await? {
            self.send_ordinary(pong, &contact.node).await?;
        } else {
            trace!(protocol = "discv5", from = %sender_id, "Received PING from unknown node, cannot respond");
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

        // If sender's enr_seq is higher than our cached version, request updated ENR.
        if let Some(contact) = self.peer_table.get_contact(sender_id).await? {
            // If we have no cached record, default to 0 so any PONG with enr_seq > 0
            // triggers a FINDNODE to fetch the ENR we're missing.
            let cached_seq = contact.record.as_ref().map_or(0, |r| r.seq);
            if pong_message.enr_seq > cached_seq {
                trace!(
                    protocol = "discv5",
                    from = %sender_id,
                    cached_seq,
                    pong_seq = pong_message.enr_seq,
                    "ENR seq mismatch, requesting updated ENR (FINDNODE distance 0)"
                );
                let find_node = Message::FindNode(FindNodeMessage {
                    req_id: generate_req_id(),
                    distances: vec![0],
                });
                self.send_ordinary(find_node, &contact.node).await?;
            }
        }

        // Collect recipient_addr for external IP detection
        self.record_ip_vote(pong_message.recipient_addr.ip(), sender_id);

        Ok(())
    }

    async fn handle_find_node(
        &mut self,
        find_node_message: FindNodeMessage,
        sender_id: H256,
        sender_addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        // Validate sender before doing any work. A peer with a session could
        // update its ENR to point to a victim IP; the IP check ensures the
        // response only goes to the address the packet actually came from.
        let contact = match self
            .peer_table
            .validate_contact(&sender_id, sender_addr.ip())
            .await?
        {
            PeerTableOutMessage::Contact(contact) => *contact,
            reason => {
                trace!(from = %sender_id, ?reason, "Rejected FINDNODE");
                return Ok(());
            }
        };

        // Get nodes at the requested distances from our local node
        let nodes = self
            .peer_table
            .get_nodes_at_distances(self.local_node.node_id(), find_node_message.distances)
            .await?;

        // Chunk nodes into multiple NODES messages if needed
        let chunks: Vec<_> = nodes.chunks(MAX_ENRS_PER_MESSAGE).collect();
        if chunks.is_empty() {
            // Send empty response
            let nodes_message = Message::Nodes(NodesMessage {
                req_id: find_node_message.req_id,
                total: 1,
                nodes: vec![],
            });
            self.send_ordinary(nodes_message, &contact.node).await?;
        } else {
            for chunk in &chunks {
                let nodes_message = Message::Nodes(NodesMessage {
                    req_id: find_node_message.req_id.clone(),
                    total: chunks.len() as u64,
                    nodes: chunk.to_vec(),
                });
                self.send_ordinary(nodes_message, &contact.node).await?;
            }
        }

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
        let req_id = generate_req_id();

        let ping = Message::Ping(PingMessage {
            req_id: req_id.clone(),
            enr_seq: self.local_node_record.seq,
        });

        self.send_ordinary(ping, node).await?;

        // Record ping sent for later PONG verification
        self.peer_table
            .record_ping_sent(&node.node_id(), req_id)
            .await?;

        Ok(())
    }

    async fn send_ordinary(
        &mut self,
        message: Message,
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

        self.send_packet(&packet, &node.node_id(), node.udp_addr())
            .await?;
        self.pending_by_nonce
            .insert(nonce, (node.clone(), message, Instant::now()));
        Ok(())
    }

    async fn send_handshake(
        &mut self,
        message: Message,
        signature: Signature,
        eph_pubkey: &[u8],
        node: Node,
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

        self.send_packet(&packet, &node.node_id(), node.udp_addr())
            .await?;
        self.pending_by_nonce
            .insert(nonce, (node, message, Instant::now()));
        Ok(())
    }

    /// Sends a WhoAreYou challenge packet in response to an unverified message.
    /// See: https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire.md#whoareyou-packet-flag--1
    async fn send_who_are_you(
        &mut self,
        nonce: [u8; 12],
        src_id: H256,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        // Rate limit: prevent amplification attacks by limiting WHOAREYOU per IP
        let ip = addr.ip();
        let now = Instant::now();

        if let Some(last_sent) = self.whoareyou_rate_limit.get(&ip)
            && now.duration_since(*last_sent) < WHOAREYOU_RATE_LIMIT
        {
            trace!(
                protocol = "discv5",
                to_ip = %ip,
                "Rate limiting WHOAREYOU packet (amplification attack prevention)"
            );
            return Ok(());
        }

        // Update rate limit tracker
        self.whoareyou_rate_limit.insert(ip, now);

        let mut rng = OsRng;

        // Get the ENR sequence number we have for this node (or 0 if unknown)
        let enr_seq = self
            .peer_table
            .get_contact(src_id)
            .await?
            .map_or(0, |c| c.record.as_ref().map_or(0, |r| r.seq));

        let who_are_you = WhoAreYou {
            id_nonce: rng.r#gen(),
            enr_seq,
        };

        let masking_iv: u128 = rng.r#gen();
        let packet = who_are_you.encode(&nonce, masking_iv.to_be_bytes(), &[0; 16])?;

        // Store challenge data BEFORE sending to avoid race condition with fast responders
        let challenge_data = build_challenge_data(
            &masking_iv.to_be_bytes(),
            &packet.header.static_header,
            &packet.header.authdata,
        );
        self.pending_challenges
            .insert(src_id, (challenge_data, Instant::now()));

        self.send_packet(&packet, &src_id, addr).await?;

        Ok(())
    }

    /// Encodes and sends a packet over UDP.
    async fn send_packet(
        &self,
        packet: &Packet,
        dest_id: &H256,
        addr: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let mut buf = BytesMut::new();
        packet.encode(&mut buf, dest_id)?;
        self.udp_socket.send_to(&buf, addr).await?;
        trace!(protocol = "discv5", to = %dest_id, %addr, flag = packet.header.flag, "Sent packet");
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

    /// Remove stale entries from caches.
    /// Called periodically to prevent unbounded growth.
    fn cleanup_stale_entries(&mut self) {
        let now = Instant::now();

        // Clean pending outgoing messages
        let before_messages = self.pending_by_nonce.len();
        self.pending_by_nonce
            .retain(|_nonce, (_node, _message, timestamp)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });
        let removed_messages = before_messages - self.pending_by_nonce.len();

        // Clean pending WhoAreYou challenges
        let before_challenges = self.pending_challenges.len();
        self.pending_challenges
            .retain(|_src_id, (_challenge_data, timestamp)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });
        let removed_challenges = before_challenges - self.pending_challenges.len();

        // Clean stale WHOAREYOU rate limit entries
        let before_rate_limits = self.whoareyou_rate_limit.len();
        self.whoareyou_rate_limit
            .retain(|_ip, timestamp| now.duration_since(*timestamp) < WHOAREYOU_RATE_LIMIT);
        let removed_rate_limits = before_rate_limits - self.whoareyou_rate_limit.len();

        // Check if IP voting round should end (in case no new votes triggered it)
        if let Some(start) = self.ip_vote_period_start
            && now.duration_since(start) >= IP_VOTE_WINDOW
        {
            self.finalize_ip_vote_round();
        }

        let total_removed = removed_messages + removed_challenges + removed_rate_limits;
        if total_removed > 0 {
            trace!(
                protocol = "discv5",
                "Cleaned up {} stale entries ({} messages, {} challenges, {} rate limits)",
                total_removed,
                removed_messages,
                removed_challenges,
                removed_rate_limits
            );
        }
    }

    /// Records an IP vote from a PONG recipient_addr.
    /// Uses voting rounds: first round ends after 3 votes, subsequent rounds after 5 minutes.
    /// At round end, the IP with most votes wins (if it has at least 3 votes).
    fn record_ip_vote(&mut self, reported_ip: IpAddr, voter_id: H256) {
        // Ignore private IPs - we only care about external IP detection
        if Self::is_private_ip(reported_ip) {
            return;
        }

        let now = Instant::now();

        // Start voting period on first vote
        if self.ip_vote_period_start.is_none() {
            self.ip_vote_period_start = Some(now);
        }

        // Record the vote
        self.ip_votes
            .entry(reported_ip)
            .or_default()
            .insert(voter_id);

        // Check if voting round should end
        let total_votes: usize = self.ip_votes.values().map(|v| v.len()).sum();
        let round_ended = if !self.first_ip_vote_round_completed {
            // First round: end when we have enough votes
            total_votes >= IP_VOTE_THRESHOLD
        } else {
            // Subsequent rounds: end after time window
            self.ip_vote_period_start
                .is_some_and(|start| now.duration_since(start) >= IP_VOTE_WINDOW)
        };

        if round_ended {
            self.finalize_ip_vote_round();
        }
    }

    /// Finalizes the current voting round: picks the IP with most votes and updates if needed.
    fn finalize_ip_vote_round(&mut self) {
        // Find the IP with the most votes
        let winner = self
            .ip_votes
            .iter()
            .map(|(ip, voters)| (*ip, voters.len()))
            .max_by_key(|(_, count)| *count);

        if let Some((winning_ip, vote_count)) = winner {
            // Only update if we have minimum votes and IP differs
            if vote_count >= IP_VOTE_THRESHOLD && winning_ip != self.local_node.ip {
                info!(
                    protocol = "discv5",
                    old_ip = %self.local_node.ip,
                    new_ip = %winning_ip,
                    votes = vote_count,
                    "External IP detected via PONG voting, updating local ENR"
                );
                self.update_local_ip(winning_ip);
            }
        }

        // Reset for next round
        self.ip_votes.clear();
        self.ip_vote_period_start = Some(Instant::now());
        self.first_ip_vote_round_completed = true;
    }

    /// Returns true if the IP is private/local (not useful for external connectivity).
    /// For IPv6, mirrors the checks from `Ipv6Addr::is_global` (nightly-only).
    fn is_private_ip(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    // unique local (fc00::/7)
                    || (v6.segments()[0] & 0xfe00) == 0xfc00
                    // link-local (fe80::/10)
                    || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
        }
    }

    /// Updates local node IP and re-signs the ENR with incremented seq.
    fn update_local_ip(&mut self, new_ip: IpAddr) {
        // Build ENR from a node with the new IP
        let mut updated_node = self.local_node.clone();
        updated_node.ip = new_ip;
        let new_seq = self.local_node_record.seq + 1;
        let Ok(mut new_record) = NodeRecord::from_node(&updated_node, new_seq, &self.signer) else {
            error!(%new_ip, "Failed to create new ENR for IP update");
            return;
        };
        // Preserve fork_id if present
        if let Some(fork_id) = self.local_node_record.decode_pairs().eth
            && new_record.set_fork_id(fork_id, &self.signer).is_err()
        {
            error!(%new_ip, "Failed to set fork_id in new ENR, aborting IP update");
            return;
        }
        self.local_node.ip = new_ip;
        self.local_node_record = new_record;
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
                self.handle_find_node(find_node_message, sender_id, sender_addr)
                    .await?;
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
                    .inspect_err(
                        |e| trace!(protocol = "discv5", err=%e, "Error Handling Discovery message"),
                    );
            }
            Self::CastMsg::Revalidate => {
                trace!(protocol = "discv5", received = "Revalidate");
                let _ = self.revalidate().await.inspect_err(
                    |e| error!(protocol = "discv5", err=%e, "Error revalidating discovered peers"),
                );
            }
            Self::CastMsg::Lookup => {
                trace!(protocol = "discv5", received = "Lookup");
                let _ = self.lookup().await.inspect_err(
                    |e| error!(protocol = "discv5", err=%e, "Error performing Discovery lookup"),
                );

                let interval = self.get_lookup_interval().await;
                send_after(interval, handle.clone(), Self::CastMsg::Lookup);
            }
            Self::CastMsg::Prune => {
                trace!(protocol = "discv5", received = "Prune");
                let _ = self.prune().await.inspect_err(
                    |e| error!(protocol = "discv5", err=?e, "Error Pruning peer table"),
                );
                self.cleanup_stale_entries();
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

fn generate_req_id() -> Bytes {
    let mut rng = OsRng;
    Bytes::from(rng.r#gen::<u64>().to_be_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use crate::{
        discv5::{messages::PongMessage, server::DiscoveryServer, session::Session},
        peer_table::PeerTable,
        types::{Node, NodeRecord},
    };
    use bytes::Bytes;
    use ethrex_common::H256;
    use ethrex_storage::{EngineType, Store};
    use rand::{SeedableRng, rngs::StdRng};
    use rustc_hash::FxHashSet;
    use secp256k1::SecretKey;
    use std::{
        net::{IpAddr, SocketAddr},
        sync::Arc,
        time::Instant,
    };
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
            peer_table: PeerTable::spawn(
                10,
                Store::new("", EngineType::InMemory).expect("Failed to create store"),
            ),
            initial_lookup_interval: 1000.0,
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        };

        let n1 = server.next_nonce(&mut rng);
        let n2 = server.next_nonce(&mut rng);

        assert_eq!(&n1[..4], &[0, 0, 0, 0]);
        assert_eq!(&n2[..4], &[0, 0, 0, 1]);
        assert_ne!(&n1[4..], &n2[4..]);
    }

    #[tokio::test]
    async fn test_whoareyou_rate_limiting() {
        let local_node = Node::from_enode_url(
            "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
        ).expect("Bad enode url");
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();
        // Use port 0 to let the OS assign an available port
        let mut server = DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket: Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
            peer_table: PeerTable::spawn(
                10,
                Store::new("", EngineType::InMemory).expect("Failed to create store"),
            ),
            initial_lookup_interval: 1000.0,
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        };

        let nonce = [0u8; 12];
        let addr: SocketAddr = "192.168.1.1:30303".parse().unwrap();
        let src_id1 = H256::from_low_u64_be(1);
        let src_id2 = H256::from_low_u64_be(2);
        let src_id3 = H256::from_low_u64_be(3);

        // Initially, rate limit map should be empty
        assert!(server.whoareyou_rate_limit.is_empty());

        // First call should NOT be rate limited
        let _ = server.send_who_are_you(nonce, src_id1, addr).await;

        // Should have recorded the IP in rate limit map
        assert!(server.whoareyou_rate_limit.contains_key(&addr.ip()));
        // Should have added a pending challenge (proves packet was processed)
        assert!(server.pending_challenges.contains_key(&src_id1));

        // Second call with SAME IP should be rate limited
        let _ = server.send_who_are_you(nonce, src_id2, addr).await;

        // Should NOT have added a pending challenge for src_id2 (rate limited)
        assert!(!server.pending_challenges.contains_key(&src_id2));

        // Call with DIFFERENT IP should NOT be rate limited
        let addr2: SocketAddr = "192.168.1.2:30303".parse().unwrap();
        let _ = server.send_who_are_you(nonce, src_id3, addr2).await;

        // Should have added a pending challenge for the different IP
        assert!(server.pending_challenges.contains_key(&src_id3));
        // Both IPs should now be in the rate limit map
        assert_eq!(server.whoareyou_rate_limit.len(), 2);
    }

    #[tokio::test]
    async fn test_enr_update_request_on_pong() {
        // Create local node
        let local_node = Node::from_enode_url(
            "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
        ).expect("Bad enode url");
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();

        // Create remote node - use a template node for IP/ports, but the record will use remote_signer's key
        let remote_signer = SecretKey::new(&mut rand::rngs::OsRng);
        let remote_node_template = Node::from_enode_url(
            "enode://a448f24c6d18e575453db127a3d8eeeea3e3426f0db43bd52067d85cc5a1e87ad09f44b2bbaa66bb3a8c47cff8082ca4cde4b03f5ba52c1e92b3d2b9125d6da5@127.0.0.1:30304",
        ).expect("Bad enode url");

        // Create NodeRecord for the remote node with seq = 5
        // Note: from_node uses remote_signer's public key, so we derive node_id from the record
        let remote_record =
            NodeRecord::from_node(&remote_node_template, 5, &remote_signer).unwrap();
        let remote_node = Node::from_enr(&remote_record).expect("Should create node from record");
        let remote_node_id = remote_node.node_id();

        let mut peer_table = PeerTable::spawn(
            10,
            Store::new("", EngineType::InMemory).expect("Failed to create store"),
        );

        // Add the remote node as a contact with its ENR record
        peer_table
            .new_contact_records(vec![remote_record], local_node.node_id())
            .await
            .unwrap();

        // Set up a session for the remote node (required for send_ordinary)
        let session = Session {
            outbound_key: [0u8; 16],
            inbound_key: [0u8; 16],
        };
        peer_table
            .set_session_info(remote_node_id, session)
            .await
            .unwrap();

        let mut server = DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket: Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
            peer_table,
            initial_lookup_interval: 1000.0,
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        };

        // Verify the contact was added
        let contact = server.peer_table.get_contact(remote_node_id).await.unwrap();
        assert!(
            contact.is_some(),
            "Contact should have been added to peer_table"
        );
        let contact = contact.unwrap();
        assert_eq!(
            contact.record.as_ref().map(|r| r.seq),
            Some(5),
            "Contact should have ENR with seq=5"
        );

        // Test 1: PONG with same enr_seq should NOT trigger FINDNODE
        let pong_same_seq = PongMessage {
            req_id: Bytes::from(vec![1, 2, 3]),
            enr_seq: 5, // Same as cached
            recipient_addr: "127.0.0.1:30303".parse().unwrap(),
        };
        let initial_pending_count = server.pending_by_nonce.len();
        server
            .handle_pong(pong_same_seq, remote_node_id)
            .await
            .expect("handle_pong failed for matching enr_seq");
        // No new message should be pending (no FINDNODE sent)
        assert_eq!(server.pending_by_nonce.len(), initial_pending_count);

        // Test 2: PONG with higher enr_seq should trigger FINDNODE
        let pong_higher_seq = PongMessage {
            req_id: Bytes::from(vec![4, 5, 6]),
            enr_seq: 10, // Higher than cached (5)
            recipient_addr: "127.0.0.1:30303".parse().unwrap(),
        };
        server
            .handle_pong(pong_higher_seq, remote_node_id)
            .await
            .expect("handle_pong failed for higher enr_seq");
        // A new message should be pending (FINDNODE sent)
        assert_eq!(server.pending_by_nonce.len(), initial_pending_count + 1);

        // Test 3: PONG with lower enr_seq should NOT trigger FINDNODE
        let pong_lower_seq = PongMessage {
            req_id: Bytes::from(vec![7, 8, 9]),
            enr_seq: 3, // Lower than cached (5)
            recipient_addr: "127.0.0.1:30303".parse().unwrap(),
        };
        server
            .handle_pong(pong_lower_seq, remote_node_id)
            .await
            .expect("handle_pong failed for lower enr_seq");
        // No new message should be pending
        assert_eq!(server.pending_by_nonce.len(), initial_pending_count + 1);
    }

    #[tokio::test]
    async fn test_ip_voting_updates_ip_on_threshold() {
        let local_node = Node::from_enode_url(
            "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
        ).expect("Bad enode url");
        let original_ip = local_node.ip;
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();
        let original_seq = local_node_record.seq;

        let mut server = DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket: Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
            peer_table: PeerTable::spawn(
                10,
                Store::new("", EngineType::InMemory).expect("Failed to create store"),
            ),
            initial_lookup_interval: 1000.0,
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        };

        let new_ip: IpAddr = "203.0.113.50".parse().unwrap();
        let voter1 = H256::from_low_u64_be(1);
        let voter2 = H256::from_low_u64_be(2);
        let voter3 = H256::from_low_u64_be(3);

        // Vote 1 - should not update yet
        server.record_ip_vote(new_ip, voter1);
        assert_eq!(server.local_node.ip, original_ip);
        assert_eq!(server.ip_votes.get(&new_ip).map(|v| v.len()), Some(1));

        // Vote 2 from different peer - should not update yet
        server.record_ip_vote(new_ip, voter2);
        assert_eq!(server.local_node.ip, original_ip);
        assert_eq!(server.ip_votes.get(&new_ip).map(|v| v.len()), Some(2));

        // Vote 3 from different peer - should trigger update (threshold reached)
        server.record_ip_vote(new_ip, voter3);
        assert_eq!(server.local_node.ip, new_ip);
        assert_eq!(server.local_node_record.seq, original_seq + 1);
        // Votes should be cleared after update
        assert!(server.ip_votes.is_empty());
    }

    #[tokio::test]
    async fn test_ip_voting_same_peer_votes_once() {
        let local_node = Node::from_enode_url(
            "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
        ).expect("Bad enode url");
        let original_ip = local_node.ip;
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();

        let mut server = DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket: Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
            peer_table: PeerTable::spawn(
                10,
                Store::new("", EngineType::InMemory).expect("Failed to create store"),
            ),
            initial_lookup_interval: 1000.0,
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        };

        let new_ip: IpAddr = "203.0.113.50".parse().unwrap();
        let same_voter = H256::from_low_u64_be(1);

        // Same peer voting 3 times should only count as 1 vote
        server.record_ip_vote(new_ip, same_voter);
        server.record_ip_vote(new_ip, same_voter);
        server.record_ip_vote(new_ip, same_voter);

        // Should still only have 1 vote (same peer)
        assert_eq!(server.ip_votes.get(&new_ip).map(|v| v.len()), Some(1));
        // IP should not change
        assert_eq!(server.local_node.ip, original_ip);
    }

    #[tokio::test]
    async fn test_ip_voting_no_update_if_same_ip() {
        let local_node = Node::from_enode_url(
            "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
        ).expect("Bad enode url");
        let original_ip = local_node.ip;
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();
        let original_seq = local_node_record.seq;

        let mut server = DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket: Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
            peer_table: PeerTable::spawn(
                10,
                Store::new("", EngineType::InMemory).expect("Failed to create store"),
            ),
            initial_lookup_interval: 1000.0,
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        };

        let voter1 = H256::from_low_u64_be(1);
        let voter2 = H256::from_low_u64_be(2);
        let voter3 = H256::from_low_u64_be(3);

        // Vote 3 times for the same IP we already have (from different peers)
        // This triggers the first round to end after 3 votes
        server.record_ip_vote(original_ip, voter1);
        server.record_ip_vote(original_ip, voter2);
        server.record_ip_vote(original_ip, voter3);

        // IP and seq should remain unchanged (winner is our current IP)
        assert_eq!(server.local_node.ip, original_ip);
        assert_eq!(server.local_node_record.seq, original_seq);
        // Votes cleared because round ended (even though no IP change)
        assert!(server.ip_votes.is_empty());
        // First round should now be completed
        assert!(server.first_ip_vote_round_completed);
    }

    #[tokio::test]
    async fn test_ip_voting_split_votes_no_update() {
        // Tests that when votes are split and no IP reaches threshold, IP is not updated
        let local_node = Node::from_enode_url(
            "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
        ).expect("Bad enode url");
        let original_ip = local_node.ip;
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();

        let mut server = DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket: Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
            peer_table: PeerTable::spawn(
                10,
                Store::new("", EngineType::InMemory).expect("Failed to create store"),
            ),
            initial_lookup_interval: 1000.0,
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        };

        let ip1: IpAddr = "203.0.113.50".parse().unwrap();
        let ip2: IpAddr = "203.0.113.51".parse().unwrap();
        let voter1 = H256::from_low_u64_be(1);
        let voter2 = H256::from_low_u64_be(2);
        let voter3 = H256::from_low_u64_be(3);

        // First round: votes are split between two IPs
        // Vote 1: ip1
        server.record_ip_vote(ip1, voter1);
        assert_eq!(server.local_node.ip, original_ip); // No change yet

        // Vote 2: ip2
        server.record_ip_vote(ip2, voter2);
        assert_eq!(server.local_node.ip, original_ip); // No change yet

        // Vote 3: ip1 - triggers first round end (3 total votes)
        // ip1 has 2 votes, ip2 has 1 vote, but ip1 doesn't reach threshold of 3
        server.record_ip_vote(ip1, voter3);
        // IP should NOT change because no IP reached threshold
        assert_eq!(server.local_node.ip, original_ip);
        // Round still ends and votes are cleared
        assert!(server.ip_votes.is_empty());
        assert!(server.first_ip_vote_round_completed);
    }

    #[tokio::test]
    async fn test_ip_vote_cleanup() {
        let local_node = Node::from_enode_url(
            "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
        ).expect("Bad enode url");
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();

        let mut server = DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket: Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
            peer_table: PeerTable::spawn(
                10,
                Store::new("", EngineType::InMemory).expect("Failed to create store"),
            ),
            initial_lookup_interval: 1000.0,
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        };

        let ip: IpAddr = "203.0.113.50".parse().unwrap();
        let voter1 = H256::from_low_u64_be(1);

        // Manually insert a vote and set period start
        let mut voters = FxHashSet::default();
        voters.insert(voter1);
        server.ip_votes.insert(ip, voters);
        server.ip_vote_period_start = Some(Instant::now());
        assert_eq!(server.ip_votes.len(), 1);

        // Cleanup should retain votes (round hasn't timed out yet)
        server.cleanup_stale_entries();
        assert_eq!(server.ip_votes.len(), 1);

        // Cleanup didn't finalize because the 5-minute window hasn't elapsed
        assert!(!server.first_ip_vote_round_completed);
    }

    #[tokio::test]
    async fn test_ip_voting_ignores_private_ips() {
        let local_node = Node::from_enode_url(
            "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
        ).expect("Bad enode url");
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();

        let mut server = DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket: Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
            peer_table: PeerTable::spawn(
                10,
                Store::new("", EngineType::InMemory).expect("Failed to create store"),
            ),
            initial_lookup_interval: 1000.0,
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        };

        let voter1 = H256::from_low_u64_be(1);
        let voter2 = H256::from_low_u64_be(2);
        let voter3 = H256::from_low_u64_be(3);

        // Private IPs should be ignored
        let private_ip: IpAddr = "192.168.1.100".parse().unwrap();
        server.record_ip_vote(private_ip, voter1);
        server.record_ip_vote(private_ip, voter2);
        server.record_ip_vote(private_ip, voter3);
        assert!(server.ip_votes.is_empty());

        // Loopback should be ignored
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        server.record_ip_vote(loopback, voter1);
        assert!(server.ip_votes.is_empty());

        // Link-local should be ignored
        let link_local: IpAddr = "169.254.1.1".parse().unwrap();
        server.record_ip_vote(link_local, voter1);
        assert!(server.ip_votes.is_empty());

        // IPv6 loopback should be ignored
        let ipv6_loopback: IpAddr = "::1".parse().unwrap();
        server.record_ip_vote(ipv6_loopback, voter1);
        assert!(server.ip_votes.is_empty());

        // IPv6 link-local (fe80::/10) should be ignored
        let ipv6_link_local: IpAddr = "fe80::1".parse().unwrap();
        server.record_ip_vote(ipv6_link_local, voter1);
        assert!(server.ip_votes.is_empty());

        // IPv6 unique local (fc00::/7) should be ignored
        let ipv6_unique_local: IpAddr = "fd12::1".parse().unwrap();
        server.record_ip_vote(ipv6_unique_local, voter1);
        assert!(server.ip_votes.is_empty());

        // Public IP should be recorded
        let public_ip: IpAddr = "203.0.113.50".parse().unwrap();
        server.record_ip_vote(public_ip, voter1);
        assert_eq!(server.ip_votes.get(&public_ip).map(|v| v.len()), Some(1));
    }
}
