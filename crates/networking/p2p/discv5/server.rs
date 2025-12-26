use crate::{
    discv5::{
        codec::Discv5Codec,
        messages::{
            DecodedPacket, FindNodeMessage, Handshake, Message, NodesMessage, Ordinary, Packet,
            PacketCodecError, PacketHeader, PingMessage, PongMessage, WhoAreYou,
        },
        session::{Session, build_challenge_data, create_id_signature, derive_session_keys},
    },
    metrics::METRICS,
    peer_table::{Contact, OutMessage as PeerTableOutMessage, PeerTable, PeerTableError},
    rlpx::utils::{compress_pubkey, ecdh_xchng},
    types::{Endpoint, Node, NodeRecord},
    utils::{get_msg_expiration_from_seconds, public_key_from_signing_key},
};
use bytes::{BufMut, BytesMut};
use ethrex_common::{H256, H512, types::ForkId};
use ethrex_storage::{Store, error::StoreError};
use futures::{
    SinkExt as _, Stream, StreamExt,
    stream::{SplitSink, SplitStream},
};
use indexmap::IndexMap;
use rand::{Rng, RngCore, rngs::OsRng, thread_rng};
use secp256k1::{PublicKey, SecretKey, ecdsa::Signature};
use spawned_concurrency::{
    messages::Unused,
    tasks::{
        CastResponse, GenServer, GenServerHandle, InitResult::Success, send_after, send_interval,
        send_message_on, spawn_listener,
    },
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;
use tokio_util::udp::UdpFramed;
use tracing::{debug, error, info, trace};

pub(crate) const MAX_NODES_IN_NEIGHBORS_PACKET: usize = 16;
const EXPIRATION_SECONDS: u64 = 20;
/// Interval between revalidation checks.
const REVALIDATION_CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60); // 12 hours,
/// Interval between revalidations.
const REVALIDATION_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60); // 12 hours,
/// The initial interval between peer lookups, until the number of peers reaches
/// [target_peers](DiscoverySideCarState::target_peers), or the number of
/// contacts reaches [target_contacts](DiscoverySideCarState::target_contacts).
pub const INITIAL_LOOKUP_INTERVAL_MS: f64 = 100.0; // 10 per second
pub const LOOKUP_INTERVAL_MS: f64 = 600.0; // 100 per minute
const CHANGE_FIND_NODE_MESSAGE_INTERVAL: Duration = Duration::from_secs(5);
const PRUNE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryServerError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Failed to decode packet")]
    InvalidPacket(#[from] PacketCodecError),
    #[error("Failed to send message")]
    MessageSendFailure(PacketCodecError),
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
    store: Store,
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

        let mut discovery_server = Self {
            local_node: local_node.clone(),
            local_node_record,
            signer,
            udp_socket: Arc::new(udp_socket),
            store: storage.clone(),
            peer_table: peer_table.clone(),
            initial_lookup_interval,
            counter: 0,
            messages_by_nonce: Default::default(),
        };

        info!(count = bootnodes.len(), "Adding bootnodes");

        for bootnode in &bootnodes {
            discovery_server.send_ping(bootnode).await?;
        }
        peer_table
            .new_contacts(bootnodes, local_node.node_id())
            .await?;

        discovery_server.start();
        Ok(())
    }

    async fn handle_packet(
        &mut self,
        Discv5Message { from, packet }: Discv5Message,
    ) -> Result<(), DiscoveryServerError> {
        trace!(?packet, address= ?from, "Discv5 packet received");
        // TODO retrieve session info
        match packet.header.flag {
            0x00 => {
                tracing::info!("NonWhoAreYou!");
                Ok(())
            }
            0x01 => self.handle_who_are_you(packet).await,
            0x02 => {
                tracing::info!("NonWhoAreYou!");
                Ok(())
            }
            _ => Err(PacketCodecError::MalformedData)?,
        }
    }

    async fn handle_who_are_you(&mut self, packet: Packet) -> Result<(), DiscoveryServerError> {
        let whoareyou: WhoAreYou = WhoAreYou::decode(&packet)?;
        let nonce = packet.header.nonce;
        tracing::info!(nonce=?nonce, id_nonce=?whoareyou.id_nonce, enr_seq=?whoareyou.enr_seq,  "WhoAreYou packet received");
        let Some((node, message, _)) = self.messages_by_nonce.swap_remove(&nonce) else {
            tracing::trace!("Received unexpected WhoAreYou packet. Ignoring it");
            return Ok(());
        };

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
            return Err(DiscoveryServerError::CryptographyError(format!(
                "Invalid public key"
            )));
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
        self.send_handshake(&message, signature, &ephemeral_pubkey, &node)
            .await?;

        Ok(())
    }

    async fn revalidate(&mut self) -> Result<(), DiscoveryServerError> {
        for contact in self
            .peer_table
            .get_contacts_to_revalidate(REVALIDATION_INTERVAL)
            .await?
        {
            self.send_ping(&contact.node).await?;
        }
        Ok(())
    }

    async fn lookup(&mut self) -> Result<(), DiscoveryServerError> {
        if let Some(contact) = self.peer_table.get_contact_for_lookup().await? {
            if let Err(e) = self
                .send_ordinary(&Message::FindNode(rand::random()), &contact.node)
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

    async fn send_find_node(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn send_ping(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn send_pong(&self, ping_hash: H256, node: &Node) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn send_nodes(
        &self,
        neighbors: Vec<Node>,
        node: &Node,
    ) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn handle_ping(
        &mut self,
        ping_message: PingMessage,
        hash: H256,
        sender_public_key: H512,
        node: Node,
    ) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn handle_pong(
        &mut self,
        message: PongMessage,
        node_id: H256,
    ) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn handle_find_node(
        &mut self,
        sender_public_key: H512,
        target: H512,
        from: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn handle_nodes(
        &mut self,
        nodes_message: NodesMessage,
    ) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    /// Validates the fork id of the given ENR is valid, saving it to the peer_table.
    async fn validate_enr_fork_id(
        &mut self,
        node_id: H256,
        sender_public_key: H512,
        node_record: NodeRecord,
    ) -> Result<(), DiscoveryServerError> {
        let pairs = node_record.decode_pairs();

        let Some(remote_fork_id) = pairs.eth else {
            self.peer_table
                .set_is_fork_id_valid(&node_id, false)
                .await?;
            debug!(received = "ENRResponse", from = %format!("{sender_public_key:#x}"), "missing fork id in ENR response, skipping");
            return Ok(());
        };

        let chain_config = self.store.get_chain_config();
        let genesis_header = self
            .store
            .get_block_header(0)?
            .ok_or(DiscoveryServerError::InvalidContact)?;
        let latest_block_number = self.store.get_latest_block_number().await?;
        let latest_block_header = self
            .store
            .get_block_header(latest_block_number)?
            .ok_or(DiscoveryServerError::InvalidContact)?;

        let local_fork_id = ForkId::new(
            chain_config,
            genesis_header.clone(),
            latest_block_header.timestamp,
            latest_block_number,
        );

        if !local_fork_id.is_valid(
            remote_fork_id.clone(),
            latest_block_number,
            latest_block_header.timestamp,
            chain_config,
            genesis_header,
        ) {
            self.peer_table
                .set_is_fork_id_valid(&node_id, false)
                .await?;
            debug!(received = "ENRResponse", from = %format!("{sender_public_key:#x}"), local_fork_id=%local_fork_id, remote_fork_id=%remote_fork_id, "fork id mismatch in ENR response, skipping");
            return Ok(());
        }

        debug!(received = "ENRResponse", from = %format!("{sender_public_key:#x}"), local_fork_id=%local_fork_id, remote_fork_id=%remote_fork_id, "valid fork id in ENR found");
        self.peer_table.set_is_fork_id_valid(&node_id, true).await?;

        Ok(())
    }

    async fn validate_contact(
        &mut self,
        sender_public_key: H512,
        node_id: H256,
        from: SocketAddr,
        message_type: &str,
    ) -> Result<Contact, DiscoveryServerError> {
        match self
            .peer_table
            .validate_contact(&node_id, from.ip())
            .await?
        {
            PeerTableOutMessage::UnknownContact => {
                debug!(received = message_type, to = %format!("{sender_public_key:#x}"), "Unknown contact, skipping");
                Err(DiscoveryServerError::InvalidContact)
            }
            PeerTableOutMessage::InvalidContact => {
                debug!(received = message_type, to = %format!("{sender_public_key:#x}"), "Contact not validated, skipping");
                Err(DiscoveryServerError::InvalidContact)
            }
            // Check that the IP address from which we receive the request matches the one we have stored to prevent amplification attacks
            // This prevents an attack vector where the discovery protocol could be used to amplify traffic in a DDOS attack.
            // A malicious actor would send a findnode request with the IP address and UDP port of the target as the source address.
            // The recipient of the findnode packet would then send a neighbors packet (which is a much bigger packet than findnode) to the victim.
            PeerTableOutMessage::IpMismatch => {
                debug!(received = message_type, to = %format!("{sender_public_key:#x}"), "IP address mismatch, skipping");
                Err(DiscoveryServerError::InvalidContact)
            }
            PeerTableOutMessage::Contact(contact) => Ok(*contact),
            _ => unreachable!(),
        }
    }

    async fn validate_enr_response(
        &mut self,
        sender_public_key: H512,
        node_id: H256,
        from: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let contact = self
            .validate_contact(sender_public_key, node_id, from, "ENRResponse")
            .await?;
        if !contact.has_pending_enr_request() {
            debug!(received = "ENRResponse", from = %format!("{sender_public_key:#x}"), "unsolicited message received, skipping");
            return Err(DiscoveryServerError::InvalidContact);
        }
        Ok(())
    }

    async fn send_ordinary(
        &mut self,
        message: &Message,
        node: &Node,
    ) -> Result<(), DiscoveryServerError> {
        let packet = DecodedPacket::Ordinary(Ordinary {
            src_id: self.local_node.node_id(),
            message: message.clone(),
        });
        let addr = node.udp_addr();
        let mut buf = BytesMut::new();
        let encrypt_key = self
            .peer_table
            .get_session_info(node.node_id())
            .await?
            .map_or([0; 16], |s| s.outbound_key);
        let nonce = self.encode_packet(&mut buf, packet, &node.node_id(), &encrypt_key)?;
        let _ = self.udp_socket.send_to(&buf, addr).await.inspect_err(
            |e| error!(sending = ?message, addr = ?addr, err=?e, "Error sending message"),
        )?;
        trace!(msg = %message, node = %node.public_key, address= %addr, nonce=?nonce, "Discv5 ordinary message sent");
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
    ) -> Result<(), DiscoveryServerError> {
        let handshake = Handshake {
            src_id: self.local_node.node_id(),
            id_signature: signature.serialize_compact().to_vec(),
            eph_pubkey: eph_pubkey.to_vec(),
            record: Some(self.local_node_record.clone()),
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

        let (static_header, authdata, encrypted_message) =
            handshake.encode(&nonce, &masking_iv.to_be_bytes(), &encrypt_key)?;

        let header = PacketHeader {
            static_header: static_header.try_into().unwrap(),
            flag: 0x02,
            nonce,
            authdata,
            header_end_offset: 23,
        };

        let packet = Packet {
            masking_iv: masking_iv.to_be_bytes(),
            header,
            encrypted_message,
        };

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

    fn encode_packet(
        &mut self,
        buf: &mut dyn BufMut,
        packet: DecodedPacket,
        dest_id: &H256,
        encrypt_key: &[u8],
    ) -> Result<[u8; 12], PacketCodecError> {
        let mut rng = OsRng;
        let masking_iv: u128 = rng.r#gen();
        let nonce = self.next_nonce(&mut rng);
        packet.encode(buf, masking_iv, &nonce, dest_id, encrypt_key)?;
        Ok(nonce)
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
                    .inspect_err(|e| error!(err=?e, "Error Handling Discovery message"));
            }
            Self::CastMsg::Revalidate => {
                trace!(received = "Revalidate");
                let _ = self
                    .revalidate()
                    .await
                    .inspect_err(|e| error!(err=?e, "Error revalidating discovered peers"));
            }
            Self::CastMsg::Lookup => {
                trace!(received = "Lookup");
                let _ = self
                    .lookup()
                    .await
                    .inspect_err(|e| error!(err=?e, "Error performing Discovery lookup"));

                let interval = self.get_lookup_interval().await;
                send_after(interval, handle.clone(), Self::CastMsg::Lookup);
            }
            Self::CastMsg::Prune => {
                trace!(received = "Prune");
                let _ = self
                    .prune()
                    .await
                    .inspect_err(|e| error!(err=?e, "Error Pruning peer table"));
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
    Duration::from_secs(5)
    // // Smooth progression curve
    // // See https://easings.net/#easeInOutCubic
    // let ease_in_out_cubic = if progress < 0.5 {
    //     4.0 * progress.powf(3.0)
    // } else {
    //     1.0 - ((-2.0 * progress + 2.0).powf(3.0)) / 2.0
    // };
    // Duration::from_micros(
    //     // Use `progress` here instead of `ease_in_out_cubic` for a linear function.
    //     (1000f64 * (ease_in_out_cubic * (upper_limit - lower_limit) + lower_limit)).round() as u64,
    // )
}

#[cfg(test)]
mod tests {
    // use rand::{SeedableRng, rngs::StdRng};

    // use crate::discv5::server::DiscoveryServer;

    // #[test]
    // fn test_next_nonce_counter() {
    //     let mut rng = StdRng::seed_from_u64(7);
    //     let server = DiscoveryServer {
    //         local_node: Default::default(),
    //         local_node_record: todo!(),
    //         signer: todo!(),
    //         udp_socket: todo!(),
    //         store: todo!(),
    //         peer_table: todo!(),
    //         initial_lookup_interval: todo!(),
    //         counter: 0,
    //     };

    //     let n1 = server.next_nonce(&mut rng);
    //     let n2 = server.next_nonce(&mut rng);

    //     assert_eq!(&n1[..4], &[0, 0, 0, 0]);
    //     assert_eq!(&n2[..4], &[0, 0, 0, 1]);
    //     assert_ne!(&n1[4..], &n2[4..]);
    // }
}
