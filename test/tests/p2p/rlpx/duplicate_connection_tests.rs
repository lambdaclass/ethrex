//! A second RLPx connection to a node that is already connected must not take the
//! live connection out of the peer table when it closes. Otherwise the node stays
//! connected while `net_peerCount`, `admin_peers`, peer selection and the
//! transaction broadcaster no longer see it.

use std::{
    fs::File,
    io::BufReader,
    net::{IpAddr, Ipv4Addr, TcpListener},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use ethrex_blockchain::Blockchain;
use ethrex_common::types::Genesis;
use ethrex_p2p::{
    network::P2PContext,
    peer_table::{PeerTable, PeerTableServer, PeerTableServerProtocol as _},
    rlpx::initiator::{RLPxInitiator, RlpxInitiatorProtocol},
    test_utils::serve_p2p_requests,
    types::{NetworkConfig, Node},
    utils::public_key_from_signing_key,
};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use tokio_util::task::TaskTracker;

struct TestNode<I: RlpxInitiatorProtocol> {
    node: Node,
    table: PeerTable,
    blockchain: Arc<Blockchain>,
    initiator: I,
}

fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .expect("bind an ephemeral port")
        .port()
}

fn genesis() -> Genesis {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/genesis/l1.json");
    serde_json::from_reader(BufReader::new(File::open(path).expect("open l1 genesis")))
        .expect("parse l1 genesis")
}

/// A node that accepts inbound RLPx connections on localhost and can dial others,
/// with no discovery, keeping at most `target_peers` peers.
async fn spawn_node(key_byte: u8, target_peers: usize) -> TestNode<impl RlpxInitiatorProtocol> {
    let signer = SecretKey::from_byte_array(&[key_byte; 32]).expect("valid secret key");
    let port = free_port();
    let node = Node::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        port,
        port,
        public_key_from_signing_key(&signer),
    );
    let mut store = Store::new("", EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(genesis())
        .await
        .expect("add genesis state");
    let table = PeerTableServer::spawn(target_peers);
    let blockchain = Arc::new(Blockchain::default_with_store(store.clone()));
    let context = P2PContext::new(
        node.clone(),
        NetworkConfig::from_node(&node),
        TaskTracker::new(),
        signer,
        table.clone(),
        store.clone(),
        blockchain.clone(),
        "ethrex/test".to_string(),
        None,
        1000,
        100.0,
    )
    .expect("p2p context");
    tokio::spawn(serve_p2p_requests(context.clone()));
    let initiator = RLPxInitiator::spawn(context);
    TestNode {
        node,
        table,
        blockchain,
        initiator,
    }
}

async fn peer_count(table: &PeerTable) -> usize {
    table.peer_count().await.expect("peer count")
}

/// Waits up to `timeout` for `table` to hold `expected` peers.
async fn wait_for_peer_count(table: &PeerTable, expected: usize, timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        let count = peer_count(table).await;
        if count == expected || Instant::now() >= deadline {
            return count;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A is connected to B, then dials B again. When the second connection closes, A must
/// still list B and keep what B's live connection told it, here the EIP-8070 cell
/// availability B advertised: the first connection is still up.
async fn dialing_twice_keeps_the_live_peer_listed(b_target_peers: usize) {
    let a = spawn_node(0x11, 10).await;
    let b = spawn_node(0x22, b_target_peers).await;

    a.initiator
        .initiate(b.node.clone())
        .expect("dial B the first time");
    assert_eq!(
        wait_for_peer_count(&a.table, 1, Duration::from_secs(10)).await,
        1,
        "A must connect to B"
    );
    assert_eq!(
        wait_for_peer_count(&b.table, 1, Duration::from_secs(10)).await,
        1,
        "B must list A"
    );
    // Stands in for a cell mask B announced over the live connection.
    let b_cell_mask = 0b1011;
    a.blockchain
        .mempool
        .record_peer_cell_availability(b.node.node_id(), b_cell_mask)
        .expect("record B's cell availability");

    a.initiator
        .initiate(b.node.clone())
        .expect("dial B a second time");
    // The refusal and teardown of the second connection take milliseconds; give them
    // ample time, and fail as soon as A drops B.
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        assert_eq!(
            peer_count(&a.table).await,
            1,
            "closing the refused duplicate must not remove B from A's table"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(peer_count(&b.table).await, 1, "B still lists A");
    assert_eq!(
        a.blockchain
            .mempool
            .peer_cell_mask(b.node.node_id())
            .expect("read B's cell availability"),
        Some(b_cell_mask),
        "closing the refused duplicate must not forget the cells B advertised"
    );
}

/// B accepts a single peer, so it refuses the second connection the way any client
/// refuses one it does not want (geth answers a duplicate with `AlreadyConnected`).
#[tokio::test]
async fn a_refused_duplicate_connection_keeps_the_live_peer_listed() {
    dialing_twice_keeps_the_live_peer_listed(1).await;
}

/// Both nodes have room for more peers, so the second connection gets as far as
/// registering, finds the node already listed and is refused with `AlreadyConnected`.
#[tokio::test]
async fn a_duplicate_refused_as_already_connected_keeps_the_live_peer_listed() {
    dialing_twice_keeps_the_live_peer_listed(10).await;
}
