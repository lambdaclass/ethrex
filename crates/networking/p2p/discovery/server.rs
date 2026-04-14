use crate::{
    discv4::{
        messages::Packet as Discv4Packet,
        server::{Discv4Message, Discv4State},
    },
    discv5::{
        messages::{Packet as Discv5Packet, PacketCodecError},
        server::{Discv5Message, Discv5State, update_local_ip},
    },
    peer_table::{DiscoveryProtocol, PeerTable, PeerTableServerProtocol as _},
    types::{INITIAL_ENR_SEQ, Node, NodeRecord},
};
use bytes::BytesMut;
use ethrex_common::utils::keccak;
use ethrex_storage::Store;
use futures::StreamExt;
use secp256k1::SecretKey;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{
        Actor, ActorStart as _, Context, Handler, send_after, send_interval, send_message_on,
        spawn_listener,
    },
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use thiserror::Error;
use tokio::net::UdpSocket;
use tokio_util::udp::UdpFramed;
use tracing::{debug, error, info, trace};

use super::{DiscoveryConfig, codec::DiscriminatingCodec, lookup_interval_function};

/// Minimum packet size for a valid discv4 packet.
/// hash (32) + signature (65) + type (1) = 98 bytes
const DISCV4_MIN_PACKET_SIZE: usize = 98;

// Shared constants
const REVALIDATION_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const PRUNE_INTERVAL: Duration = Duration::from_secs(5);
const CHANGE_FIND_NODE_MESSAGE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum DiscoveryServerError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Failed to decode discv4 packet")]
    Discv4Decode(#[from] crate::discv4::messages::PacketDecodeErr),
    #[error("Failed to decode discv5 packet")]
    Discv5Decode(#[from] crate::discv5::messages::PacketCodecError),
    #[error("Only partial message was sent")]
    PartialMessageSent,
    #[error("Unknown or invalid contact")]
    InvalidContact,
    #[error(transparent)]
    PeerTable(#[from] ActorError),
    #[error(transparent)]
    Store(#[from] ethrex_storage::error::StoreError),
    #[error("Internal error {0}")]
    InternalError(String),
    #[error("Cryptography Error {0}")]
    CryptographyError(String),
    #[error(transparent)]
    RlpDecode(#[from] ethrex_rlp::error::RLPDecodeError),
}

#[protocol]
pub trait DiscoveryServerProtocol: Send + Sync {
    fn raw_packet(&self, data: BytesMut, from: SocketAddr) -> Result<(), ActorError>;
    fn revalidate_v4(&self) -> Result<(), ActorError>;
    fn revalidate_v5(&self) -> Result<(), ActorError>;
    fn lookup_v4(&self) -> Result<(), ActorError>;
    fn lookup_v5(&self) -> Result<(), ActorError>;
    fn enr_lookup(&self) -> Result<(), ActorError>;
    fn change_find_node_message(&self) -> Result<(), ActorError>;
    fn prune(&self) -> Result<(), ActorError>;
    fn shutdown(&self) -> Result<(), ActorError>;
}

pub struct DiscoveryServer {
    pub local_node: Node,
    pub local_node_record: NodeRecord,
    pub(crate) signer: SecretKey,
    pub(crate) udp_socket: Arc<UdpSocket>,
    pub(crate) store: Store,
    pub peer_table: PeerTable,
    pub(crate) config: DiscoveryConfig,
    pub discv4: Option<Discv4State>,
    pub discv5: Option<Discv5State>,
}

impl std::fmt::Debug for DiscoveryServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveryServer")
            .field("local_node", &self.local_node)
            .field("discv4_enabled", &self.discv4.is_some())
            .field("discv5_enabled", &self.discv5.is_some())
            .finish()
    }
}

#[actor(protocol = DiscoveryServerProtocol)]
impl DiscoveryServer {
    pub async fn spawn(
        storage: Store,
        local_node: Node,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        peer_table: PeerTable,
        bootnodes: Vec<Node>,
        config: DiscoveryConfig,
    ) -> Result<(), DiscoveryServerError> {
        info!("Starting unified discovery server");

        let mut local_node_record = NodeRecord::from_node(&local_node, INITIAL_ENR_SEQ, &signer)
            .expect("Failed to create local node record");
        if let Ok(fork_id) = storage.get_fork_id().await {
            local_node_record
                .set_fork_id(fork_id, &signer)
                .expect("Failed to set fork_id on local node record");
        }

        let discv4 = if config.discv4_enabled {
            info!(
                protocol = "discv4",
                count = bootnodes.len(),
                "Adding bootnodes"
            );
            peer_table.new_contacts(
                bootnodes.clone(),
                local_node.node_id(),
                DiscoveryProtocol::Discv4,
            )?;
            Some(Discv4State::new(&signer))
        } else {
            None
        };

        let discv5 = if config.discv5_enabled {
            info!(
                protocol = "discv5",
                count = bootnodes.len(),
                "Adding bootnodes"
            );
            peer_table.new_contacts(
                bootnodes.clone(),
                local_node.node_id(),
                DiscoveryProtocol::Discv5,
            )?;
            Some(Discv5State::default())
        } else {
            None
        };

        let mut server = Self {
            local_node: local_node.clone(),
            local_node_record,
            signer,
            udp_socket: udp_socket.clone(),
            store: storage,
            peer_table: peer_table.clone(),
            config,
            discv4,
            discv5,
        };

        // Ping discv4 bootnodes
        if server.discv4.is_some() {
            for bootnode in &bootnodes {
                server.discv4_send_ping(bootnode).await?;
            }
        }

        server.start();

        Ok(())
    }

    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        let local_addr = self.udp_socket.local_addr();
        info!(
            local_addr=?local_addr,
            discv4_enabled=self.config.discv4_enabled,
            discv5_enabled=self.config.discv5_enabled,
            "Discovery server started, listening for UDP packets"
        );

        // Set up UDP listener
        let stream = UdpFramed::new(self.udp_socket.clone(), DiscriminatingCodec::new());
        spawn_listener(
            ctx.clone(),
            stream.filter_map(|result| async move {
                match result {
                    Ok((data, from)) => Some(discovery_server_protocol::RawPacket { data, from }),
                    Err(e) => {
                        debug!(error=?e, "Error receiving packet in discovery server");
                        None
                    }
                }
            }),
        );

        // Discv4 timers
        if self.discv4.is_some() {
            send_interval(
                REVALIDATION_CHECK_INTERVAL,
                ctx.clone(),
                discovery_server_protocol::RevalidateV4,
            );
            send_interval(
                CHANGE_FIND_NODE_MESSAGE_INTERVAL,
                ctx.clone(),
                discovery_server_protocol::ChangeFindNodeMessage,
            );
            let _ = ctx.send(discovery_server_protocol::LookupV4);
            let _ = ctx.send(discovery_server_protocol::EnrLookup);
        }

        // Discv5 timers
        if self.discv5.is_some() {
            send_interval(
                REVALIDATION_CHECK_INTERVAL,
                ctx.clone(),
                discovery_server_protocol::RevalidateV5,
            );
            let _ = ctx.send(discovery_server_protocol::LookupV5);
        }

        // Shared prune timer
        send_interval(
            PRUNE_INTERVAL,
            ctx.clone(),
            discovery_server_protocol::Prune,
        );

        // Shutdown handler
        send_message_on(
            ctx.clone(),
            tokio::signal::ctrl_c(),
            discovery_server_protocol::Shutdown,
        );
    }

    #[send_handler]
    async fn handle_raw_packet(
        &mut self,
        msg: discovery_server_protocol::RawPacket,
        _ctx: &Context<Self>,
    ) {
        self.route_packet(&msg.data, msg.from).await;
    }

    #[send_handler]
    async fn handle_revalidate_v4(
        &mut self,
        _msg: discovery_server_protocol::RevalidateV4,
        _ctx: &Context<Self>,
    ) {
        trace!(protocol = "discv4", received = "Revalidate");
        let _ = self.discv4_revalidate().await.inspect_err(
            |e| error!(protocol = "discv4", err=?e, "Error revalidating discovered peers"),
        );
    }

    #[send_handler]
    async fn handle_revalidate_v5(
        &mut self,
        _msg: discovery_server_protocol::RevalidateV5,
        _ctx: &Context<Self>,
    ) {
        trace!(protocol = "discv5", received = "Revalidate");
        let _ = self.discv5_revalidate().await.inspect_err(
            |e| error!(protocol = "discv5", err=?e, "Error revalidating discovered peers"),
        );
    }

    #[send_handler]
    async fn handle_lookup_v4(
        &mut self,
        _msg: discovery_server_protocol::LookupV4,
        ctx: &Context<Self>,
    ) {
        trace!(protocol = "discv4", received = "Lookup");
        let _ = self.discv4_lookup().await.inspect_err(
            |e| error!(protocol = "discv4", err=?e, "Error performing Discovery lookup"),
        );
        let interval = self.get_lookup_interval().await;
        send_after(interval, ctx.clone(), discovery_server_protocol::LookupV4);
    }

    #[send_handler]
    async fn handle_lookup_v5(
        &mut self,
        _msg: discovery_server_protocol::LookupV5,
        ctx: &Context<Self>,
    ) {
        trace!(protocol = "discv5", received = "Lookup");
        let _ = self.discv5_lookup().await.inspect_err(
            |e| error!(protocol = "discv5", err=?e, "Error performing Discovery lookup"),
        );
        let interval = self.get_lookup_interval().await;
        send_after(interval, ctx.clone(), discovery_server_protocol::LookupV5);
    }

    #[send_handler]
    async fn handle_enr_lookup(
        &mut self,
        _msg: discovery_server_protocol::EnrLookup,
        ctx: &Context<Self>,
    ) {
        trace!(protocol = "discv4", received = "EnrLookup");
        let _ = self.discv4_enr_lookup().await.inspect_err(
            |e| error!(protocol = "discv4", err=?e, "Error performing Discovery lookup"),
        );
        let interval = self.get_lookup_interval().await;
        send_after(interval, ctx.clone(), discovery_server_protocol::EnrLookup);
    }

    #[send_handler]
    async fn handle_change_find_node_message(
        &mut self,
        _msg: discovery_server_protocol::ChangeFindNodeMessage,
        _ctx: &Context<Self>,
    ) {
        if let Some(discv4) = &mut self.discv4 {
            discv4.find_node_message = Discv4State::random_message(&self.signer);
        }
    }

    #[send_handler]
    async fn handle_prune(&mut self, _msg: discovery_server_protocol::Prune, _ctx: &Context<Self>) {
        trace!(received = "Prune");
        let _ = self
            .prune()
            .await
            .inspect_err(|e| error!(err=?e, "Error Pruning peer table"));
    }

    #[send_handler]
    async fn handle_shutdown(
        &mut self,
        _msg: discovery_server_protocol::Shutdown,
        ctx: &Context<Self>,
    ) {
        ctx.stop();
    }

    // --- Shared logic ---

    async fn route_packet(&mut self, data: &[u8], from: SocketAddr) {
        if is_discv4_packet(data) {
            self.route_to_discv4(data, from).await;
        } else {
            self.route_to_discv5(data, from).await;
        }
    }

    async fn route_to_discv4(&mut self, data: &[u8], from: SocketAddr) {
        if self.discv4.is_none() {
            return;
        }
        match Discv4Packet::decode(data) {
            Ok(packet) => {
                let msg = Discv4Message::from(packet, from);
                let _ = self.discv4_process_message(msg).await.inspect_err(
                    |e| error!(protocol = "discv4", err=?e, "Error handling discovery message"),
                );
            }
            Err(e) => {
                debug!(error=?e, "Failed to decode discv4 packet");
            }
        }
    }

    async fn route_to_discv5(&mut self, data: &[u8], from: SocketAddr) {
        if self.discv5.is_none() {
            return;
        }
        match Discv5Packet::decode(&self.local_node.node_id(), data) {
            Ok(packet) => {
                let msg = Discv5Message::from(packet, from);
                let _ = self.discv5_handle_packet(msg).await.inspect_err(
                    |e| trace!(protocol = "discv5", err=?e, "Error handling discovery message"),
                );
            }
            Err(
                PacketCodecError::InvalidProtocol(_)
                | PacketCodecError::InvalidHeader
                | PacketCodecError::InvalidSize
                | PacketCodecError::CipherError(_),
            ) => {
                trace!(from=?from, "Dropping unrecognized UDP packet");
            }
            Err(e) => {
                debug!(error=?e, "Failed to decode discv5 packet");
            }
        }
    }

    async fn prune(&mut self) -> Result<(), DiscoveryServerError> {
        self.peer_table.prune_table()?;
        if let Some(discv4) = &mut self.discv4 {
            let expiration = Duration::from_secs(crate::discv4::server::EXPIRATION_SECONDS);
            discv4
                .pending_find_node
                .retain(|_, sent_at| sent_at.elapsed() < expiration);
        }
        let winning_ip = self
            .discv5
            .as_mut()
            .and_then(|discv5| discv5.cleanup_stale_entries());
        if let Some(winning_ip) = winning_ip
            && winning_ip != self.local_node.ip
        {
            info!(
                protocol = "discv5",
                old_ip = %self.local_node.ip,
                new_ip = %winning_ip,
                "External IP detected via PONG voting, updating local ENR"
            );
            update_local_ip(
                &mut self.local_node,
                &mut self.local_node_record,
                &self.signer,
                winning_ip,
            );
        }
        Ok(())
    }

    pub(crate) async fn get_lookup_interval(&self) -> Duration {
        let peer_completion = self
            .peer_table
            .target_peers_completion()
            .await
            .unwrap_or_default();
        lookup_interval_function(
            peer_completion,
            self.config.initial_lookup_interval,
            super::LOOKUP_INTERVAL_MS,
        )
    }
}

/// Check if a packet is a discv4 packet by verifying the hash.
pub fn is_discv4_packet(data: &[u8]) -> bool {
    if data.len() < DISCV4_MIN_PACKET_SIZE {
        return false;
    }
    let packet_hash = &data[0..32];
    let computed_hash = keccak(&data[32..]);
    packet_hash == computed_hash.as_bytes()
}

#[cfg(any(test, feature = "test-utils"))]
impl DiscoveryServer {
    /// Builds a DiscoveryServer suitable for unit tests of discv5 handlers.
    /// Only discv5 state is initialized; discv4 is disabled.
    /// Uses an in-memory store and a dummy initial lookup interval.
    pub fn new_for_discv5_test(
        local_node: Node,
        local_node_record: NodeRecord,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        peer_table: PeerTable,
    ) -> Self {
        Self {
            local_node,
            local_node_record,
            signer,
            udp_socket,
            store: Store::new("", ethrex_storage::EngineType::InMemory)
                .expect("Failed to create store"),
            peer_table,
            config: DiscoveryConfig {
                discv4_enabled: false,
                discv5_enabled: true,
                initial_lookup_interval: 1000.0,
            },
            discv4: None,
            discv5: Some(Discv5State::default()),
        }
    }
}
