use crate::{
    discovery::ip_predictor::IpPredictor,
    discv4::{
        messages::Packet as Discv4Packet,
        server::{Discv4Message, Discv4State},
    },
    discv5::{
        messages::{Packet as Discv5Packet, PacketCodecError},
        server::{Discv5Message, Discv5State, update_local_ip},
    },
    peer_table::{DiscoveryProtocol, PeerTable, PeerTableServerProtocol as _},
    types::{Node, NodeRecord, SharedLocalNode},
};
use bytes::BytesMut;
use ethrex_common::types::ForkId;
use ethrex_common::utils::keccak;
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
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio_util::udp::UdpFramed;
use tracing::{debug, error, info, trace, warn};

use super::{
    DiscoveryConfig, codec::DiscriminatingCodec, lookup_interval_function, next_lookup_interval,
};

/// Minimum packet size for a valid discv4 packet.
/// hash (32) + signature (65) + type (1) = 98 bytes
const DISCV4_MIN_PACKET_SIZE: usize = 98;

// Shared constants
const REVALIDATION_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const PRUNE_INTERVAL: Duration = Duration::from_secs(5);

/// Lookup interval bounds for iterative lookups. Each iterative lookup
/// generates ~16 FindNode messages (vs 1 in the old approach), so we use
/// longer intervals to produce similar per-second traffic.
const ITERATIVE_LOOKUP_INITIAL_MS: f64 = 500.0; // 6 FindNode/sec at startup (alpha=3 × 2 ticks/sec)
const ITERATIVE_LOOKUP_INTERVAL_MS: f64 = 10_000.0; // ~6 lookups/min at steady-state

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
    fn prune(&self) -> Result<(), ActorError>;
    fn shutdown(&self) -> Result<(), ActorError>;
}

pub struct DiscoveryServer {
    pub local_node: Node,
    pub local_node_record: NodeRecord,
    pub(crate) signer: SecretKey,
    pub(crate) udp_socket: Arc<UdpSocket>,
    pub peer_table: PeerTable,
    pub(crate) config: DiscoveryConfig,
    pub discv4: Option<Discv4State>,
    pub discv5: Option<Discv5State>,
    /// Shared IP predictor fed by both discv4 and discv5 PONGs.
    pub ip_predictor: IpPredictor,
    /// When true the user supplied `--nat extip:<addr>` and we must not override it.
    pub(crate) ip_override_locked: bool,
    /// Live-updated local node identity shared with the RPC layer.
    pub shared_local_node: SharedLocalNode,
    /// Current EIP-2124 fork id, published by the network layer. `None` inside the
    /// channel means the chain could not supply one yet.
    ///
    /// Discovery does not read this from storage: the network layer decides what this
    /// node announces about itself, and discovery only republishes. Without a refresh
    /// the `eth` ENR entry goes stale the moment the chain crosses a fork, and geth's
    /// dial-candidate filter (`NewNodeFilter`) drops us outright.
    pub(crate) fork_id: watch::Receiver<Option<ForkId>>,
}

impl std::fmt::Debug for DiscoveryServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveryServer")
            .field("local_node", &self.local_node)
            .field("discv4_enabled", &self.discv4.is_some())
            .field("discv5_enabled", &self.discv5.is_some())
            .field("ip_predictor", &self.ip_predictor)
            .field("ip_override_locked", &self.ip_override_locked)
            .finish()
    }
}

#[actor(protocol = DiscoveryServerProtocol)]
impl DiscoveryServer {
    /// Starts the discovery actor, advertising `local_node_record` as this
    /// node's ENR.
    #[expect(
        clippy::too_many_arguments,
        reason = "caller-supplied values; bundling them would only hide the call site"
    )]
    pub async fn spawn(
        local_node: Node,
        local_node_record: NodeRecord,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        peer_table: PeerTable,
        bootnodes: Vec<Node>,
        config: DiscoveryConfig,
        shared_local_node: SharedLocalNode,
        fork_id: watch::Receiver<Option<ForkId>>,
    ) -> Result<(), DiscoveryServerError> {
        debug!("Starting discovery server");

        let bootnodes: Vec<Node> = bootnodes
            .into_iter()
            .filter(|node| {
                let allowed = config.netrestrict.allows(node.ip);
                if !allowed {
                    warn!(
                        node = %node,
                        netrestrict = %config.netrestrict,
                        "Dropping bootnode outside --p2p.netrestrict"
                    );
                }
                allowed
            })
            .collect();

        let discv4 = if config.discv4_enabled {
            info!(
                protocol = "discv4",
                count = bootnodes.len(),
                "Adding bootnodes"
            );
            peer_table.new_contacts(bootnodes.clone(), DiscoveryProtocol::Discv4)?;
            Some(Discv4State::default())
        } else {
            None
        };

        let discv5 = if config.discv5_enabled {
            info!(
                protocol = "discv5",
                count = bootnodes.len(),
                "Adding bootnodes"
            );
            peer_table.new_contacts(bootnodes.clone(), DiscoveryProtocol::Discv5)?;
            Some(Discv5State::default())
        } else {
            None
        };

        // With both protocols off the bootnodes are the whole network this node
        // will ever know. Hand them to the dialer anyway, so disabling discovery
        // doubles as a static-peers mode instead of a node with no peers at all.
        // The protocol tag only steers revalidation, which is not running.
        if discv4.is_none() && discv5.is_none() && !bootnodes.is_empty() {
            info!(
                count = bootnodes.len(),
                "Discovery disabled, using bootnodes as static peers"
            );
            peer_table.new_contacts(bootnodes.clone(), DiscoveryProtocol::Discv4)?;
        }

        let ip_override_locked = config.nat_extip_set;
        let mut server = Self {
            local_node: local_node.clone(),
            local_node_record,
            signer,
            udp_socket: udp_socket.clone(),
            peer_table: peer_table.clone(),
            ip_predictor: IpPredictor::default(),
            ip_override_locked,
            config,
            discv4,
            discv5,
            shared_local_node,
            fork_id,
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
        let interval = self.get_lookup_interval(DiscoveryProtocol::Discv4).await;
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
        let interval = self.get_lookup_interval(DiscoveryProtocol::Discv5).await;
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
        let interval = self.get_lookup_interval(DiscoveryProtocol::Discv4).await;
        send_after(interval, ctx.clone(), discovery_server_protocol::EnrLookup);
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
        if !self.config.netrestrict.allows(from.ip()) {
            trace!(%from, "Dropping UDP packet from outside --p2p.netrestrict");
            return;
        }
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
        if let Some(discv5) = &mut self.discv5 {
            discv5.cleanup_stale_entries();
        }
        self.refresh_fork_id();
        if let Some(ip) = self.ip_predictor.check_timeout() {
            self.apply_predicted_ip(ip, "timeout");
        }
        Ok(())
    }

    /// `source` names the protocol/path that produced the winning vote
    /// ("discv4", "discv5", or "timeout"), purely for the convergence log line.
    pub fn apply_predicted_ip(&mut self, winning_ip: IpAddr, source: &str) {
        // `winning_ip` is already routability-filtered upstream: `record_ip_vote`
        // drops only unroutable addresses (loopback/link-local/unspecified) via
        // `is_unroutable_ip`. RFC1918 / IPv6 unique-local are intentionally kept
        // and may be advertised — on a flat private network (e.g. a kurtosis
        // enclave) the private IP is the address peers actually reach us at, and
        // a public winner still takes precedence when one reaches quorum (see
        // a49c779cc). Do not add an `is_private_ip` guard here.
        if self.ip_override_locked {
            return;
        }
        if winning_ip == self.local_node.ip {
            return;
        }
        if winning_ip.is_ipv4() != self.local_node.ip.is_ipv4() {
            warn!(
                predicted_ip = %winning_ip,
                local_ip = %self.local_node.ip,
                "Predicted external IP has different address family than local IP, ignoring"
            );
            return;
        }
        info!(
            source,
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
        // Propagate to the shared Arc so RPC and shutdown see the current identity.
        let mut guard = self
            .shared_local_node
            .write()
            .expect("shared_local_node poisoned");
        guard.node = self.local_node.clone();
        guard.record = self.local_node_record.clone();
    }

    /// How long until the next lookup tick for `protocol`.
    ///
    /// While a lookup is in flight the tick follows the completion-based pacing,
    /// so the lookup finishes promptly. Between lookups the saturation backoff
    /// applies as well: a network that keeps answering with nodes we already
    /// know is asked less and less often, down to the steady-state rate.
    pub(crate) async fn get_lookup_interval(&self, protocol: DiscoveryProtocol) -> Duration {
        let peer_completion = self
            .peer_table
            .target_peers_completion()
            .await
            .unwrap_or_default();
        let (lookup_in_flight, empty_lookups_in_a_row) = match protocol {
            DiscoveryProtocol::Discv4 => self
                .discv4
                .as_ref()
                .map(|s| (!s.active_lookups.is_empty(), s.empty_lookups_in_a_row)),
            DiscoveryProtocol::Discv5 => self
                .discv5
                .as_ref()
                .map(|s| (!s.active_lookups.is_empty(), s.empty_lookups_in_a_row)),
        }
        .unwrap_or((false, 0));
        if lookup_in_flight {
            lookup_interval_function(
                peer_completion,
                ITERATIVE_LOOKUP_INITIAL_MS,
                ITERATIVE_LOOKUP_INTERVAL_MS,
            )
        } else {
            next_lookup_interval(
                peer_completion,
                empty_lookups_in_a_row,
                ITERATIVE_LOOKUP_INITIAL_MS,
                ITERATIVE_LOOKUP_INTERVAL_MS,
            )
        }
    }
    /// Republish the ENR when the network layer reports a new fork id.
    ///
    /// `has_changed` is only true once per send, so a steady chain costs a flag check
    /// per tick. A closed channel means the network layer is gone, which is shutdown,
    /// not an error worth logging every tick. A `None` in the channel is the seed for
    /// a chain that could not supply a fork id at startup; there is nothing to
    /// republish until the publisher replaces it.
    fn refresh_fork_id(&mut self) {
        if !self.fork_id.has_changed().unwrap_or(false) {
            return;
        }
        let Some(fork_id) = self.fork_id.borrow_and_update().clone() else {
            return;
        };
        match self.local_node_record.set_fork_id(fork_id, &self.signer) {
            Ok(()) => {
                info!(
                    seq = self.local_node_record.seq,
                    "Chain crossed a fork; republished the local ENR with the new fork id"
                );
                // Mirror the re-signed record into the shared identity, as
                // apply_predicted_ip does: RPC serves the record from there, and
                // without this it would keep answering with the pre-fork ENR.
                let mut guard = self
                    .shared_local_node
                    .write()
                    .expect("shared_local_node poisoned");
                guard.record = self.local_node_record.clone();
            }
            // A record we cannot sign is worth surfacing: peers that require an `eth`
            // entry keep skipping us until this succeeds.
            Err(e) => warn!(error = ?e, "Could not set the new fork id on the local ENR"),
        }
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
    /// Uses a dummy initial lookup interval.
    pub fn new_for_discv5_test(
        local_node: Node,
        local_node_record: NodeRecord,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        peer_table: PeerTable,
    ) -> Self {
        use crate::types::LocalNode;
        use std::sync::{Arc, RwLock};
        let shared_local_node = Arc::new(RwLock::new(LocalNode {
            node: local_node.clone(),
            record: local_node_record.clone(),
        }));
        Self {
            local_node,
            local_node_record,
            signer,
            udp_socket,
            peer_table,
            config: DiscoveryConfig {
                discv4_enabled: false,
                discv5_enabled: true,
                nat_extip_set: false,
                netrestrict: crate::netrestrict::NetRestrict::default(),
            },
            discv4: None,
            discv5: Some(Discv5State::default()),
            ip_predictor: IpPredictor::default(),
            ip_override_locked: false,
            shared_local_node,
            // Tests drive the ENR directly; there is no publisher to listen to. The
            // sender is dropped, so `has_changed` errs and the refresh is a no-op.
            fork_id: watch::channel(None).1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        peer_table::{PeerTableServer, TARGET_PEERS},
        types::{INITIAL_ENR_SEQ, LocalNode},
    };
    use ethrex_common::H256;
    use secp256k1::SecretKey;
    use std::{
        net::Ipv4Addr,
        sync::{Arc, RwLock},
    };
    use tokio::net::UdpSocket;

    async fn make_server(ip: IpAddr, ip_override_locked: bool) -> DiscoveryServer {
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let pubkey = crate::utils::public_key_from_signing_key(&signer);
        let local_node = Node::new(ip, 30303, 30303, pubkey);
        let local_node_record =
            NodeRecord::from_node(&local_node, INITIAL_ENR_SEQ, &signer).unwrap();
        let shared_local_node = Arc::new(RwLock::new(LocalNode {
            node: local_node.clone(),
            record: local_node_record.clone(),
        }));
        let udp_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let peer_table = PeerTableServer::spawn(
            H256::random(),
            TARGET_PEERS,
            ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory).unwrap(),
            crate::netrestrict::NetRestrict::default(),
        );
        DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket,
            peer_table,
            config: DiscoveryConfig {
                discv4_enabled: false,
                discv5_enabled: false,
                nat_extip_set: ip_override_locked,
                netrestrict: crate::netrestrict::NetRestrict::default(),
            },
            discv4: None,
            discv5: None,
            ip_predictor: IpPredictor::default(),
            ip_override_locked,
            // No publisher in these tests; the dropped sender makes refresh a no-op.
            fork_id: tokio::sync::watch::channel(None).1,
            shared_local_node,
        }
    }

    /// apply_predicted_ip from unspecified -> public must update local_node, bump ENR seq,
    /// and propagate the new identity into the shared Arc.
    #[tokio::test]
    async fn apply_predicted_ip_updates_shared_arc() {
        let unspecified = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        let public_ip: IpAddr = "1.2.3.4".parse().unwrap();

        let mut server = make_server(unspecified, false).await;
        let original_seq = server.local_node_record.seq;

        server.apply_predicted_ip(public_ip, "test");

        assert_eq!(
            server.local_node.ip, public_ip,
            "local_node.ip must be updated"
        );
        assert!(
            server.local_node_record.seq > original_seq,
            "ENR seq must be bumped"
        );

        let guard = server.shared_local_node.read().unwrap();
        assert_eq!(
            guard.node.ip, public_ip,
            "shared Arc node.ip must be updated"
        );
        assert_eq!(
            guard.record.seq, server.local_node_record.seq,
            "shared Arc record.seq must match"
        );
    }

    /// With ip_override_locked=true (--nat.extip set), apply_predicted_ip is a no-op.
    #[tokio::test]
    async fn apply_predicted_ip_noop_when_locked() {
        let unspecified = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        let public_ip: IpAddr = "1.2.3.4".parse().unwrap();

        let mut server = make_server(unspecified, true).await;
        let original_seq = server.local_node_record.seq;

        server.apply_predicted_ip(public_ip, "test");

        assert_eq!(
            server.local_node.ip, unspecified,
            "local_node.ip must not change when locked"
        );
        assert_eq!(
            server.local_node_record.seq, original_seq,
            "ENR seq must not change when locked"
        );

        let guard = server.shared_local_node.read().unwrap();
        assert_eq!(
            guard.node.ip, unspecified,
            "shared Arc must not be updated when locked"
        );
    }
}
