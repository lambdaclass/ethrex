use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

use ethrex_common::H512;
use ethrex_p2p_2::{
    discv4::{server::DiscoveryServer, side_car::DiscoverySideCar},
    monitor::{app::Monitor, init_terminal, restore_terminal},
    types::Node,
};
use k256::{PublicKey, ecdsa::SigningKey, elliptic_curve::sec1::ToEncodedPoint};
use rand::rngs::OsRng;
use tokio::{net::UdpSocket, sync::Mutex};
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt};
use tui_logger::{LevelFilter, TuiTracingSubscriberLayer};

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

    let kademlia = Arc::new(Mutex::new(HashMap::new()));

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

    let _ = DiscoverySideCar::spawn(local_node, signer, udp_socket, kademlia.clone())
        .await
        .inspect_err(|e| {
            error!("Failed to start discovery side car: {e}");
        });

    // Barrani kademlia contacts counter
    let kademlia_clone = kademlia.clone();
    let kademlia_counter_handle = tokio::spawn(async move {
        let start = std::time::Instant::now();
        loop {
            let elapsed = start.elapsed();
            info!(
                contacts = kademlia_clone.lock().await.len(),
                elapsed = format_duration(elapsed)
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    let mut terminal = init_terminal().expect("Failed to initialize terminal");

    let mut monitor = Monitor::new("Ethrex P2P", kademlia);

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("Received Ctrl+C, shutting down...");
            restore_terminal(&mut terminal).expect("Failed to restore terminal");
            kademlia_counter_handle.abort();
        }
        _ = monitor.start(&mut terminal) => {
            println!("Monitor has exited, shutting down...");
            restore_terminal(&mut terminal).expect("Failed to restore terminal");
            kademlia_counter_handle.abort();
        }
    }
}

pub fn init_tracing() {
    let level_filter = EnvFilter::builder().parse_lossy("debug");
    let subscriber = tracing_subscriber::registry()
        .with(TuiTracingSubscriberLayer)
        .with(level_filter);
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
    tui_logger::init_logger(LevelFilter::max()).expect("Failed to initialize tui_logger");
}

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
