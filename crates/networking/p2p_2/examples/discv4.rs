use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use ethrex_blockchain::{Blockchain, BlockchainType};
use ethrex_common::H512;
use ethrex_p2p_2::{
    discv4::{Kademlia, server::DiscoveryServer, side_car::DiscoverySideCar},
    monitor::{app::Monitor, init_terminal, restore_terminal},
    network::P2PContext,
    rlpx::initiator::RLPxInitiator,
    types::{Node, NodeRecord},
};
use ethrex_storage::Store;
use ethrex_vm::EvmEngine;
use k256::{PublicKey, ecdsa::SigningKey, elliptic_curve::sec1::ToEncodedPoint};
use rand::rngs::OsRng;
use tokio::{net::UdpSocket, sync::Mutex};
use tokio_util::task::TaskTracker;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, FmtSubscriber, filter::Directive, layer::SubscriberExt};
use tui_logger::{LevelFilter, TuiTracingSubscriberLayer};

pub const HOLESKY_GENESIS_PATH: &str = "cmd/ethrex/networks/holesky/genesis.json";
pub const HOLESKY_GENESIS_CONTENTS: &str =
    include_str!("../../../../cmd/ethrex/networks/holesky/genesis.json");

#[tokio::main]
async fn main() {
    init_tracing();

    let signer = SigningKey::random(&mut OsRng);

    let local_node = Node::new(
        IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        30303,
        30303,
        public_key_from_signing_key(&signer),
    );

    let kademlia = Kademlia::new();

    let udp_socket = Arc::new(
        UdpSocket::bind(local_node.udp_addr())
            .await
            .expect("Failed to bind udp socket"),
    );

    let _ = DiscoveryServer::spawn(
        local_node.clone(),
        signer.clone(),
        udp_socket.clone(),
        kademlia.clone(),
        bootnode(),
    )
    .await
    .inspect_err(|e| {
        error!("Failed to start discovery server: {e}");
    });

    let _ = DiscoverySideCar::spawn(
        local_node.clone(),
        signer.clone(),
        udp_socket,
        kademlia.clone(),
    )
    .await
    .inspect_err(|e| {
        error!("Failed to start discovery side car: {e}");
    });

    let local_node_record = NodeRecord::from_node(&local_node, 1, &signer).unwrap();

    let store = Store::new("./db", ethrex_storage::EngineType::InMemory).unwrap();

    let genesis = serde_json::from_str(HOLESKY_GENESIS_CONTENTS).unwrap();
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to create genesis block");

    let blockchain = Blockchain::new(EvmEngine::LEVM, store.clone(), BlockchainType::L1).into();

    let context = P2PContext::new(
        local_node.clone(),
        Arc::new(Mutex::new(local_node_record)),
        TaskTracker::new(),
        signer.clone(),
        kademlia.clone(),
        store,
        blockchain,
        "0.0.1".to_owned(),
    );

    let _ = RLPxInitiator::spawn(context, local_node, signer, kademlia.clone())
        .await
        .inspect_err(|e| {
            error!("Failed to start RLPx Initiator: {e}");
        });

    // Barrani kademlia contacts counter
    let kademlia_counter_handle = tokio::spawn(async move {
        let start = std::time::Instant::now();
        loop {
            let elapsed = start.elapsed();
            let number_of_peers = kademlia.number_of_peers().await;
            let number_of_tried_peers = kademlia.number_of_tried_peers().await;
            info!(
                contacts = kademlia.table.lock().await.len(),
                number_of_peers = number_of_peers,
                number_of_tried_peers = number_of_tried_peers,
                elapsed = format_duration(elapsed)
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    // let mut terminal = init_terminal().expect("Failed to initialize terminal");

    // let mut monitor = Monitor::new("Ethrex P2P");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("Received Ctrl+C, shutting down...");
            // restore_terminal(&mut terminal).expect("Failed to restore terminal");
            kademlia_counter_handle.abort();
        }
        // _ = monitor.start(&mut terminal) => {
        //     println!("Monitor has exited, shutting down...");
        //     restore_terminal(&mut terminal).expect("Failed to restore terminal");
        //     kademlia_counter_handle.abort();
        // }
    }
}

pub fn init_tracing() {
    let log_filter = EnvFilter::builder().from_env_lossy();
    // .add_directive(Directive::from(opts.log_level));
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(log_filter)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
}

// pub fn init_tracing() {
//     let level_filter = EnvFilter::builder().parse_lossy("debug");
//     let subscriber = tracing_subscriber::registry()
//         .with(TuiTracingSubscriberLayer)
//         .with(level_filter);
//     tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
//     tui_logger::init_logger(LevelFilter::max()).expect("Failed to initialize tui_logger");
// }

pub fn public_key_from_signing_key(signer: &SigningKey) -> H512 {
    let public_key = PublicKey::from(signer.verifying_key());
    let encoded = public_key.to_encoded_point(false);
    H512::from_slice(&encoded.as_bytes()[1..])
}

pub fn bootnode() -> Node {
    Node::from_enode_url(
        "enode://ac906289e4b7f12df423d654c5a962b6ebe5b3a74cc9e06292a85221f9a64a6f1cfdd6b714ed6dacef51578f92b34c60ee91e9ede9c7f8fadc4d347326d95e2b@146.190.13.128:30303",
    ).expect("Failed to parse bootnode enode URL")
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
