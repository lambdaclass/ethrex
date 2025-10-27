use clap::Parser;
use ethrex::{
    cli::CLI,
    initializers::{init_l1, init_tracing},
    utils::{NodeConfigFile, get_client_version, store_node_config_file},
};
use ethrex_p2p::{discv4::peer_table::PeerTable, types::NodeRecord};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};
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
    datadir: &Path,
    cancel_token: &CancellationToken,
    peer_table: PeerTable,
    local_node_record: Arc<Mutex<NodeRecord>>,
) {
    info!("Server shut down started...");
    let node_config_path = datadir.join("node_config.json");
    info!("Storing config at {:?}...", node_config_path);
    cancel_token.cancel();
    let node_config = NodeConfigFile::new(peer_table, local_node_record.lock().await.clone()).await;
    store_node_config_file(node_config, node_config_path).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    info!("Server shutting down!");
}

async fn ethrex_main() -> eyre::Result<()> {
    let CLI { opts, command } = CLI::parse();

    if let Some(subcommand) = command {
        return subcommand.run(&opts).await;
    }

    let log_filter_handler = init_tracing(&opts);

    info!("ethrex version: {}", get_client_version());

    let (datadir, cancel_token, peer_table, local_node_record) =
        init_l1(opts, Some(log_filter_handler)).await?;

    let mut signal_terminate = signal(SignalKind::terminate())?;

    log_global_allocator();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            server_shutdown(&datadir, &cancel_token, peer_table, local_node_record).await;
        }
        _ = signal_terminate.recv() => {
            server_shutdown(&datadir, &cancel_token, peer_table, local_node_record).await;
        }
    }

    Ok(())
}

pub fn main() -> eyre::Result<()> {
    let mut core = AtomicUsize::new(0);
    let cores = core_affinity::get_core_ids().unwrap_or_default();
    // Reserve core 0 and 1 for OS, 2 for block execution.
    let count = cores.len().saturating_sub(3);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(count)
        .on_thread_start(|| {
            let core_offset = core.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if let Some(core_id) = cores.get(3 + core_offset.rem_euclid(count)) {
                core_affinity::set_for_current(*core_id);
            }
        })
        .build()
        .unwrap()
        .block_on(ethrex_main())
}
