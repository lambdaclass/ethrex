use clap::Parser;
use ethrex::{
    cli::CLI,
    initializers::{init_l1, init_tracing},
    shutdown::{SHUTDOWN_DEADLINE, shutdown_l1},
    utils::get_client_version,
};
use serde::Deserialize;
use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};
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

// Tune jemalloc for throughput: use a background thread for memory purging instead
// of doing it inline during malloc/free (which adds ~12% of total block execution CPU time
// to jemalloc purge calls, measured via perf profiling on mainnet blocks).
#[cfg(all(
    feature = "jemalloc",
    not(feature = "jemalloc_profiling"),
    not(target_env = "msvc")
))]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "malloc_conf")]
pub static malloc_conf: &[u8] =
    b"background_thread:true,dirty_decay_ms:30000,muzzy_decay_ms:30000\0";

// This could be also enabled via `MALLOC_CONF` env var, but for consistency with the previous jemalloc feature
// usage, we keep it in the code and enable the profiling feature only with the `jemalloc_profiling` feature flag.
#[cfg(all(feature = "jemalloc_profiling", not(target_env = "msvc")))]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "malloc_conf")]
pub static malloc_conf: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19,background_thread:true,dirty_decay_ms:30000,muzzy_decay_ms:30000\0";

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

    let node = init_l1(opts, Some(log_filter_handler)).await?;
    let cancel_token = node.cancel_token.clone();

    let mut signal_terminate = signal(SignalKind::terminate())?;

    log_global_allocator();

    // The node's one shutdown path, whatever starts it. No actor listens for
    // ctrl-c on its own.
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = signal_terminate.recv() => {}
        // A fatal subsystem (e.g. the RPC server) cancels the token to abort the node.
        _ = cancel_token.cancelled() => {}
    }
    let outcome = shutdown_l1(node, SHUTDOWN_DEADLINE).await;

    #[cfg(feature = "cpu_profiling")]
    if let Err(e) = write_cpu_profile(profiler_guard) {
        tracing::error!("Failed to write CPU profile: {e}");
    }

    // A shutdown initiated by a failing subsystem exits non-zero so orchestrators
    // (systemd `Restart=on-failure`, Docker restart policies) can tell a crashed node
    // from a clean signal-triggered stop.
    if let Some(cause) = ethrex::initializers::fatal_shutdown_cause() {
        return Err(eyre::eyre!(
            "node shut down after a fatal subsystem failure: {cause}"
        ));
    }

    // The database is already flushed. Returning would have the runtime wait,
    // with no deadline, on whatever did not stop; `shutdown_l1` has named it.
    if !outcome.complete {
        std::process::exit(1);
    }

    Ok(())
}
