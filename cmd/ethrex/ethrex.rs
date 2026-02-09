use clap::Parser;
use ethrex::{
    cli::CLI,
    initializers::{init_l1, init_tracing},
    utils::{NodeConfigFile, get_client_version, is_memory_datadir, store_node_config_file},
};
use ethrex_p2p::{peer_table::PeerTable, types::NodeRecord};
use serde::Deserialize;
use std::{path::Path, time::Duration};
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tracing::info;

const LATEST_VERSION_URL: &str = "https://api.github.com/repos/lambdaclass/ethrex/releases/latest";

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
    local_node_record: NodeRecord,
) {
    info!("Server shut down started...");
    cancel_token.cancel();
    if !is_memory_datadir(datadir) {
        let node_config_path = datadir.join("node_config.json");
        info!("Storing config at {:?}...", node_config_path);
        let node_config = NodeConfigFile::new(peer_table, local_node_record).await;
        store_node_config_file(node_config, node_config_path);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    info!("Server shutting down!");
}

/// Fetches the latest release version on github
/// Returns None if there was an error when requesting the latest version
async fn latest_release_version() -> Option<String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }
    let client = reqwest::Client::new();
    let response = client
        .get(LATEST_VERSION_URL)
        .header("User-Agent", "ethrex")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        None
    } else {
        Some(
            response
                .json::<Release>()
                .await
                .ok()?
                .tag_name
                .trim_start_matches("v")
                .to_string(),
        )
    }
}

/// Reads current crate version
fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Returns true if the received latest version is higher than the current ethrex version
fn is_higher_than_current(latest_version: &str) -> bool {
    let current_version_numbers = current_version()
        .split(".")
        .map(|c| c.parse::<u64>().unwrap_or_default());
    let latest_version_numbers = latest_version
        .split(".")
        .map(|c| c.parse::<u64>().unwrap_or_default());
    for (current, latest) in current_version_numbers.zip(latest_version_numbers) {
        match current.cmp(&latest) {
            std::cmp::Ordering::Less => return true,
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => return false,
        };
    }
    false
}

/// Checks if the latest released version is higher than the current version and emits an info log
/// Won't emit a log line if the current version is newer or equal, or if there was a problem reading either version
async fn check_version_update() {
    if let Some(latest_version) = latest_release_version().await
        && is_higher_than_current(&latest_version)
    {
        info!(
            "There is a newer ethrex version available, current version: {} vs latest version: {latest_version}",
            current_version()
        );
    }
}

/// Checks if there is a newer ethrex verison available every hour
async fn periodically_check_version_update() {
    let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
    loop {
        interval.tick().await;
        check_version_update().await;
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let CLI { opts, command } = CLI::parse();

    rayon::ThreadPoolBuilder::default()
        .thread_name(|i| format!("rayon-worker-{i}"))
        .build_global()
        .expect("failed to build rayon threadpool");

    if let Some(subcommand) = command {
        return subcommand.run(&opts).await;
    }

    let (log_filter_handler, _guard) = init_tracing(&opts);

    info!("ethrex version: {}", get_client_version());
    tokio::spawn(periodically_check_version_update());

    #[cfg(feature = "experimental-discv5")]
    tracing::warn!("Experimental Discovery V5 protocol enabled");

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
