use prometheus::{Encoder, HistogramTimer, HistogramVec, TextEncoder, register_histogram_vec};
use std::{borrow::Cow, future::Future, sync::LazyLock};
use tracing::{
    Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id},
};
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

/// Span extension storing the profiling namespace selected by instrumentation.
struct Namespace(Cow<'static, str>);

#[derive(Default)]
struct NamespaceVisitor {
    namespace: Option<Cow<'static, str>>,
}

impl Visit for NamespaceVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "namespace" {
            self.namespace = Some(Cow::Owned(value.to_owned()));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "namespace" {
            let rendered = format!("{value:?}");
            let cleaned = rendered
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(&rendered);
            self.namespace = Some(Cow::Owned(cleaned.to_owned()));
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

            let namespace = span
                .extensions()
                .get::<Namespace>()
                .map(|ns| ns.0.clone())
                .unwrap_or_else(|| Cow::Borrowed("default"));

            let function_name = span.metadata().name();

            let timer = METRICS_BLOCK_PROCESSING_PROFILE
                .with_label_values(&[namespace.as_ref(), function_name])
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

/// Record the duration of an async operation under the shared function profiling histogram.
pub async fn record_async_duration<Fut, T>(namespace: &str, function_name: &str, future: Fut) -> T
where
    Fut: Future<Output = T>,
{
    let timer = METRICS_BLOCK_PROCESSING_PROFILE
        .with_label_values(&[namespace, function_name])
        .start_timer();

    let output = future.await;
    timer.observe_duration();
    output
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
