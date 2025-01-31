use commonware_runtime::{Runner, Spawner};
use ethereum_p2p::types::{Endpoint, Node, NodeId, PeerData};
use libsecp256k1::{PublicKeyFormat, SecretKey};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::Arc, time::Duration,
};
use tokio::{sync::Mutex, time::timeout};
use tracing_subscriber::{filter::Directive, EnvFilter, FmtSubscriber};

fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(Directive::from_str("info").unwrap())
                .from_env_lossy(),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let timeout_duration = Duration::from_secs(10);

    let (executor, runtime) = commonware_runtime::tokio::Executor::default();

    executor.start(async move {
        let bootnode_enode = "enode://bdcf92f566bc180a10355b7c0bc25cd049ede640ab6ac850ab56ef51441523dd98b59e78e594db1d82212ca3b16e5e83bef16a4a1b26db78713fc8ca99409c8e@127.0.0.1:30303";
        let bootnode_socket_address: SocketAddr = bootnode_enode[137..].parse().unwrap();
        let bootnode = Node::new(
            bootnode_socket_address.ip(),
            bootnode_socket_address.port(),
            bootnode_socket_address.port(),
            NodeId::parse_slice(
                &hex::decode(&bootnode_enode[8..136]).unwrap(),
                Some(PublicKeyFormat::Raw),
            )
            .unwrap(),
        );

        let kademlia = Arc::new(Mutex::new(BTreeMap::new()));
        kademlia.lock().await.insert(bootnode_socket_address, PeerData::new_known(bootnode.endpoint));

        let signer = SecretKey::random(&mut rand::thread_rng());
        
        let (discovery, discovery_mailbox) = ethereum_p2p::discovery::Actor::new(
            runtime.clone(),
            kademlia, 
            ethereum_p2p::discovery::Config {
                endpoint: Endpoint::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 30304, 0),
                signer,
                node_id: NodeId::from_secret_key(&signer),
                seek_interval: Duration::from_secs(15),
                revalidation_interval: Duration::from_secs(10),
                timeout_duration,
            }
        );
        let mut discovery_handle = runtime.spawn("discovery", async move {
            discovery.run().await
        });

        tokio::select! {
            discovery_handle_result = &mut discovery_handle => {
                if let Err(err) = discovery_handle_result {
                    tracing::error!(error = ?err, "Discovery handle failed");
                }
            }

            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received CTRL+C, shutting down");
                let _ = discovery_mailbox.terminate().await;
                if let Err(err) = timeout(timeout_duration, discovery_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate discovery handle");
                }
            }
        }

        tracing::info!("Shutdown complete");
    })
}
