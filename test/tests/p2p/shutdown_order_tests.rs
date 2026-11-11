//! The L1 node's shutdown order, pinned against the #7491 regression.
//!
//! #7491: each p2p actor stopped on its own ctrl-c listener, racing the main
//! shutdown path. A sync cycle that saw the stopped peer table before the
//! node's token was cancelled treated `ActorStopped` as fatal and called
//! `process::exit(2)`, skipping the database flush (6 of 25 failed CI runs).
//!
//! `shutdown_l1` now cancels the token before anything stops the peer table.
//! These tests watch the peer table stop and classify the `ActorStopped` a sync
//! cycle would see at that instant, exactly as `Syncer::start_sync` does. The
//! last test runs the old order and shows the same check catches it.

use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

use ethrex::shutdown::{L1Node, shutdown_l1};
use ethrex::utils::{NodeConfigFile, read_node_config_file, store_node_config_file};
use ethrex_common::{H256, H512};
use ethrex_p2p::{
    peer_table::{
        PeerTable, PeerTableServer, ToPeerTableServerRef as _, peer_table_server_protocol,
    },
    sync::{SyncError, SyncFailureAction},
    types::{LocalNode, Node, NodeRecord, SharedLocalNode},
};
use ethrex_storage::{EngineType, Store};
use spawned_concurrency::{error::ActorError, system::ActorSystem};
use spawned_test::tasks::Mock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const DEADLINE: Duration = Duration::from_secs(5);

fn node(port: u16) -> Node {
    Node::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        port,
        port,
        H512::from_low_u64_be(u64::from(port)),
    )
}

fn shared_local_node() -> SharedLocalNode {
    Arc::new(RwLock::new(LocalNode {
        node: node(30303),
        record: NodeRecord::default(),
    }))
}

/// Waits for `peer_table` to stop, then decides what a sync cycle failing on
/// it at that moment would do: the `NoProc` it gets (spawned 0.8: the table is
/// past its death point; `ActorStopped` before 0.8), judged against
/// whether the node had begun shutting down (`Syncer::is_shutting_down`).
fn sync_failure_when_stopped(
    peer_table: &PeerTable,
    cancel_token: &CancellationToken,
) -> JoinHandle<SyncFailureAction> {
    let peer_table = peer_table.clone();
    let cancel_token = cancel_token.clone();
    tokio::spawn(async move {
        peer_table.wait_exit().await;
        SyncError::PeerTableError(ActorError::NoProc).failure_action(cancel_token.is_cancelled())
    })
}

/// A node with only its peer table owned by an actor system, which is all the
/// shutdown order concerns.
fn l1_node(datadir: PathBuf) -> L1Node {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let system = ActorSystem::new().expect("actor system");
    let group = system.async_group("p2p", 1).expect("p2p group");
    let peer_table = PeerTableServer::spawn_on(H256::random(), 10, store.clone(), &group)
        .expect("peer table admitted");
    L1Node {
        datadir,
        cancel_token: CancellationToken::new(),
        actor_names: vec![(peer_table.id(), "peer_table")],
        peer_table,
        shared_local_node: shared_local_node(),
        store,
        system,
        discovery: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_cancels_the_node_before_stopping_the_peer_table() {
    let datadir = tempfile::tempdir().expect("datadir");
    let node = l1_node(datadir.path().to_path_buf());
    let sync_failure = sync_failure_when_stopped(&node.peer_table, &node.cancel_token);

    let outcome = shutdown_l1(node, DEADLINE).await;

    assert!(
        outcome.complete,
        "every owned actor stops within the deadline"
    );
    assert_eq!(
        sync_failure.await.expect("observer"),
        SyncFailureAction::StopForShutdown,
        "a sync cycle failing on the stopped peer table must not exit the node"
    );
    // The peer snapshot was taken while the table still answered (an empty
    // table here), and saved next to the record.
    let saved = read_node_config_file(datadir.path())
        .expect("readable")
        .expect("node config saved");
    assert!(saved.known_peers.is_empty());
}

/// Negative control: the order before this change, actors stopped while the
/// node still runs and the token cancelled afterwards. The same check reports
/// the `exit(2)` #7491 hit, so reintroducing that order fails the test above.
#[tokio::test(flavor = "multi_thread")]
async fn the_7491_order_would_exit_the_node() {
    let node = l1_node(PathBuf::from("memory"));
    let sync_failure = sync_failure_when_stopped(&node.peer_table, &node.cancel_token);

    let report = node.system.shutdown(DEADLINE).await;
    assert!(report.complete);
    let action = sync_failure.await.expect("observer");
    node.cancel_token.cancel();

    assert_eq!(action, SyncFailureAction::Exit);
}

/// The peer snapshot reads the table through a protocol reference, so a mock
/// stands in for a real table with connected peers, which would need live
/// RLPx sessions to fill.
#[tokio::test]
async fn shutdown_saves_the_connected_peers() {
    let datadir = tempfile::tempdir().expect("datadir");
    let peers = vec![node(30304), node(30305)];
    let replied = peers.clone();
    let table = Mock::new()
        .on::<peer_table_server_protocol::GetConnectedNodes>(move |_| replied.clone())
        .start();

    let config = NodeConfigFile::new(
        table.to_peer_table_server_ref(),
        NodeRecord::default(),
        datadir.path(),
    )
    .await;

    assert_eq!(config.known_peers, peers);
}

/// A peer table that cannot answer in time keeps the peers the previous run
/// saved, instead of overwriting them with an empty list as before.
#[tokio::test]
async fn a_slow_peer_table_keeps_the_previously_saved_peers() {
    let datadir = tempfile::tempdir().expect("datadir");
    let previous = vec![node(30306)];
    store_node_config_file(
        NodeConfigFile {
            known_peers: previous.clone(),
            node_record: NodeRecord::default(),
        },
        datadir.path().join("node_config.json"),
    );
    let table = Mock::new()
        .on::<peer_table_server_protocol::GetConnectedNodes>(|_| Vec::new())
        .delay::<peer_table_server_protocol::GetConnectedNodes>(Duration::from_secs(30))
        .start();

    let config = NodeConfigFile::new(
        table
            .actor_ref()
            .with_request_timeout(Duration::from_millis(200))
            .to_peer_table_server_ref(),
        NodeRecord::default(),
        datadir.path(),
    )
    .await;

    assert_eq!(config.known_peers, previous);
}

/// The same fallback when the peer table has already stopped.
#[tokio::test]
async fn a_stopped_peer_table_keeps_the_previously_saved_peers() {
    let datadir = tempfile::tempdir().expect("datadir");
    let previous = vec![node(30307)];
    store_node_config_file(
        NodeConfigFile {
            known_peers: previous.clone(),
            node_record: NodeRecord::default(),
        },
        datadir.path().join("node_config.json"),
    );
    let table = Mock::new().start();
    table.actor_ref().context().stop();
    table.actor_ref().join().await;

    let config = NodeConfigFile::new(
        table.to_peer_table_server_ref(),
        NodeRecord::default(),
        datadir.path(),
    )
    .await;

    assert_eq!(config.known_peers, previous);
}
