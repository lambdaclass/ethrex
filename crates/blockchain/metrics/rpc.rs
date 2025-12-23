use prometheus::{CounterVec, HistogramOpts, HistogramVec, register_counter_vec};
use std::{future::Future, sync::LazyLock};

pub static METRICS_RPC_REQUEST_OUTCOMES: LazyLock<CounterVec> =
    LazyLock::new(initialize_rpc_outcomes_counter);

pub static METRICS_RPC_DURATION: LazyLock<HistogramVec> =
    LazyLock::new(initialize_rpc_duration_histogram);

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
    let opts = HistogramOpts::new(
        "rpc_request_duration_seconds",
        "Histogram of RPC request handling duration partitioned by namespace and method",
    )
    .buckets({
        let mut buckets = vec![0.0];
        // Fine buckets from 0.01s to 0.7s (step 0.01s, 70 buckets: 0.01, 0.02, ..., 0.7)
        buckets.extend(prometheus::linear_buckets(0.01, 0.01, 70).unwrap());
        // Coarser buckets from 0.8s to 3.0s (step 0.1s, 23 buckets: 0.8, 0.9, ..., 3.0)
        buckets.extend(prometheus::linear_buckets(0.8, 0.1, 23).unwrap());
        // Higher buckets for rare outliers
        buckets.extend(vec![5.0, 10.0]);
        buckets
    });
    let histogram = HistogramVec::new(opts, &["namespace", "method"]).unwrap();
    prometheus::register(Box::new(histogram.clone())).unwrap();
    histogram
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
