use metrics::histogram;
use prometheus::{HistogramTimer, HistogramVec, register_histogram_vec};
use std::sync::LazyLock;
use std::time::Instant;
use tracing::{
    Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id},
};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

pub static METRICS_BLOCK_PROCESSING_PROFILE: LazyLock<HistogramVec> =
    LazyLock::new(initialize_histogram_vec);

// Metrics defined in this module register into the Prometheus default registry.
// The metrics API exposes them by calling `gather_default_metrics()`.

fn initialize_histogram_vec() -> HistogramVec {
    register_histogram_vec!(
        "old_function_duration_seconds",
        "[DEPRECATED] Histogram of the run time of the functions in block processing and RPC handling",
        &["namespace", "function_name"]
    )
    .unwrap()
}

// We use this struct to simplify accumulating the time spent doing each task and publishing the metric only when the sync cycle is finished
// We need to do this because things like database reads and writes are spread out throughout the code, so we need to gather multiple measurements to publish
#[derive(Default)]
pub struct FunctionProfilingLayer;

/// Wrapper around [`HistogramTimer`] to avoid conflicts with other layers
struct ProfileTimer {
    prometheus_timer: HistogramTimer,
    start_instant: Instant,
    namespace: String,
    function_name: String,
}

/// Span extension storing the profiling namespace selected by instrumentation. This needs to
/// be a String instead of a &'static str because using the span macros we could recieve dynamically
/// generated names and can't rely on only string literals.
struct Namespace(String);

#[derive(Default)]
struct NamespaceVisitor {
    namespace: Option<String>,
}

impl Visit for NamespaceVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "namespace" {
            self.namespace = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "namespace" {
            let rendered = format!("{value:?}");
            let cleaned = rendered
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(&rendered);
            self.namespace = Some(cleaned.to_owned());
        }
    }
}

impl<S> Layer<S> for FunctionProfilingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut visitor = NamespaceVisitor::default();
        attrs.record(&mut visitor);

        if let (Some(span), Some(namespace)) = (ctx.span(id), visitor.namespace) {
            span.extensions_mut().insert(Namespace(namespace));
        }
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && span.metadata().target().starts_with("ethrex")
        {
            let target = span.metadata().target();

            // Skip RPC modules; RPC timing is recorded explicitly at the call sites.
            if target.contains("::rpc") {
                return;
            }
            let timer = {
                let extensions = span.extensions();
                let namespace = extensions
                    .get::<Namespace>()
                    .map(|ns| ns.0.as_str())
                    .unwrap_or("default");

                let function_name = span.metadata().name();

                let prometheus_timer = METRICS_BLOCK_PROCESSING_PROFILE
                    .with_label_values(&[namespace, function_name])
                    .start_timer();

                ProfileTimer {
                    prometheus_timer,
                    start_instant: Instant::now(),
                    namespace: namespace.to_string(),
                    function_name: function_name.to_string(),
                }
            };

            // PERF: `extensions_mut` uses a Mutex internally (per span)
            span.extensions_mut().insert(timer);
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let timer = ctx
            .span(id)
            // PERF: `extensions_mut` uses a Mutex internally (per span)
            .and_then(|span| span.extensions_mut().remove::<ProfileTimer>());
        if let Some(profile_timer) = timer {
            profile_timer.prometheus_timer.observe_duration();

            // Record to new metrics system for summary quantiles (p50, p90, p95, p99, p999)
            let duration_secs = profile_timer.start_instant.elapsed().as_secs_f64();
            histogram!(
                "function_duration_seconds",
                "namespace" => profile_timer.namespace,
                "function_name" => profile_timer.function_name
            )
            .record(duration_secs);
        }
    }
}

pub fn initialize_block_processing_profile() {
    METRICS_BLOCK_PROCESSING_PROFILE.reset();
}
