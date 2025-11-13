use clap::Parser;
use ethrex::{
    cli::CLI,
    initializers::{init_l1, init_tracing},
    utils::{NodeConfigFile, get_client_version, store_node_config_file},
};
use ethrex_p2p::{discv4::peer_table::PeerTable, types::NodeRecord};
use semver::Version;
use serde::Deserialize;
use std::{path::Path, time::Duration};
use tokio::signal::unix::{SignalKind, signal};
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
    local_node_record: NodeRecord,
) {
    info!("Server shut down started...");
    let node_config_path = datadir.join("node_config.json");
    info!("Storing config at {:?}...", node_config_path);
    cancel_token.cancel();
    let node_config = NodeConfigFile::new(peer_table, local_node_record).await;
    store_node_config_file(node_config, node_config_path).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    info!("Server shutting down!");
}

/// Fetches the latest release version on github
/// Returns None if there was an error when requesting the latest version
async fn latest_release_version() -> Option<Version> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }
    let client = reqwest::Client::new();
    let response = client
        .get("https://api.github.com/repos/lambdaclass/ethrex/releases/latest")
        .header("User-Agent", "ethrex")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        None
    } else {
        Version::parse(
            response
                .json::<Release>()
                .await
                .ok()?
                .tag_name
                .trim_start_matches("v"),
        )
        .ok()
    }
}

/// Reads current crate version
fn current_version() -> Option<Version> {
    Version::parse(env!("CARGO_PKG_VERSION")).ok()
}

/// Checks if the latest released version is higher than the current version and emits an info log
/// Won't emit a log line if the current version is newer or equal, or if there was a problem reading either version
async fn check_version_update() {
    if let (Some(current_version), Some(latest_version)) =
        (current_version(), latest_release_version().await)
        && current_version < latest_version
    {
        info!(
            "There is a newer ethrex version available, current version: {current_version} vs latest version: {latest_version}"
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

    if let Some(subcommand) = command {
        return subcommand.run(&opts).await;
    }

    let log_filter_handler = init_tracing(&opts);

    info!("ethrex version: {}", get_client_version());
    tokio::spawn(periodically_check_version_update());

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
