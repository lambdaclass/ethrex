use axum::{Router, routing::get};
use prometheus::{Encoder, TextEncoder};

use crate::profiling::gather_profiling_metrics;

use crate::{
    MetricsApiError, metrics_blocks::METRICS_BLOCKS, metrics_process::METRICS_PROCESS,
    metrics_transactions::METRICS_TX,
};

pub async fn start_prometheus_metrics_api(
    address: String,
    port: String,
) -> Result<(), MetricsApiError> {
    let app = Router::new()
        .route("/metrics", get(get_metrics))
        .route("/health", get("Service Up"));

    // Start the axum app
    let listener = tokio::net::TcpListener::bind(&format!("{address}:{port}")).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[allow(unused_mut)]
pub(crate) async fn get_metrics() -> String {
    let mut ret_string = match METRICS_TX.gather_metrics() {
        Ok(string) => string,
        Err(_) => {
            tracing::error!("Failed to register METRICS_TX");
            String::new()
        }
    };

    ret_string.push('\n');
    match gather_profiling_metrics() {
        Ok(string) => ret_string.push_str(&string),
        Err(_) => {
            tracing::error!("Failed to register METRICS_PROFILING");
            return String::new();
        }
    };

    ret_string.push('\n');
    match METRICS_BLOCKS.gather_metrics() {
        Ok(string) => ret_string.push_str(&string),
        Err(_) => {
            tracing::error!("Failed to register METRICS_BLOCKS");
            return String::new();
        }
    }

    ret_string.push('\n');
    match METRICS_PROCESS.gather_metrics() {
        Ok(s) => ret_string.push_str(&s),
        Err(_) => tracing::error!("Failed to register METRICS_PROCESS"),
    };

    ret_string.push('\n');
    let encoder = TextEncoder::new();
    let metric_families = prometheus::default_registry().gather();
    let mut buffer = Vec::new();
    match encoder.encode(&metric_families, &mut buffer) {
        Ok(()) => match String::from_utf8(buffer) {
            Ok(s) => ret_string.push_str(&s),
            Err(err) => tracing::error!(err = %err, "Failed to encode default registry metrics"),
        },
        Err(err) => tracing::error!(err = %err, "Failed to gather default registry metrics"),
    }

    ret_string
}
