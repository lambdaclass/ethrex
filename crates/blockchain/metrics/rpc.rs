use prometheus::{
    CounterVec, Histogram, HistogramVec, register_counter_vec, register_histogram,
    register_histogram_vec,
};
use std::{future::Future, sync::LazyLock};

pub static METRICS_RPC_REQUEST_OUTCOMES: LazyLock<CounterVec> =
    LazyLock::new(initialize_rpc_outcomes_counter);

pub static METRICS_RPC_DURATION: LazyLock<HistogramVec> =
    LazyLock::new(initialize_rpc_duration_histogram);

pub static METRICS_ENGINE_NEWPAYLOAD_QUEUE_WAIT: LazyLock<Histogram> =
    LazyLock::new(initialize_engine_newpayload_queue_wait_histogram);

pub static METRICS_ENGINE_NEWPAYLOAD_PRECHECK: LazyLock<Histogram> =
    LazyLock::new(initialize_engine_newpayload_precheck_histogram);

pub static METRICS_ENGINE_NEWPAYLOAD_WORKER_EXEC: LazyLock<Histogram> =
    LazyLock::new(initialize_engine_newpayload_worker_exec_histogram);

pub static METRICS_ENGINE_NEWPAYLOAD_OUTCOMES: LazyLock<CounterVec> =
    LazyLock::new(initialize_engine_newpayload_outcomes_counter);

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

fn initialize_engine_newpayload_queue_wait_histogram() -> Histogram {
    register_histogram!(
        "engine_newpayload_queue_wait_seconds",
        "Time spent waiting in the block executor queue in seconds",
        prometheus::exponential_buckets(0.0001, 2.0, 18).unwrap()
    )
    .unwrap()
}

fn initialize_engine_newpayload_precheck_histogram() -> Histogram {
    register_histogram!(
        "engine_newpayload_precheck_seconds",
        "Time spent on engine_newPayload prechecks in seconds",
        prometheus::exponential_buckets(0.0001, 2.0, 18).unwrap()
    )
    .unwrap()
}

fn initialize_engine_newpayload_worker_exec_histogram() -> Histogram {
    register_histogram!(
        "engine_newpayload_worker_exec_seconds",
        "Time spent executing the block in the worker in seconds",
        prometheus::exponential_buckets(0.0001, 2.0, 18).unwrap()
    )
    .unwrap()
}

fn initialize_engine_newpayload_outcomes_counter() -> CounterVec {
    register_counter_vec!(
        "engine_newpayload_outcomes_total",
        "Total engine_newPayload outcomes partitioned by outcome and reason",
        &["outcome", "reason"]
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
    // Force initialization of engine_newPayload metrics in the default registry.
    LazyLock::force(&METRICS_ENGINE_NEWPAYLOAD_QUEUE_WAIT);
    LazyLock::force(&METRICS_ENGINE_NEWPAYLOAD_PRECHECK);
    LazyLock::force(&METRICS_ENGINE_NEWPAYLOAD_WORKER_EXEC);
    LazyLock::force(&METRICS_ENGINE_NEWPAYLOAD_OUTCOMES);
}

/// Observe engine_newPayload queue wait time. `seconds` should come from `Duration::as_secs_f64()`.
pub fn observe_engine_newpayload_queue_wait(seconds: f64) {
    METRICS_ENGINE_NEWPAYLOAD_QUEUE_WAIT.observe(seconds);
}

/// Observe engine_newPayload precheck time. `seconds` should come from `Duration::as_secs_f64()`.
pub fn observe_engine_newpayload_precheck(seconds: f64) {
    METRICS_ENGINE_NEWPAYLOAD_PRECHECK.observe(seconds);
}

/// Observe engine_newPayload worker execution time. `seconds` should come from `Duration::as_secs_f64()`.
pub fn observe_engine_newpayload_worker_exec(seconds: f64) {
    METRICS_ENGINE_NEWPAYLOAD_WORKER_EXEC.observe(seconds);
}

/// Record an engine_newPayload outcome.
pub fn record_engine_newpayload_outcome(outcome: &str, reason: &str) {
    METRICS_ENGINE_NEWPAYLOAD_OUTCOMES
        .with_label_values(&[outcome, reason])
        .inc();
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
