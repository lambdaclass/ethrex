//! Bridge from tracing spans to profiler frames.
//!
//! `#[timed]` covers synchronous code. Spans cover the `async fn`s on the
//! engine path, where a drop guard placed in the function body would open on
//! one runtime worker and close on another, and the per-block pipeline spans
//! that already feed the Prometheus histograms. This layer opens a profiler
//! frame when one of the allowlisted spans is entered and closes it on exit,
//! once per poll, on the thread that polled.

use flux_profiler::TimerGuard;
use tracing::{Metadata, Subscriber, span::Id};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

/// Span names that produce profiler frames. Everything else, including the
/// per-read VM spans, stays out of the rings.
pub const BRIDGED_SPANS: &[&str] = &[
    // Engine API request path: async handlers.
    "Engine authrpc request",
    "Engine execute payload",
    "Engine validate ancestors",
    "Engine add block",
    "Engine fork choice",
    "Engine get payload",
    // Per-block pipeline phases.
    "Execute Block",
    "Trie update",
    "Trie update (BAL)",
    "Block DB update",
    // Batch state reads; the per-read variants are deliberately absent.
    "Account read batch",
    "Storage read batch",
    "Account codes batch read",
];

/// Filter for the layer: allowlisted spans only, never events.
pub fn bridged_span(meta: &Metadata<'_>) -> bool {
    meta.is_span() && BRIDGED_SPANS.contains(&meta.name())
}

pub struct FluxBridgeLayer;

/// Open frame held in the span's extensions between enter and exit. The guard
/// is never read; dropping it is what closes the frame.
struct Frame {
    _open: TimerGuard,
}

impl<S> Layer<S> for FluxBridgeLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && bridged_span(span.metadata())
        {
            span.extensions_mut().insert(Frame {
                _open: TimerGuard::new(span.metadata().name()),
            });
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().remove::<Frame>();
        }
    }
}
