use clap::Parser;
use ethrex::{
    cli::CLI,
    initializers::{init_l1, init_tracing},
    utils::{NodeConfigFile, store_node_config_file},
};
use ethrex_p2p::{discv4::peer_table::PeerTableHandle, types::NodeRecord};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
use tracing::info;

async fn server_shutdown(
    data_dir: String,
    cancel_token: &CancellationToken,
    peer_table: PeerTableHandle,
    local_node_record: Arc<Mutex<NodeRecord>>,
) {
    info!("Server shut down started...");
    let node_config_path = PathBuf::from(data_dir + "/node_config.json");
    info!("Storing config at {:?}...", node_config_path);
    cancel_token.cancel();
    let node_config = NodeConfigFile::new(peer_table, local_node_record.lock().await.clone()).await;
    store_node_config_file(node_config, node_config_path).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    info!("Server shutting down!");
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let CLI { opts, command } = CLI::parse();

    if let Some(subcommand) = command {
        return subcommand.run(&opts).await;
    }

    let log_filter_handler = init_tracing(&opts);

    let (data_dir, cancel_token, peer_table, local_node_record) =
        init_l1(opts, Some(log_filter_handler)).await?;

    let mut signal_terminate = signal(SignalKind::terminate())?;

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            server_shutdown(data_dir, &cancel_token, peer_table, local_node_record).await;
        }
        _ = signal_terminate.recv() => {
            server_shutdown(data_dir, &cancel_token, peer_table, local_node_record).await;
        }
    }

    Ok(())
}
