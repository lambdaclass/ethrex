//! The L1 node's single shutdown path.
//!
//! Before this, three p2p actors (the peer table, the RLPx initiator and
//! discovery) each stopped on their own ctrl-c listener, racing the main path,
//! which cancelled the node's token, asked the (possibly already stopped) peer
//! table for its peers, saved an empty list if it had stopped, and slept 1 s. A
//! sync cycle that saw the stopped peer table before the token was cancelled
//! treated `ActorStopped` as fatal and exited with code 2, skipping the database
//! flush (#7491).
//!
//! Now one [`ActorSystem`] owns those actors and nothing stops them but
//! [`shutdown_l1`], which runs the steps in dependency order under one
//! deadline.

use crate::utils::{NodeConfigFile, is_memory_datadir, store_node_config_file};
use ethrex_p2p::{peer_table::PeerTable, types::SharedLocalNode};
use ethrex_storage::Store;
use spawned_concurrency::ActorId;
use spawned_concurrency::system::{ActorSystem, ShutdownReport};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

/// How long a graceful shutdown may take, from the signal to the last owned
/// actor stopped.
///
/// Under Docker's default 10 s `docker stop` grace period, so a shutdown that
/// overruns reports which actors held it instead of being SIGKILLed with no
/// record of why.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(8);

/// The least time the owned actors get to stop, even when flushing the
/// database used up the rest of the deadline.
const MIN_ACTOR_STOP_BUDGET: Duration = Duration::from_secs(1);

/// What the L1 node's shutdown needs. Returned by
/// [`init_l1`](crate::initializers::init_l1).
pub struct L1Node {
    pub datadir: PathBuf,
    /// Cancelled first on shutdown: stops the sync cycle, the RPC servers, the
    /// dev block producer and the other tasks on the node's tracker.
    pub cancel_token: CancellationToken,
    pub peer_table: PeerTable,
    pub shared_local_node: SharedLocalNode,
    pub store: Store,
    /// Owns the node's long-lived actors. Keep it alive while the node runs:
    /// dropping it stops them.
    pub system: ActorSystem,
    /// Names for the actors `system` owns, for the shutdown report, which
    /// identifies actors by id only.
    pub actor_names: Vec<(ActorId, &'static str)>,
}

/// How a shutdown went.
#[derive(Debug)]
pub struct ShutdownOutcome {
    /// Every owned actor stopped within the deadline.
    pub complete: bool,
    pub elapsed: Duration,
}

/// Shuts the node down within `deadline`. The order is the point:
///
/// 1. Cancel the node's token, so producers (sync, RPC, block producer) stop
///    first. Any of them that fails from here on sees a node that is shutting
///    down, which the sync cycle treats as a reason to stop, not to exit.
/// 2. Snapshot the connected peers while the peer table is still running.
/// 3. Flush the database.
/// 4. Save the node config (record and peers).
/// 5. Stop the owned actors, in reverse admission order, with what is left of
///    the deadline.
pub async fn shutdown_l1(node: L1Node, deadline: Duration) -> ShutdownOutcome {
    let started = Instant::now();
    let L1Node {
        datadir,
        cancel_token,
        peer_table,
        shared_local_node,
        store,
        system,
        actor_names,
    } = node;

    info!(
        deadline_secs = deadline.as_secs(),
        "Server shut down started..."
    );
    cancel_token.cancel();

    let node_config = if is_memory_datadir(&datadir) {
        None
    } else {
        // Clone the current (possibly updated) record out and drop the guard.
        let record = {
            let guard = shared_local_node
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.record.clone()
        };
        Some(NodeConfigFile::new(peer_table, record, &datadir).await)
    };

    // Drain the persist worker, force-flush the block-data buffer, and fsync the
    // DB. Without this an abrupt exit (e.g. `docker restart -t 0`) loses the
    // buffered block-data tail and leaves the DB needing WAL recovery on next
    // start. In-memory trie diff-layers are intentionally left uncommitted and
    // re-executed on the next start (see `Store::shutdown`).
    info!("Flushing database to disk...");
    if let Err(err) = store.shutdown().await {
        error!("Failed to flush database on shutdown: {err}");
    }

    if let Some(node_config) = node_config {
        let node_config_path = datadir.join("node_config.json");
        info!("Storing config at {:?}...", node_config_path);
        store_node_config_file(node_config, node_config_path);
    }

    // The flush cannot be cut short, so a slow one still leaves the actors
    // `MIN_ACTOR_STOP_BUDGET` rather than nothing.
    let actor_budget = deadline
        .saturating_sub(started.elapsed())
        .max(MIN_ACTOR_STOP_BUDGET);
    let report = system.shutdown(actor_budget).await;
    let elapsed = started.elapsed();
    if report.complete {
        info!(
            elapsed_ms = elapsed.as_millis() as u64,
            "Server shutting down!"
        );
    } else {
        log_incomplete_shutdown(&report, &actor_names, elapsed, deadline);
    }
    ShutdownOutcome {
        complete: report.complete,
        elapsed,
    }
}

/// Names what a shutdown that missed its deadline left running.
fn log_incomplete_shutdown(
    report: &ShutdownReport,
    actor_names: &[(ActorId, &'static str)],
    elapsed: Duration,
    deadline: Duration,
) {
    let name = |id: &ActorId| {
        actor_names
            .iter()
            .find(|(known, _)| known == id)
            .map_or_else(|| id.to_string(), |(_, name)| (*name).to_string())
    };
    let unfinished_actors: Vec<String> = report.unfinished_actors.iter().map(name).collect();
    let exits: Vec<String> = report
        .exits
        .iter()
        .map(|exit| format!("{}: {}", name(&exit.id), exit.reason))
        .collect();
    error!(
        elapsed_ms = elapsed.as_millis() as u64,
        deadline_secs = deadline.as_secs(),
        unfinished_groups = ?report.unfinished_groups,
        ?unfinished_actors,
        ?exits,
        supervisor_failures = ?report.supervisor_failures,
        group_failures = ?report.group_failures,
        "Shutdown deadline exceeded; exiting with actors still running"
    );
}
