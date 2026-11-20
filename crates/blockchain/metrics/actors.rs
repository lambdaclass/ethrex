//! Actor-runtime metrics from the `spawned` framework.
//!
//! `spawned-concurrency` keeps its own process-wide counters, gauges and
//! histograms (actors alive, mailbox depth, handler and request outcomes, queue
//! wait and latency). They are not in a Prometheus registry; this module
//! configures them and renders them for the metrics API under the `spawned_*`
//! namespace.

use spawned_concurrency::metrics::{self, Config};

/// When metrics are enabled, time one in every this many messages and requests.
///
/// Counters and gauges always count every operation. In spawned's own
/// no-op-handler microbenchmark, timing every message cut throughput by 18-33%,
/// while 1-in-16 sampling (11-13%) cost about the same as counters alone
/// (7-14%). The overhead is a fixed cost per message, so it shrinks relative to
/// handlers that do real work.
/// Histogram `_count`/`_sum` describe the sampled observations only, so they
/// are about 1/16 of the matching counters.
pub const TIMING_SAMPLE_EVERY: u64 = 16;

/// Sets spawned's timing policy. Call before any actor sends a message: the
/// policy is fixed on first use.
///
/// The counters are compiled in with the `spawned` feature and can't be turned
/// off at runtime, so when metrics are disabled this only turns timing off.
pub fn configure(metrics_enabled: bool) {
    let timing_sample_every = if metrics_enabled {
        TIMING_SAMPLE_EVERY
    } else {
        0
    };
    if let Err(config) = metrics::configure(Config {
        timing_sample_every,
    }) {
        tracing::warn!(
            requested = ?config,
            "spawned metrics were already initialized; keeping the existing timing policy"
        );
    }
}

/// Renders spawned's metrics in Prometheus text format.
pub fn gather_metrics() -> String {
    metrics::render_prometheus()
}
