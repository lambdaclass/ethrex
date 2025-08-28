use prometheus::{Encoder, HistogramTimer, HistogramVec, TextEncoder, register_histogram_vec};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};
use tracing::{Subscriber, span::Id};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

use crate::MetricsError;

pub static METRICS_BLOCK_PROCESSING_PROFILE: LazyLock<HistogramVec> =
    LazyLock::new(initialize_histogram_vec);

fn initialize_histogram_vec() -> HistogramVec {
    register_histogram_vec!(
        "function_duration_seconds",
        "Histogram of the run time of the functions in block processing",
        &["function_name"]
    )
    .unwrap()
}

// We use this struct to simplify accumulating the time spent doing each task and publishing the metric only when the sync cycle is finished
// We need to do this because things like database reads and writes are spread out throughout the code, so we need to gather multiple measurements to publish
#[derive(Default)]
pub struct FunctionProfilingLayer {
    function_timers: Mutex<HashMap<Id, HistogramTimer>>,
}

impl<S> Layer<S> for FunctionProfilingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            if span.metadata().target().starts_with("ethrex") {
                let name = span.metadata().name();

                let timer = METRICS_BLOCK_PROCESSING_PROFILE
                    .with_label_values(&[name])
                    .start_timer();
                let mut timers = self.function_timers.lock().unwrap();
                timers.insert(id.clone(), timer);
            }
        }
    }

    fn on_exit(&self, id: &Id, _ctx: Context<'_, S>) {
        let mut timers = self.function_timers.lock().unwrap();
        if let Some(timer) = timers.remove(id) {
            timer.observe_duration();
        }
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

pub fn initialize_block_processing_profile() {
    METRICS_BLOCK_PROCESSING_PROFILE.reset();
}
