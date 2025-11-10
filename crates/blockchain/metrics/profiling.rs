use prometheus::{Encoder, HistogramTimer, HistogramVec, TextEncoder, register_histogram_vec};
use std::sync::LazyLock;
use tracing::{Subscriber, span::{Attributes, Id}, field::{Field, Visit}};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

use crate::MetricsError;

pub static METRICS_BLOCK_PROCESSING_PROFILE: LazyLock<HistogramVec> =
    LazyLock::new(initialize_histogram_vec);

fn initialize_histogram_vec() -> HistogramVec {
    register_histogram_vec!(
        "function_duration_seconds",
        "Histogram of the run time of the functions in block processing and RPC handling",
        &["namespace", "function_name"]
    )
    .unwrap()
}

// We use this struct to simplify accumulating the time spent doing each task and publishing the metric only when the sync cycle is finished
// We need to do this because things like database reads and writes are spread out throughout the code, so we need to gather multiple measurements to publish
#[derive(Default)]
pub struct FunctionProfilingLayer;

/// Wrapper around [`HistogramTimer`] to avoid conflicts with other layers
struct ProfileTimer(HistogramTimer);

/// Visitor to extract the 'method' field from RPC spans for more granular profiling
#[derive(Default)]
struct MethodVisitor {
    method: Option<String>,
}

impl Visit for MethodVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "method" {
            self.method = Some(value.to_string());
        }
    }
    
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "method" {
            self.method = Some(format!("{:?}", value).trim_matches('"').to_string());
        }
    }
}

impl<S> Layer<S> for FunctionProfilingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        // Extract the 'method' field if present (used by RPC instrumentation)
        let mut visitor = MethodVisitor::default();
        attrs.record(&mut visitor);
        
        if let Some(method_name) = visitor.method {
            // Store the method name in span extensions for later use
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(method_name);
            }
        }
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && span.metadata().target().starts_with("ethrex")
        {
            let target = span.metadata().target();
            
            // Determine namespace based on the module target
            // ethrex_networking::rpc -> "rpc"
            // ethrex_blockchain -> "block_processing"
            // ethrex_storage -> "storage"
            let namespace = if target.contains("::rpc") {
                "rpc"
            } else if target.contains("blockchain") {
                "block_processing"
            } else if target.contains("storage") {
                "storage"
            } else if target.contains("networking") {
                "networking"
            } else {
                "other"
            };

            // Check if we have a stored method name (from RPC middleware)
            // Otherwise fall back to the span name
            let function_name = span
                .extensions()
                .get::<String>()
                .map(|s| s.clone())
                .unwrap_or_else(|| span.metadata().name().to_string());

            let timer = METRICS_BLOCK_PROCESSING_PROFILE
                .with_label_values(&[namespace, &function_name])
                .start_timer();
            // PERF: `extensions_mut` uses a Mutex internally (per span)
            span.extensions_mut().insert(ProfileTimer(timer));
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let timer = ctx
            .span(id)
            // PERF: `extensions_mut` uses a Mutex internally (per span)
            .and_then(|span| span.extensions_mut().remove::<ProfileTimer>());
        if let Some(ProfileTimer(timer)) = timer {
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
