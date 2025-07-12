use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

use ethrex_common::H512;
use ethrex_p2p_2::{
    discv4::{server::DiscoveryServer, side_car::DiscoverySideCar},
    types::Node,
};
use k256::{PublicKey, ecdsa::SigningKey, elliptic_curve::sec1::ToEncodedPoint};
use rand::rngs::OsRng;
use tokio::{net::UdpSocket, sync::Mutex};
use tracing::{error, info, level_filters::LevelFilter};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Failed to set global subscriber");

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
    let kademlia_counter_handle = tokio::spawn(async move {
        loop {
            info!(contacts = kademlia.lock().await.len());
            tokio::time::sleep(Duration::from_secs(6)).await;
        }
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("Received Ctrl+C, shutting down...");
            kademlia_counter_handle.abort();
        }
    }
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
