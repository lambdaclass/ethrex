use prometheus::{CounterVec, HistogramVec, register_counter_vec, register_histogram_vec};
use std::{future::Future, sync::LazyLock, time::Instant};

pub static METRICS_RPC_REQUEST_OUTCOMES: LazyLock<CounterVec> =
    LazyLock::new(initialize_rpc_outcomes_counter);

pub static METRICS_RPC_DURATION: LazyLock<HistogramVec> =
    LazyLock::new(initialize_rpc_duration_histogram);

pub static METRICS_RPC_FULL_DURATION: LazyLock<HistogramVec> =
    LazyLock::new(initialize_rpc_full_duration_histogram);

// Metrics defined in this module register into the Prometheus default registry.
// The metrics API exposes them by calling `gather_default_metrics()`.

fn initialize_rpc_outcomes_counter() -> CounterVec {
    register_counter_vec!(
        "rpc_requests_total",
        "Total number of RPC requests partitioned by namespace, method, and outcome",
        &["namespace", "method", "outcome", "error_kind"],
    )
    .unwrap()
}

fn initialize_rpc_duration_histogram() -> HistogramVec {
    register_histogram_vec!(
        "rpc_request_duration_seconds",
        "Histogram of RPC request handling duration partitioned by namespace and method",
        &["namespace", "method"],
    )
    .unwrap()
}

fn initialize_rpc_full_duration_histogram() -> HistogramVec {
    register_histogram_vec!(
        "rpc_request_full_duration_seconds",
        "Histogram of full RPC request duration including JSON parsing, partitioned by namespace and method",
        &["namespace", "method"],
    )
    .unwrap()
}

/// Represents the outcome of an RPC request when recording metrics.
#[derive(Clone)]
pub enum RpcOutcome {
    Success,
    Error(&'static str),
}

impl RpcOutcome {
    fn as_label(&self) -> &'static str {
        match self {
            RpcOutcome::Success => "success",
            RpcOutcome::Error(_) => "error",
        }
    }

    fn error_kind(&self) -> &str {
        match self {
            RpcOutcome::Success => "",
            RpcOutcome::Error(kind) => kind,
        }
    }
}

pub fn record_rpc_outcome(namespace: &str, method: &str, outcome: RpcOutcome) {
    METRICS_RPC_REQUEST_OUTCOMES
        .with_label_values(&[namespace, method, outcome.as_label(), outcome.error_kind()])
        .inc();
}

pub fn initialize_rpc_metrics() {
    METRICS_RPC_REQUEST_OUTCOMES.reset();
    METRICS_RPC_DURATION.reset();
    METRICS_RPC_FULL_DURATION.reset();
}

/// Records the full duration of an RPC request using manual timing.
///
/// Use this when you need to start timing before you know the method name
/// (e.g., before JSON parsing), then record after parsing when the method is known.
pub fn record_full_rpc_duration(namespace: &str, method: &str, start: Instant) {
    let duration = start.elapsed();
    METRICS_RPC_FULL_DURATION
        .with_label_values(&[namespace, method])
        .observe(duration.as_secs_f64());
}

/// Records the duration of an async operation in the RPC request duration histogram.
///
/// This provides a lightweight alternative to the `#[instrument]` attribute.
///
/// # Parameters
/// * `namespace` - Category for the metric (e.g., "rpc", "engine", "block_execution")
/// * `method` - Name identifier for the operation being timed
/// * `future` - The async operation to time
///
pub async fn record_async_duration<Fut, T>(namespace: &str, method: &str, future: Fut) -> T
where
    Fut: Future<Output = T>,
{
    let timer = METRICS_RPC_DURATION
        .with_label_values(&[namespace, method])
        .start_timer();

    let output = future.await;
    timer.observe_duration();
    output
}
