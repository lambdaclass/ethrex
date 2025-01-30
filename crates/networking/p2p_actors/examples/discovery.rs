use commonware_runtime::{Runner, Spawner};
use ethereum_p2p::types::{Endpoint, Node, NodeId};
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
        let bootnode_enode = "enode://ac906289e4b7f12df423d654c5a962b6ebe5b3a74cc9e06292a85221f9a64a6f1cfdd6b714ed6dacef51578f92b34c60ee91e9ede9c7f8fadc4d347326d95e2b@146.190.13.128:30303";
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

        let this_endpoint = Endpoint::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 30303, 0);
        let signer = SecretKey::random(&mut rand::thread_rng());
        let this_node_id = NodeId::from_secret_key(&signer);

        let (mut router, router_mailbox) = ethereum_p2p::discovery::router::actor::Actor::new(
            runtime.clone(),
            ethereum_p2p::discovery::router::actor::Config {
                endpoint: this_endpoint.clone(),
                timeout_duration,
            }
        ).unwrap();

        let kademlia = Arc::new(Mutex::new(BTreeMap::new()));
        kademlia.lock().await.insert(bootnode_socket_address, bootnode);

        let (seeker, seeker_mailbox) = ethereum_p2p::discovery::seeker::Actor::new(
            runtime.clone(),
            router_mailbox.clone(),
            kademlia.clone(),
            ethereum_p2p::discovery::seeker::Config {
                signer,
                node_id: this_node_id,
                timeout_duration,
                seek_interval: Duration::from_secs(1),
            }
        );

        let (server, server_mailbox) = ethereum_p2p::discovery::server::Actor::new(
            router_mailbox.clone(),
            seeker_mailbox.clone(),
            kademlia.clone(),
            ethereum_p2p::discovery::server::Config {
                signer,
                node_id: this_node_id,
            }
        );

        router.register_discovery_server(server_mailbox.clone());

        let (validator, validator_mailbox) = ethereum_p2p::discovery::validator::Actor::new(
            runtime.clone(),
            router_mailbox.clone(),
            kademlia.clone(),
            ethereum_p2p::discovery::validator::Config {
                signer,
                node_id: this_node_id,
                endpoint: this_endpoint,
                timeout_duration,
                revalidation_interval: Duration::from_secs(1),
            }
        );
        
        let mut server_handle = runtime.spawn("server", async move {
            server.run().await
        });
        let mut router_handle = runtime.spawn("router", async move {
            router.run().await
        });
        let mut seeker_handle = runtime.spawn("seeker", async move {
            seeker.run().await
        });
        let mut validator_handle = runtime.spawn("validator", async move {
            validator.run().await
        });


        tokio::select! {
            server_handle_result = &mut server_handle => {
                tracing::error!(?server_handle_result, "Server handle failed, terminating other handles");
                let _ = router_mailbox.terminate().await;
                let _ = seeker_mailbox.terminate().await;
                let _ = validator_mailbox.terminate().await;
                if let Err(err) = timeout(timeout_duration, router_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate router handle");
                }
                if let Err(err) = timeout(timeout_duration, seeker_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate seeker handle");
                }
                if let Err(err) = timeout(timeout_duration, validator_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate validator handle");
                }
            }

            router_handle_result = &mut router_handle => {
                tracing::error!(?router_handle_result, "Router handle failed, shutting down");
                let _ = server_mailbox.terminate().await;
                let _ = seeker_mailbox.terminate().await;
                let _ = validator_mailbox.terminate().await;
                if let Err(err) = timeout(timeout_duration, server_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate server handle");
                }
                if let Err(err) = timeout(timeout_duration, seeker_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate seeker handle");
                }
                if let Err(err) = timeout(timeout_duration, validator_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate validator handle");
                }
            }

            seeker_handle_result = &mut seeker_handle => {
                tracing::error!(?seeker_handle_result, "Seeker handle failed, shutting down");
                let _ = server_mailbox.terminate().await;
                let _ = router_mailbox.terminate().await;
                let _ = validator_mailbox.terminate().await;
                if let Err(err) = timeout(timeout_duration, server_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate server handle");
                }
                if let Err(err) = timeout(timeout_duration, router_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate router handle");
                }
                if let Err(err) = timeout(timeout_duration, validator_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate validator handle");
                }
            }

            validator_handle_result = &mut validator_handle => {
                tracing::error!(?validator_handle_result, "Validator handle failed, shutting down");
                let _ = server_mailbox.terminate().await;
                let _ = router_mailbox.terminate().await;
                let _ = seeker_mailbox.terminate().await;
                if let Err(err) = timeout(timeout_duration, server_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate server handle");
                }
                if let Err(err) = timeout(timeout_duration, router_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate router handle");
                }
                if let Err(err) = timeout(timeout_duration, seeker_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate seeker handle");
                }
            }

            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received CTRL+C, shutting down");
                let _ = server_mailbox.terminate().await;
                let _ = router_mailbox.terminate().await;
                let _ = seeker_mailbox.terminate().await;
                let _ = validator_mailbox.terminate().await;
                if let Err(err) = timeout(timeout_duration, server_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate server handle");
                }
                if let Err(err) = timeout(timeout_duration, router_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate router handle");
                }
                if let Err(err) = timeout(timeout_duration, seeker_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate seeker handle");
                }
                if let Err(err) = timeout(timeout_duration, validator_handle).await {
                    tracing::error!(error = ?err, "Failed to terminate validator handle");
                }

            }
        }

        tracing::info!("Shutdown complete");
    })
}
