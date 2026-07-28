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
    types::{INITIAL_ENR_SEQ, Node, NodeRecord, SharedLocalNode},
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
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio::net::UdpSocket;
use tokio_util::udp::UdpFramed;
use tracing::{debug, error, info, trace, warn};

use super::{DiscoveryConfig, codec::DiscriminatingCodec, lookup_interval_function};

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
    pub(crate) store: Store,
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
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn(
        storage: Store,
        local_node: Node,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        peer_table: PeerTable,
        bootnodes: Vec<Node>,
        config: DiscoveryConfig,
        shared_local_node: SharedLocalNode,
    ) -> Result<(), DiscoveryServerError> {
        debug!("Starting discovery server");

        let mut local_node_record = NodeRecord::from_node(&local_node, INITIAL_ENR_SEQ, &signer)
            .expect("Failed to create local node record");
        match storage.get_fork_id().await {
            Ok(fork_id) => local_node_record
                .set_fork_id(fork_id, &signer)
                .expect("Failed to set fork_id on local node record"),
            // Without an `eth` entry geth's dial-candidate filter skips us outright
            // (`NewNodeFilter` in eth/protocols/eth/discovery.go returns false when the
            // entry fails to load), so this is worth surfacing. `refresh_fork_id` fills
            // the entry in on the next prune tick once the store can answer.
            Err(e) => warn!(
                error = ?e,
                "Could not derive fork id for the local ENR; publishing a record without an `eth` entry for now"
            ),
        }

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

        let ip_override_locked = config.nat_extip_set;
        let mut server = Self {
            local_node: local_node.clone(),
            local_node_record,
            signer,
            udp_socket: udp_socket.clone(),
            store: storage,
            peer_table: peer_table.clone(),
            ip_predictor: IpPredictor::default(),
            ip_override_locked,
            config,
            discv4,
            discv5,
            shared_local_node,
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
        if let Some(discv5) = &mut self.discv5 {
            discv5.cleanup_stale_entries();
        }
        if let Some(ip) = self.ip_predictor.check_timeout() {
            self.apply_predicted_ip(ip, "timeout");
        }
        self.refresh_fork_id().await;
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

    /// Re-derives the `eth` ENR entry from the current chain head, re-signing the record
    /// with a bumped seq when the fork id changed.
    ///
    /// The entry is otherwise only set once, when the discovery server starts, so a node
    /// that is already running when a fork activates keeps advertising the pre-fork id for
    /// the rest of the process' life. geth instead re-derives the entry on every chain head
    /// event (`StartENRUpdater` in eth/protocols/eth/discovery.go) and filters dial
    /// candidates on it (`NewNodeFilter`), which is what makes a current entry matter on a
    /// discv5-only network, where the ENR is the only fork signal a peer has before the
    /// RLPx handshake.
    async fn refresh_fork_id(&mut self) {
        let Ok(fork_id) = self.store.get_fork_id().await else {
            return;
        };
        if self.local_node_record.get_fork_id() == Some(&fork_id) {
            return;
        }
        let previous = self.local_node_record.get_fork_id().cloned();
        if let Err(e) = self
            .local_node_record
            .set_fork_id(fork_id.clone(), &self.signer)
        {
            error!(error = ?e, "Failed to set the new fork id on the local ENR");
            return;
        }
        info!(
            ?previous,
            new = ?fork_id,
            seq = self.local_node_record.seq,
            "Chain fork id changed, updated local ENR"
        );
        // Propagate to the shared Arc so RPC and shutdown see the current identity.
        self.shared_local_node
            .write()
            .expect("shared_local_node poisoned")
            .record = self.local_node_record.clone();
    }

    pub(crate) async fn get_lookup_interval(&self) -> Duration {
        let peer_completion = self
            .peer_table
            .target_peers_completion()
            .await
            .unwrap_or_default();
        lookup_interval_function(
            peer_completion,
            ITERATIVE_LOOKUP_INITIAL_MS,
            ITERATIVE_LOOKUP_INTERVAL_MS,
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
            store: Store::new("", ethrex_storage::EngineType::InMemory)
                .expect("Failed to create store"),
            peer_table,
            config: DiscoveryConfig {
                discv4_enabled: false,
                discv5_enabled: true,
                initial_lookup_interval: 1000.0,
                nat_extip_set: false,
            },
            discv4: None,
            discv5: Some(Discv5State::default()),
            ip_predictor: IpPredictor::default(),
            ip_override_locked: false,
            shared_local_node,
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
    use ethrex_common::{
        H256,
        types::{Block, BlockHeader, Genesis},
    };
    use ethrex_storage::EngineType;
    use secp256k1::SecretKey;
    use std::{
        net::Ipv4Addr,
        sync::{Arc, RwLock},
    };
    use tokio::net::UdpSocket;

    const FORK_TIMESTAMP: u64 = 2_000_000_000;

    /// Store whose only scheduled fork is Amsterdam, at `FORK_TIMESTAMP`.
    async fn store_with_pending_fork() -> Store {
        let mut genesis = Genesis::default();
        genesis.config.chain_id = 1;
        genesis.config.amsterdam_time = Some(FORK_TIMESTAMP);
        let mut store = Store::new("", EngineType::InMemory).expect("failed to create store");
        store
            .add_initial_state(genesis)
            .await
            .expect("failed to seed genesis");
        store
    }

    /// Moves the store's head to a block whose timestamp is past `FORK_TIMESTAMP`.
    async fn advance_head_past_fork(store: &Store) {
        let genesis_hash = store
            .get_canonical_block_hash(0)
            .await
            .unwrap()
            .expect("missing genesis hash");
        let header = BlockHeader {
            number: 1,
            parent_hash: genesis_hash,
            timestamp: FORK_TIMESTAMP,
            ..Default::default()
        };
        let hash = header.hash();
        store
            .add_block(Block {
                header,
                ..Default::default()
            })
            .await
            .unwrap();
        store
            .forkchoice_update(vec![(1, hash)], 1, hash, None, None)
            .await
            .unwrap();
    }

    async fn make_server(ip: IpAddr, ip_override_locked: bool) -> DiscoveryServer {
        let store = Store::new("", EngineType::InMemory).unwrap();
        make_server_with_store(ip, ip_override_locked, store).await
    }

    async fn make_server_with_store(
        ip: IpAddr,
        ip_override_locked: bool,
        store: Store,
    ) -> DiscoveryServer {
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let pubkey = crate::utils::public_key_from_signing_key(&signer);
        let local_node = Node::new(ip, 30303, 30303, pubkey);
        let mut local_node_record =
            NodeRecord::from_node(&local_node, INITIAL_ENR_SEQ, &signer).unwrap();
        // Mirror the startup path: seed the `eth` entry when the store can answer.
        if let Ok(fork_id) = store.get_fork_id().await {
            local_node_record.set_fork_id(fork_id, &signer).unwrap();
        }
        let shared_local_node = Arc::new(RwLock::new(LocalNode {
            node: local_node.clone(),
            record: local_node_record.clone(),
        }));
        let udp_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let peer_table = PeerTableServer::spawn(
            H256::random(),
            TARGET_PEERS,
            Store::new("", EngineType::InMemory).unwrap(),
        );
        DiscoveryServer {
            local_node,
            local_node_record,
            signer,
            udp_socket,
            store,
            peer_table,
            config: DiscoveryConfig {
                discv4_enabled: false,
                discv5_enabled: false,
                initial_lookup_interval: 1000.0,
                nat_extip_set: ip_override_locked,
            },
            discv4: None,
            discv5: None,
            ip_predictor: IpPredictor::default(),
            ip_override_locked,
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

    /// Crossing a fork must re-derive the `eth` ENR entry, bump the record's seq, and
    /// propagate the new record into the shared Arc, so peers filtering dial candidates
    /// on the ENR (and RPC) see the post-fork id.
    #[tokio::test]
    async fn refresh_fork_id_updates_enr_after_fork() {
        let store = store_with_pending_fork().await;
        let localhost = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let mut server = make_server_with_store(localhost, false, store.clone()).await;

        let pre_fork_id = server.local_node_record.get_fork_id().cloned().unwrap();
        let pre_fork_seq = server.local_node_record.seq;
        assert_eq!(
            pre_fork_id.fork_next, FORK_TIMESTAMP,
            "pre-fork ENR must announce the upcoming fork"
        );

        advance_head_past_fork(&store).await;
        server.refresh_fork_id().await;

        let post_fork_id = server.local_node_record.get_fork_id().cloned().unwrap();
        assert_ne!(
            post_fork_id.fork_hash, pre_fork_id.fork_hash,
            "fork hash must be re-derived once the fork is passed"
        );
        assert_eq!(
            post_fork_id.fork_next, 0,
            "no further fork is scheduled, so fork_next must be 0"
        );
        assert!(
            server.local_node_record.seq > pre_fork_seq,
            "ENR seq must be bumped so peers pick up the new record"
        );
        assert_eq!(
            post_fork_id,
            store.get_fork_id().await.unwrap(),
            "ENR must match the fork id derived from the current head"
        );

        let guard = server.shared_local_node.read().unwrap();
        assert_eq!(
            guard.record, server.local_node_record,
            "shared Arc must carry the refreshed record"
        );
    }

    /// An unchanged fork id must leave the record alone: re-signing on every prune tick
    /// would churn the seq and force peers to re-fetch an identical record.
    #[tokio::test]
    async fn refresh_fork_id_is_a_noop_when_unchanged() {
        let store = store_with_pending_fork().await;
        let localhost = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let mut server = make_server_with_store(localhost, false, store).await;

        let before = server.local_node_record.clone();
        server.refresh_fork_id().await;

        assert_eq!(
            server.local_node_record, before,
            "an unchanged fork id must not touch the record"
        );
    }
}
