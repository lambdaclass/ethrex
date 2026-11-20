//! Ctrl-C must not stop the peer table. The node's shutdown, which also runs on Ctrl-C,
//! reads the connected peers from the table to store them in `node_config.json`; a
//! stopped table answers with an error and the node saves an empty peer list.
//!
//! Its own binary: it delivers SIGINT to the whole test process.
#![cfg(unix)]

use std::{process::Command, time::Duration};

use ethrex_common::H256;
use ethrex_p2p::peer_table::{PeerTableServer, PeerTableServerProtocol as _};
use ethrex_storage::{EngineType, Store};
use tokio::signal::unix::{SignalKind, signal};

#[tokio::test]
async fn peer_table_keeps_serving_after_ctrl_c() {
    // Handle SIGINT here first so the signal doesn't terminate the test process.
    let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");

    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let table = PeerTableServer::spawn(H256::random(), 10, store);
    table.peer_count().await.expect("peer table is running");
    // Give any Ctrl-C listener the table starts time to register.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let status = Command::new("kill")
        .args(["-INT", &std::process::id().to_string()])
        .status()
        .expect("run kill");
    assert!(status.success(), "kill -INT failed");
    tokio::time::timeout(Duration::from_secs(5), sigint.recv())
        .await
        .expect("SIGINT delivered");
    // Leave time for a listener to stop the table.
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(
        table.get_connected_nodes().await.is_ok(),
        "peer table stopped on Ctrl-C"
    );
}
