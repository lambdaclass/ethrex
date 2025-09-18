use clap::Parser;
use ethrex::{
    cli::CLI,
    initializers::{init_l1, init_tracing},
    utils::{NodeConfigFile, store_node_config_file},
};
use ethrex_p2p::{kademlia::Kademlia, types::NodeRecord};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
use tracing::info;

#[cfg(all(feature = "jemalloc", not(target_env = "msvc")))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn log_global_allocator() {
    if cfg!(all(feature = "jemalloc", not(target_env = "msvc"))) {
        tracing::info!("Global allocator: jemalloc (tikv-jemallocator)");
    } else {
        tracing::info!("Global allocator: system (std::alloc::System)");
    }
}

// This could be also enabled via `MALLOC_CONF` env var, but for consistency with the previous jemalloc feature
// usage, we keep it in the code and enable the profiling feature only with the `jemalloc_profiling` feature flag.
#[cfg(all(feature = "jemalloc_profiling", not(target_env = "msvc")))]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "malloc_conf")]
pub static malloc_conf: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";

async fn server_shutdown(
    data_dir: String,
    cancel_token: &CancellationToken,
    peer_table: Kademlia,
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

    log_global_allocator();

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
