#[cfg(feature = "l2")]
use crate::rlpx::l2::l2_connection::P2PBasedContext;
#[cfg(not(feature = "l2"))]
#[derive(Clone, Debug)]
pub struct P2PBasedContext;
use crate::{
    discv4::{
        peer_table::{PeerData, PeerTable},
        server::{DiscoveryServer, DiscoveryServerError},
    },
    metrics::METRICS,
    rlpx::{
        connection::server::{PeerConnBroadcastSender, PeerConnection},
        message::Message,
        p2p::SUPPORTED_SNAP_CAPABILITIES,
    },
    tx_broadcaster::{TxBroadcaster, TxBroadcasterError},
    types::Node,
};
use ethrex_blockchain::Blockchain;
use ethrex_storage::Store;
use secp256k1::SecretKey;
use spawned_concurrency::tasks::GenServerHandle;
use std::{
    io,
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
    time::{Duration, SystemTime},
};
use tokio::net::{TcpListener, TcpSocket, UdpSocket};
use tokio_util::task::TaskTracker;
use tracing::{error, info};

pub const MAX_MESSAGES_TO_BROADCAST: usize = 100000;

#[derive(Clone, Debug)]
pub struct P2PContext {
    pub tracker: TaskTracker,
    pub signer: SecretKey,
    pub table: PeerTable,
    pub storage: Store,
    pub blockchain: Arc<Blockchain>,
    pub(crate) broadcast: PeerConnBroadcastSender,
    pub local_node: Node,
    pub client_version: String,
    #[cfg(feature = "l2")]
    pub based_context: Option<P2PBasedContext>,
    pub tx_broadcaster: GenServerHandle<TxBroadcaster>,
}

impl P2PContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        local_node: Node,
        tracker: TaskTracker,
        signer: SecretKey,
        peer_table: PeerTable,
        storage: Store,
        blockchain: Arc<Blockchain>,
        client_version: String,
        based_context: Option<P2PBasedContext>,
        tx_broadcasting_time_interval: u64,
    ) -> Result<Self, NetworkError> {
        let (channel_broadcast_send_end, _) = tokio::sync::broadcast::channel::<(
            tokio::task::Id,
            Arc<Message>,
        )>(MAX_MESSAGES_TO_BROADCAST);

        let tx_broadcaster = TxBroadcaster::spawn(
            peer_table.clone(),
            blockchain.clone(),
            tx_broadcasting_time_interval,
        )
        .await
        .inspect_err(|e| {
            error!("Failed to start Tx Broadcaster: {e}");
        })?;

        #[cfg(not(feature = "l2"))]
        let _ = &based_context;

        Ok(P2PContext {
            local_node,
            tracker,
            signer,
            table: peer_table,
            storage,
            blockchain,
            broadcast: channel_broadcast_send_end,
            client_version,
            #[cfg(feature = "l2")]
            based_context,
            tx_broadcaster,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Failed to start discovery server: {0}")]
    DiscoveryServerError(#[from] DiscoveryServerError),
    #[error("Failed to start Tx Broadcaster: {0}")]
    TxBroadcasterError(#[from] TxBroadcasterError),
}

pub async fn start_network(context: P2PContext, bootnodes: Vec<Node>) -> Result<(), NetworkError> {
    let udp_socket = Arc::new(
        UdpSocket::bind(context.local_node.udp_addr())
            .await
            .expect("Failed to bind udp socket"),
    );

    DiscoveryServer::spawn(
        context.storage.clone(),
        context.local_node.clone(),
        context.signer,
        udp_socket.clone(),
        context.table.clone(),
        bootnodes,
    )
    .await
    .inspect_err(|e| {
        error!("Failed to start discovery server: {e}");
    })?;

    context.tracker.spawn(serve_p2p_requests(context.clone()));

    Ok(())
}

pub(crate) async fn serve_p2p_requests(context: P2PContext) {
    let tcp_addr = context.local_node.tcp_addr();
    let listener = match listener(tcp_addr) {
        Ok(result) => result,
        Err(e) => {
            error!("Error opening tcp socket at {tcp_addr}: {e}. Stopping p2p server");
            return;
        }
    };
    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(result) => result,
            Err(e) => {
                error!("Error receiving data from tcp socket {tcp_addr}: {e}. Stopping p2p server");
                return;
            }
        };

        if tcp_addr == peer_addr {
            // Ignore connections from self
            continue;
        }

        let _ = PeerConnection::spawn_as_receiver(context.clone(), peer_addr, stream).await;
    }
}

fn listener(tcp_addr: SocketAddr) -> Result<TcpListener, io::Error> {
    let tcp_socket = match tcp_addr {
        SocketAddr::V4(_) => TcpSocket::new_v4(),
        SocketAddr::V6(_) => TcpSocket::new_v6(),
    }?;
    tcp_socket.set_reuseport(true).ok();
    tcp_socket.set_reuseaddr(true).ok();
    tcp_socket.bind(tcp_addr)?;

    tcp_socket.listen(50)
}

pub async fn periodically_show_peer_stats(blockchain: Arc<Blockchain>, mut peer_table: PeerTable) {
    periodically_show_peer_stats_during_syncing(blockchain, &mut peer_table).await;
    periodically_show_peer_stats_after_sync(&mut peer_table).await;
}

pub async fn periodically_show_peer_stats_during_syncing(
    blockchain: Arc<Blockchain>,
    peer_table: &mut PeerTable,
) {
    let start = std::time::Instant::now();
    loop {
        {
            if blockchain.is_synced() {
                return;
            }
            let metrics_enabled = *METRICS.enabled.lock().await;
            // Show the metrics only when these are enabled
            if !metrics_enabled {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }

            // Common metrics
            let elapsed = format_duration(start.elapsed());
            let peer_number = peer_table.peer_count().await.unwrap_or(0);
            let current_step = METRICS.current_step.get();
            let current_header_hash = *METRICS.sync_head_hash.lock().await;

            // Headers metrics
            let headers_to_download = METRICS.sync_head_block.load(Ordering::Relaxed);
            // We may download more than expected headers due to duplicates
            // We just clamp it to the max to avoid showing the user confusing data
            let headers_downloaded =
                u64::min(METRICS.downloaded_headers.get(), headers_to_download);
            let headers_remaining = headers_to_download.saturating_sub(headers_downloaded);
            let headers_download_progress = if headers_to_download == 0 {
                "0%".to_string()
            } else {
                format!(
                    "{:.2}%",
                    (headers_downloaded as f64 / headers_to_download as f64) * 100.0
                )
            };

            // Account leaves metrics
            let account_leaves_downloaded =
                METRICS.downloaded_account_tries.load(Ordering::Relaxed);
            let account_leaves_inserted = METRICS.account_tries_inserted.load(Ordering::Relaxed);
            let account_leaves_inserted_percentage = if account_leaves_downloaded != 0 {
                (account_leaves_inserted as f64 / account_leaves_downloaded as f64) * 100.0
            } else {
                0.0
            };
            let account_leaves_pending =
                account_leaves_downloaded.saturating_sub(account_leaves_inserted);
            let account_leaves_time = format_duration({
                let end_time = METRICS
                    .account_tries_download_end_time
                    .lock()
                    .await
                    .unwrap_or(SystemTime::now());

                METRICS
                    .account_tries_download_start_time
                    .lock()
                    .await
                    .map(|start_time| {
                        end_time
                            .duration_since(start_time)
                            .unwrap_or(Duration::from_secs(0))
                    })
                    .unwrap_or(Duration::from_secs(0))
            });
            let account_leaves_inserted_time = format_duration({
                let end_time = METRICS
                    .account_tries_insert_end_time
                    .lock()
                    .await
                    .unwrap_or(SystemTime::now());

                METRICS
                    .account_tries_insert_start_time
                    .lock()
                    .await
                    .map(|start_time| {
                        end_time
                            .duration_since(start_time)
                            .unwrap_or(Duration::from_secs(0))
                    })
                    .unwrap_or(Duration::from_secs(0))
            });

            // Storage leaves metrics
            let storage_leaves_downloaded = METRICS.storage_leaves_downloaded.get();
            let storage_leaves_inserted = METRICS.storage_leaves_inserted.get();
            let storage_leaves_inserted_percentage = if storage_leaves_downloaded != 0 {
                storage_leaves_inserted as f64 / storage_leaves_downloaded as f64 * 100.0
            } else {
                0.0
            };
            // We round up because of the accounts whose slots get downloaded and then not used
            let storage_leaves_inserted_percentage =
                (storage_leaves_inserted_percentage * 10.0).round() / 10.0;
            let storage_leaves_time = format_duration({
                let end_time = METRICS
                    .storage_tries_download_end_time
                    .lock()
                    .await
                    .unwrap_or(SystemTime::now());

                METRICS
                    .storage_tries_download_start_time
                    .lock()
                    .await
                    .map(|start_time| {
                        end_time
                            .duration_since(start_time)
                            .unwrap_or(Duration::from_secs(0))
                    })
                    .unwrap_or(Duration::from_secs(0))
            });
            let storage_leaves_inserted_time = format_duration({
                let end_time = METRICS
                    .storage_tries_insert_end_time
                    .lock()
                    .await
                    .unwrap_or(SystemTime::now());

                METRICS
                    .storage_tries_insert_start_time
                    .lock()
                    .await
                    .map(|start_time| {
                        end_time
                            .duration_since(start_time)
                            .unwrap_or(Duration::from_secs(0))
                    })
                    .unwrap_or(Duration::from_secs(0))
            });

            // Healing stuff
            let heal_time = format_duration({
                let end_time = METRICS
                    .heal_end_time
                    .lock()
                    .await
                    .unwrap_or(SystemTime::now());

                METRICS
                    .heal_start_time
                    .lock()
                    .await
                    .map(|start_time| {
                        end_time
                            .duration_since(start_time)
                            .expect("Failed to get storage tries download time")
                    })
                    .unwrap_or(Duration::from_secs(0))
            });
            let healed_accounts = METRICS
                .global_state_trie_leafs_healed
                .load(Ordering::Relaxed);
            let healed_storages = METRICS
                .global_storage_tries_leafs_healed
                .load(Ordering::Relaxed);
            let heal_current_throttle =
                if METRICS.healing_empty_try_recv.load(Ordering::Relaxed) == 0 {
                    "Database"
                } else {
                    "Peers"
                };

            // Bytecode metrics
            let bytecodes_download_time = format_duration({
                let end_time = METRICS
                    .bytecode_download_end_time
                    .lock()
                    .await
                    .unwrap_or(SystemTime::now());

                METRICS
                    .bytecode_download_start_time
                    .lock()
                    .await
                    .map(|start_time| {
                        end_time
                            .duration_since(start_time)
                            .expect("Failed to get storage tries download time")
                    })
                    .unwrap_or(Duration::from_secs(0))
            });

            let bytecodes_downloaded = METRICS.downloaded_bytecodes.load(Ordering::Relaxed);

            info!(
                r#"
P2P Snap Sync | elapsed {elapsed} | peers {peer_number} | step {current_step} | head {current_header_hash:x}
  headers : {headers_downloaded}/{headers_to_download} ({headers_download_progress}), remaining {headers_remaining}
  accounts: downloaded {account_leaves_downloaded} @ {account_leaves_time} | inserted {account_leaves_inserted} ({account_leaves_inserted_percentage:.1}%) in {account_leaves_inserted_time} | pending {account_leaves_pending}
  storage : downloaded {storage_leaves_downloaded} @ {storage_leaves_time} | inserted {storage_leaves_inserted} ({storage_leaves_inserted_percentage:.1}%) in {storage_leaves_inserted_time}
  healing : accounts {healed_accounts}, storages {healed_storages}, elapsed {heal_time}, throttle {heal_current_throttle}
  bytecodes: downloaded {bytecodes_downloaded} in {bytecodes_download_time}"#
            );
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// Shows the amount of connected peers, active peers, and peers suitable for snap sync on a set interval
pub async fn periodically_show_peer_stats_after_sync(peer_table: &mut PeerTable) {
    const INTERVAL_DURATION: tokio::time::Duration = tokio::time::Duration::from_secs(60);
    let mut interval = tokio::time::interval(INTERVAL_DURATION);
    loop {
        // clone peers to keep the lock short
        let peers: Vec<PeerData> = peer_table.get_peers_data().await.unwrap_or(Vec::new());
        let active_peers = peers
            .iter()
            .filter(|peer| -> bool { peer.connection.as_ref().is_some() })
            .count();
        let snap_active_peers = peers
            .iter()
            .filter(|peer| -> bool {
                peer.connection.as_ref().is_some()
                    && SUPPORTED_SNAP_CAPABILITIES
                        .iter()
                        .any(|cap| peer.supported_capabilities.contains(cap))
            })
            .count();
        info!("Snap Peers: {snap_active_peers} / Total Peers: {active_peers}");
        interval.tick().await;
    }
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let milliseconds = total_seconds / 1000;

    format!("{hours:02}h {minutes:02}m {seconds:02}s {milliseconds:02}ms")
}
