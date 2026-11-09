//! The discovery actor under a OneForOne supervisor.
//!
//! Discovery is the one long-lived p2p actor that restarts safely, so it is the
//! one under a supervisor:
//!
//! - Nothing holds a reference to it. It is fed by its own UDP listener and
//!   timers, and talks to the peer table, so a replacement needs no rebinding.
//! - Its state is soft: discv4/discv5 sessions, pending pings and lookups, and
//!   the IP votes. A fresh instance rebuilds them from the peer table (which
//!   keeps every contact found so far) and the bootnodes, as at startup.
//! - It republishes what the previous instance published: the local node and
//!   ENR it mirrored into the shared identity, and the current fork id.
//!
//! The UDP socket is bound once and shared by every generation. A stopped
//! generation's listener stops with it (`spawn_listener` ends on actor stop),
//! so two generations never split the socket's packets.
//!
//! The other p2p actors stay unsupervised; see `init_l1` in the ethrex binary.
//!
//! Before this, a panic in a discovery handler stopped discovery for the rest
//! of the run: the node kept its connected peers but found no new ones.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use ethrex_common::types::ForkId;
use secp256k1::SecretKey;
use spawned_concurrency::{
    supervision::{
        Backoff, ChildSpec, RestartMode, SupervisedChild, Supervisor, SupervisorConfig,
        SupervisorStrategy,
    },
    system::{ActorSystem, ExecutionGroup},
};
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tracing::{error, warn};

use super::{DiscoveryConfig, DiscoveryServer, DiscoveryServerError};
use crate::{
    peer_table::PeerTable,
    types::{Node, NodeRecord, SharedLocalNode},
};

/// At most three restarts a minute, the first after a second and backing off
/// to ten. Discovery that keeps crashing faster than that (a packet that
/// reliably panics it, say) exhausts the budget: the supervisor gives up with
/// an error, and the node runs on without discovery, as it did before
/// supervision. Its other actors are not in this supervisor, so giving up
/// stops nothing else.
pub fn supervisor_config() -> SupervisorConfig {
    SupervisorConfig {
        strategy: SupervisorStrategy::OneForOne,
        max_restarts: 3,
        restart_window: Duration::from_secs(60),
        backoff: Backoff {
            initial: Duration::from_secs(1),
            maximum: Duration::from_secs(10),
            stable_after: Duration::from_secs(60),
            ..Backoff::default()
        },
        ..SupervisorConfig::default()
    }
}

/// What every discovery generation is built from.
#[derive(Clone)]
pub(crate) struct DiscoveryInputs {
    pub local_node: Node,
    pub local_node_record: NodeRecord,
    pub signer: SecretKey,
    pub udp_socket: Arc<UdpSocket>,
    pub peer_table: PeerTable,
    pub bootnodes: Vec<Node>,
    pub config: DiscoveryConfig,
    pub shared_local_node: SharedLocalNode,
    pub fork_id: watch::Receiver<Option<ForkId>>,
}

impl DiscoveryInputs {
    /// The local node and ENR a new generation starts from.
    ///
    /// The first generation advertises what startup built. A generation keeps
    /// the shared identity current as it edits its ENR (new external IP, new
    /// fork id), so once the shared record differs from what it was at startup,
    /// a replacement continues from it: rolling back to the startup record
    /// would re-announce a stale IP with a sequence number peers already saw.
    fn identity(&self, record_at_startup: &NodeRecord) -> (Node, NodeRecord) {
        let shared = self
            .shared_local_node
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if shared.record != *record_at_startup {
            (shared.node.clone(), shared.record.clone())
        } else {
            (self.local_node.clone(), self.local_node_record.clone())
        }
    }

    async fn build(
        self,
        record_at_startup: &NodeRecord,
    ) -> Result<DiscoveryServer, DiscoveryServerError> {
        let (local_node, local_node_record) = self.identity(record_at_startup);
        DiscoveryServer::build(
            local_node,
            local_node_record,
            self.signer,
            self.udp_socket,
            self.peer_table,
            self.bootnodes,
            self.config,
            self.shared_local_node,
            self.fork_id,
        )
        .await
    }
}

/// Starts discovery under its own supervisor in `system`, on `group`, and
/// waits for the first generation to be ready.
pub(crate) async fn start_supervised(
    system: &ActorSystem,
    group: &ExecutionGroup,
    inputs: DiscoveryInputs,
) -> Result<SupervisedChild<DiscoveryServer>, DiscoveryServerError> {
    let local_addr: Option<SocketAddr> = inputs.udp_socket.local_addr().ok();
    let record_at_startup = Arc::new(
        inputs
            .shared_local_node
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record
            .clone(),
    );
    let supervision_error = |err: &dyn std::fmt::Display| {
        DiscoveryServerError::InternalError(format!("discovery supervisor: {err}"))
    };
    let mut tree = system
        .supervisor(supervisor_config())
        .map_err(|err| supervision_error(&err))?;
    let discovery = tree
        .add(
            // Transient: only a crash restarts it. A deliberate stop (its
            // `shutdown` message) leaves it stopped.
            ChildSpec::new("discovery", group, move |_| {
                let inputs = inputs.clone();
                let record_at_startup = record_at_startup.clone();
                async move { inputs.build(&record_at_startup).await }
            })
            .restart(RestartMode::Transient),
        )
        .map_err(|err| supervision_error(&err))?;
    let supervisor = tree.start().await.map_err(|err| supervision_error(&err))?;
    watch(discovery.clone(), supervisor, local_addr);
    Ok(discovery)
}

/// Logs every restart of the discovery actor, and the supervisor giving up if
/// it exhausts its budget. Spawned logs neither itself.
fn watch(
    discovery: SupervisedChild<DiscoveryServer>,
    supervisor: Supervisor,
    local_addr: Option<SocketAddr>,
) {
    tokio::spawn(async move {
        let mut seen = discovery.snapshot().generation;
        while discovery.wait_for_generation(seen).await.is_ok() {
            let snapshot = discovery.snapshot();
            warn!(
                generation = snapshot.generation,
                last_exit = ?snapshot.last_exit,
                last_error = ?snapshot.last_error,
                ?local_addr,
                "Supervisor restarted the discovery server"
            );
            seen = snapshot.generation;
        }
        supervisor.join().await;
        if let Some(failure) = supervisor.failure() {
            error!(
                %failure,
                "Discovery supervisor gave up; the node runs on without discovery"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        discovery::InjectCrash,
        peer_table::{PeerTableServer, PeerTableServerProtocol as _, TARGET_PEERS},
        types::{INITIAL_ENR_SEQ, LocalNode},
    };
    use ethrex_common::H256;
    use spawned_concurrency::ExitReason;
    use std::{
        net::{IpAddr, Ipv4Addr},
        sync::RwLock,
    };

    /// Longer than the supervisor's worst backoff (10 s) plus a rebuild.
    const RECOVERY: Duration = Duration::from_secs(15);

    async fn inputs() -> DiscoveryInputs {
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let pubkey = crate::utils::public_key_from_signing_key(&signer);
        let local_node = Node::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 30303, 30303, pubkey);
        let local_node_record =
            NodeRecord::from_node(&local_node, INITIAL_ENR_SEQ, &signer).unwrap();
        let shared_local_node = Arc::new(RwLock::new(LocalNode {
            node: local_node.clone(),
            record: local_node_record.clone(),
        }));
        DiscoveryInputs {
            local_node,
            local_node_record,
            signer,
            udp_socket: Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()),
            peer_table: PeerTableServer::spawn(
                H256::random(),
                TARGET_PEERS,
                ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory).unwrap(),
            ),
            bootnodes: Vec::new(),
            config: DiscoveryConfig {
                discv4_enabled: false,
                discv5_enabled: true,
                nat_extip_set: false,
            },
            shared_local_node,
            // No publisher: the dropped sender makes the fork id refresh a no-op.
            fork_id: watch::channel(None).1,
        }
    }

    /// The fault-injection crash is a real handler panic: the supervisor builds
    /// a fresh discovery actor, and the peer table it feeds is untouched.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_crashed_discovery_server_is_replaced() {
        let system = ActorSystem::new().unwrap();
        let group = system.async_group("p2p", 1).unwrap();
        let inputs = inputs().await;
        let peer_table = inputs.peer_table.clone();
        let discovery = start_supervised(&system, &group, inputs).await.unwrap();

        let first = discovery.wait_ready().await.unwrap();
        let generation = discovery.snapshot().generation;
        first.send(InjectCrash).unwrap();
        let second = tokio::time::timeout(RECOVERY, discovery.wait_for_generation(generation))
            .await
            .expect("the supervisor restarts discovery")
            .unwrap();

        assert_ne!(first.id(), second.id());
        assert!(matches!(
            discovery.snapshot().last_exit,
            Some(ExitReason::Panic(_))
        ));
        assert!(peer_table.peer_count().await.is_ok());
        assert!(system.shutdown(RECOVERY).await.complete);
    }

    /// Discovery that keeps crashing exhausts the budget: the supervisor gives
    /// up instead of looping, and the rest of p2p keeps running.
    #[tokio::test(flavor = "multi_thread")]
    async fn discovery_that_keeps_crashing_exhausts_the_budget() {
        let system = ActorSystem::new().unwrap();
        let group = system.async_group("p2p", 1).unwrap();
        let inputs = inputs().await;
        let peer_table = inputs.peer_table.clone();
        let discovery = start_supervised(&system, &group, inputs).await.unwrap();

        let budget = supervisor_config().max_restarts;
        for _ in 0..=budget {
            let Ok(current) = discovery.wait_ready().await else {
                break;
            };
            let generation = discovery.snapshot().generation;
            current.send(InjectCrash).unwrap();
            if tokio::time::timeout(RECOVERY, discovery.wait_for_generation(generation))
                .await
                .expect("each crash is either replaced or ends supervision")
                .is_err()
            {
                break;
            }
        }

        assert!(
            discovery.current().is_err(),
            "no discovery after the budget"
        );
        assert!(peer_table.peer_count().await.is_ok());
        assert!(system.shutdown(RECOVERY).await.complete);
    }

    /// A replacement continues from the identity its predecessor published,
    /// and the first generation from the one startup built.
    #[tokio::test]
    async fn a_replacement_starts_from_the_published_identity() {
        let inputs = inputs().await;
        let at_startup = inputs.shared_local_node.read().unwrap().record.clone();
        assert_eq!(inputs.identity(&at_startup).1, inputs.local_node_record);

        let moved = Node::new(
            IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
            30303,
            30303,
            inputs.local_node.public_key,
        );
        let republished =
            NodeRecord::from_node(&moved, INITIAL_ENR_SEQ + 1, &inputs.signer).unwrap();
        {
            let mut shared = inputs.shared_local_node.write().unwrap();
            shared.node = moved.clone();
            shared.record = republished.clone();
        }

        let (node, record) = inputs.identity(&at_startup);
        assert_eq!(node.ip, moved.ip);
        assert_eq!(record, republished);
    }
}
