//! Running an ethrex node for a chain defined outside this repository.
//!
//! Support for a chain is a crate that describes it with a [`Chain`] and builds its
//! own binary with [`run`]. The `ethrex` binary is the same thing for Ethereum:
//! `run(ETHEREUM)`.
//!
//! A chain can add:
//! - named networks for `--network` ([`Chain::networks`]),
//! - its own command line flags (the type parameter of [`Chain`]),
//! - RPC namespaces and mempool rules ([`NodeComponents`]).
//!
//! Everything a chain leaves out behaves as on Ethereum.

use std::{path::Path, sync::Arc, time::Duration};

use clap::{Args, CommandFactory, FromArgMatches, builder::Resettable};
use ethrex_blockchain::MempoolPolicy;
use ethrex_common::types::Genesis;
use ethrex_config::networks::{Network, NetworkPreset};
use ethrex_p2p::{peer_table::PeerTable, types::SharedLocalNode};
use ethrex_rpc::RpcExtension;
use ethrex_storage::Store;
use eyre::WrapErr;
use serde::Deserialize;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    cli::{CLI, Options},
    initializers::{fatal_shutdown_cause, init_node, init_tracing},
    utils::{NodeConfigFile, get_client_version, is_memory_datadir, store_node_config_file},
};

const LATEST_VERSION_URL: &str = "https://api.github.com/repos/lambdaclass/ethrex/releases/latest";

/// A chain the node can run on, compiled into the binary.
///
/// `A` is the chain's own command line flags, added to the node's: a
/// `clap::Args` struct, or [`NoArgs`] for none. Prefix the flag names with the
/// chain's name (`--<chain>.<flag>`) so they cannot collide with the node's.
pub struct Chain<A> {
    /// Name of the binary, shown in `--help` and in startup errors.
    pub name: &'static str,
    /// Networks of this chain, added to `--network`. When there is at least one,
    /// the first is the default network of the binary.
    pub networks: &'static [NetworkPreset],
    /// What the node runs with on this chain, in addition to its Ethereum behavior.
    ///
    /// Called once per start, after the command line is parsed and the genesis is
    /// loaded, and before the database is opened, with the chain's flags and the
    /// genesis of the network selected with `--network`. An error aborts the start.
    pub components: fn(&A, &Genesis) -> eyre::Result<NodeComponents>,
}

/// The parts of the node a chain can add to. `NodeComponents::default()` adds
/// nothing, which is a node that behaves as on Ethereum.
///
/// Build it by filling in the fields of the default value. New fields may be added
/// in later versions.
#[derive(Default)]
#[non_exhaustive]
pub struct NodeComponents {
    /// RPC namespaces served in addition to the built-in ones.
    pub rpc: Vec<Arc<dyn RpcExtension>>,
    /// Extra admission rules for the mempool.
    pub mempool_policy: Option<Arc<dyn MempoolPolicy>>,
}

/// The flags of a chain without flags of its own.
#[derive(Args, Debug, Clone, Default)]
pub struct NoArgs {}

/// The built-in chain: Ethereum, which adds nothing.
pub const ETHEREUM: Chain<NoArgs> = Chain {
    name: "ethrex",
    networks: &[],
    components: |_, _| Ok(NodeComponents::default()),
};

/// Runs the node for `chain` with the arguments of the current process, until it is
/// shut down. This is the whole `main` of a chain's binary.
///
/// The binary declares its global allocator with [`crate::global_allocator!`], as
/// `cmd/ethrex/ethrex.rs` does.
pub fn run<A: Args>(chain: Chain<A>) -> eyre::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_node(chain))
}

/// The command line of `chain`'s binary: the node's own, plus the chain's flags and
/// networks.
pub fn command<A: Args>(chain: &Chain<A>) -> clap::Command {
    let node = CLI::command();
    // `augment_args` replaces the command's description with the doc comment of the
    // flags struct; keep the node's.
    let keep = |text: Option<&clap::builder::StyledStr>| {
        text.cloned().map_or(Resettable::Reset, Resettable::Value)
    };
    let about = keep(node.get_about());
    let long_about = keep(node.get_long_about());
    let mut command = A::augment_args(node)
        .name(chain.name)
        .about(about)
        .long_about(long_about);
    let presets = chain.networks;
    if let Some(default) = presets.first() {
        let names = presets
            .iter()
            .map(|preset| preset.name)
            .collect::<Vec<_>>()
            .join(", ");
        command = command.mut_arg("network", |arg| {
            arg.long_help(format!(
                "Alternatively, the name of a known network can be provided instead to use its preset genesis file and include its preset bootnodes. The networks currently supported include {names}. If not specified, defaults to {}.",
                default.name
            ))
        });
    }
    command
}

/// Parses `argv` for `chain`'s binary and resolves `--network` against the chain's
/// networks.
pub fn parse_args<A, T>(
    chain: &Chain<A>,
    argv: impl IntoIterator<Item = T>,
) -> Result<(CLI, A), clap::Error>
where
    A: Args,
    T: Into<std::ffi::OsString> + Clone,
{
    let mut command = command(chain);
    let matches = command.try_get_matches_from_mut(argv)?;
    let mut cli = CLI::from_arg_matches(&matches)?;
    let args = A::from_arg_matches(&matches)?;
    resolve_network(&mut cli.opts, chain.networks)
        .map_err(|err| command.error(clap::error::ErrorKind::ValueValidation, err))?;
    Ok((cli, args))
}

/// Replaces a `--network` value that names a preset with that preset, and selects
/// the first preset when no network was given. A malformed preset is an error here,
/// before anything reads it.
fn resolve_network(opts: &mut Options, presets: &[NetworkPreset]) -> Result<(), String> {
    for (index, preset) in presets.iter().enumerate() {
        preset.validate_name()?;
        if presets[..index]
            .iter()
            .any(|other| other.name == preset.name)
        {
            return Err(format!(
                "network preset {:?} is defined more than once",
                preset.name
            ));
        }
    }
    let selected = match &opts.network {
        // The CLI parser reads a name it does not know as a genesis file path.
        Some(Network::GenesisPath(path)) => presets
            .iter()
            .find(|preset| Path::new(preset.name) == path.as_path()),
        // Dev mode runs its own local network.
        None if !opts.dev => presets.first(),
        _ => None,
    };
    if let Some(preset) = selected {
        preset.validate()?;
        opts.network = Some(Network::Preset(*preset));
    }
    Ok(())
}

async fn run_node<A: Args>(chain: Chain<A>) -> eyre::Result<()> {
    let (CLI { opts, command }, args) =
        parse_args(&chain, std::env::args_os()).unwrap_or_else(|err| err.exit());

    rayon::ThreadPoolBuilder::default()
        .thread_name(|i| format!("rayon-worker-{i}"))
        .build_global()
        .expect("failed to build rayon threadpool");

    if let Some(subcommand) = command {
        return subcommand.run(&opts).await;
    }

    let (log_filter_handler, _guard) = init_tracing(&opts);

    #[cfg(feature = "cpu_profiling")]
    let profiler_guard = {
        let guard = pprof::ProfilerGuardBuilder::default()
            .frequency(1000)
            .build()
            .expect("failed to build CPU profiler");
        info!("CPU profiling enabled (1000 Hz), will write profile.pb at shutdown");
        guard
    };

    info!("ethrex version: {}", get_client_version());
    tokio::spawn(periodically_check_version_update());

    let (datadir, cancel_token, peer_table, shared_local_node, store) = init_node(
        opts,
        |genesis| {
            (chain.components)(&args, genesis)
                .wrap_err_with(|| format!("{}: failed to build the node's components", chain.name))
        },
        Some(log_filter_handler),
    )
    .await?;

    let mut signal_terminate = signal(SignalKind::terminate())?;

    log_global_allocator();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            server_shutdown(&datadir, &cancel_token, peer_table, shared_local_node, &store).await;
        }
        _ = signal_terminate.recv() => {
            server_shutdown(&datadir, &cancel_token, peer_table, shared_local_node, &store).await;
        }
        // A fatal subsystem (e.g. the RPC server) cancels the token to abort the node.
        _ = cancel_token.cancelled() => {
            server_shutdown(&datadir, &cancel_token, peer_table, shared_local_node, &store).await;
        }
    }

    #[cfg(feature = "cpu_profiling")]
    if let Err(e) = write_cpu_profile(profiler_guard) {
        tracing::error!("Failed to write CPU profile: {e}");
    }

    // A shutdown initiated by a failing subsystem exits non-zero so orchestrators
    // (systemd `Restart=on-failure`, Docker restart policies) can tell a crashed node
    // from a clean signal-triggered stop.
    if let Some(cause) = fatal_shutdown_cause() {
        return Err(eyre::eyre!(
            "node shut down after a fatal subsystem failure: {cause}"
        ));
    }

    Ok(())
}

fn log_global_allocator() {
    if cfg!(all(feature = "jemalloc", not(target_env = "msvc"))) {
        tracing::info!("Global allocator: jemalloc (tikv-jemallocator)");
    } else {
        tracing::info!("Global allocator: system (std::alloc::System)");
    }
}

async fn server_shutdown(
    datadir: &Path,
    cancel_token: &CancellationToken,
    peer_table: PeerTable,
    shared_local_node: SharedLocalNode,
    store: &Store,
) {
    info!("Server shut down started...");
    // Stop feeding new blocks before draining, so the persist queue can't grow.
    cancel_token.cancel();
    // Drain the persist worker, force-flush the block-data buffer, and fsync the
    // DB. Without this an abrupt exit (e.g. `docker restart -t 0`) loses the
    // buffered block-data tail and leaves the DB needing WAL recovery on next
    // start. In-memory trie diff-layers are intentionally left uncommitted and
    // re-executed on the next start (see `Store::shutdown`).
    info!("Flushing database to disk...");
    if let Err(err) = store.shutdown().await {
        error!("Failed to flush database on shutdown: {err}");
    }
    if !is_memory_datadir(datadir) {
        let node_config_path = datadir.join("node_config.json");
        info!("Storing config at {:?}...", node_config_path);
        // Clone the current (possibly updated) record out and drop the guard.
        let record = {
            let guard = shared_local_node
                .read()
                .expect("shared_local_node poisoned");
            guard.record.clone()
        };
        let node_config = NodeConfigFile::new(peer_table, record).await;
        store_node_config_file(node_config, node_config_path);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    info!("Server shutting down!");
}

/// Builds the CPU profile report and writes it to `profile.pb` in the current directory.
#[cfg(feature = "cpu_profiling")]
fn write_cpu_profile(guard: pprof::ProfilerGuard<'_>) -> eyre::Result<()> {
    use pprof::protos::Message;

    let report = guard.report().build()?;
    let profile = report.pprof()?;
    let mut content = Vec::new();
    profile.encode(&mut content)?;
    std::fs::write("profile.pb", &content)?;
    info!("CPU profile written to profile.pb");
    Ok(())
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
