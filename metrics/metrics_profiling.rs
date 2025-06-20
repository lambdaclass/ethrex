use prometheus::{register_gauge_vec, Encoder, GaugeVec, TextEncoder};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Instant,
};
use tokio::fs::metadata;
use tracing::{span::Id, Event, Subscriber};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

use crate::MetricsError;

pub static METRICS_PROFILING: LazyLock<GaugeVec> = LazyLock::new(initialize_profiling_vec);

// We use this struct to simplify accumulating the time spent doing each task and publishing the metric only when the sync cycle is finished
// We need to do this because things like database reads and writes are spread out throughout the code, so we need to gather multiple measurements to publish

#[derive(Default)]
pub struct FunctionProfilingLayer {
    functions: Mutex<HashMap<Id, Instant>>,
    durations: Mutex<HashMap<String, f64>>, // Temporary per-function storage
}

fn initialize_profiling_vec() -> GaugeVec {
    register_gauge_vec!(
        "function_duration_seconds",
        "Duration of spans inside add blocks",
        &["function"]
    )
    .unwrap()
}

impl<S> Layer<S> for FunctionProfilingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &Id, _ctx: Context<'_, S>) {
        self.functions
            .lock()
            .unwrap()
            .insert(id.clone(), Instant::now());
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let start_time = self.functions.lock().unwrap().remove(id).unwrap();
        let duration = start_time.elapsed().as_secs_f64();
        let span = ctx.span(id).unwrap();
        let name = span.metadata().name().split("::").last().unwrap();
        self.durations
            .lock()
            .unwrap()
            .entry(name.to_string())
            .and_modify(|v| *v += duration)
            .or_insert(duration);
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().name() != "export_metrics" {
            self.export_and_clear();
        }
    }
}

impl FunctionProfilingLayer {
    fn export_and_clear(&self) {
        let mut durations = self.durations.lock().unwrap();
        for (function_name, duration) in durations.iter() {
            METRICS_PROFILING
                .with_label_values(&[function_name])
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
