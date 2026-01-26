use axum::{Router, routing::get};

use crate::{
    MetricsApiError, blocks::METRICS_BLOCKS, gather_default_metrics, node::METRICS_NODE,
    p2p::METRICS_P2P, process::METRICS_PROCESS, recorder, transactions::METRICS_TX,
};

pub async fn start_prometheus_metrics_api(
    address: String,
    port: String,
) -> Result<(), MetricsApiError> {
    // Initialize the metrics recorder (for p50/p99 summary metrics)
    if let Err(e) = recorder::initialize_metrics_recorder() {
        tracing::warn!("Failed to initialize metrics recorder: {e}");
    }

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
            tracing::error!("Failed to gather METRICS_TX");
            String::new()
        }
    };

    ret_string.push('\n');
    match gather_default_metrics() {
        Ok(string) => ret_string.push_str(&string),
        Err(_) => tracing::error!("Failed to gather default Prometheus metrics"),
    };

    ret_string.push('\n');
    match METRICS_BLOCKS.gather_metrics() {
        Ok(string) => ret_string.push_str(&string),
        Err(_) => tracing::error!("Failed to gather METRICS_BLOCKS"),
    };

    ret_string.push('\n');
    match METRICS_PROCESS.gather_metrics() {
        Ok(s) => ret_string.push_str(&s),
        Err(_) => tracing::error!("Failed to gather METRICS_PROCESS"),
    };

    ret_string.push('\n');
    match METRICS_P2P.gather_metrics() {
        Ok(s) => ret_string.push_str(&s),
        Err(_) => tracing::error!("Failed to gather METRICS_P2P"),
    };

    ret_string.push('\n');
    if let Some(node_metrics) = METRICS_NODE.get() {
        match node_metrics.gather_metrics() {
            Ok(s) => ret_string.push_str(&s),
            Err(_) => tracing::error!("Failed to gather METRICS_NODE"),
        };
    }

    // Include metrics from the new metrics-exporter-prometheus recorder
    // These include summary metrics with p50, p90, p95, p99, p999 quantiles
    ret_string.push('\n');
    ret_string.push_str(&recorder::render_metrics());

    ret_string
}
