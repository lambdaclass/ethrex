use prometheus::{Encoder, GaugeVec, TextEncoder, register_gauge_vec};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tracing::{Subscriber, span::Id};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

use crate::MetricsError;

pub static METRICS_PROFILING: LazyLock<GaugeVec> = LazyLock::new(initialize_profiling_vec);
pub static RUN_ID: LazyLock<u128> = LazyLock::new(get_current_time);
pub static RUN_ID_GAUGE: LazyLock<GaugeVec> = LazyLock::new(initialize_current_run_id);

fn get_current_time() -> u128 {
    let start = SystemTime::now();
    start.duration_since(UNIX_EPOCH).unwrap().as_millis()
}

fn initialize_current_run_id() -> GaugeVec {
    register_gauge_vec!(
        "current_run_id",
        "Keeps track of the timestamp at which the latest import benchmark started and uses it as an id. Used to avoid Grafana mixing data from different runs.",
        &["run_id"]
    )
    .unwrap()
}

fn initialize_profiling_vec() -> GaugeVec {
    register_gauge_vec!(
        "function_duration_seconds",
        "Duration of spans inside add blocks",
        &["function_name", "run_id"]
    )
    .unwrap()
}

// We use this struct to simplify accumulating the time spent doing each task and publishing the metric only when the sync cycle is finished
// We need to do this because things like database reads and writes are spread out throughout the code, so we need to gather multiple measurements to publish

#[derive(Default)]
pub struct FunctionProfilingLayer {
    functions: Mutex<HashMap<Id, Instant>>,
    durations: Mutex<HashMap<String, f64>>, // Temporary per-function storage
}

impl<S> Layer<S> for FunctionProfilingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        let name = ctx.span(id).unwrap().metadata().name();
        if name != "execution_context" {
            self.functions
                .lock()
                .unwrap()
                .insert(id.clone(), Instant::now());
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let name = ctx.span(id).unwrap().metadata().name();
        if name != "execution_context" {
            let start_time = self.functions.lock().unwrap().remove(id).unwrap();
            let duration = start_time.elapsed().as_secs_f64();
            self.durations
                .lock()
                .unwrap()
                .entry(name.to_string())
                .and_modify(|v| *v += duration)
                .or_insert(duration);
        } else {
            self.export_and_clear();
        }
    }
}

impl FunctionProfilingLayer {
    fn export_and_clear(&self) {
        let mut durations = self.durations.lock().unwrap();
        for (function_name, duration) in durations.iter() {
            METRICS_PROFILING
                .with_label_values(&[function_name, &RUN_ID.to_string()])
                .set(*duration);
        }
        durations.clear();
    }
}

pub fn gather_profiling_metrics() -> Result<String, MetricsError> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();

    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

    let res = String::from_utf8(buffer)?;

    Ok(res)
}

pub fn initialize_profiling_metrics() {
    METRICS_PROFILING.reset();
    RUN_ID_GAUGE
        .with_label_values(&[&RUN_ID.to_string()])
        .set(0.0);
}
