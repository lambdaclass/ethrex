use axum::{Router, routing::get};

use crate::{
    MetricsApiError, blocks::METRICS_BLOCKS, gather_default_metrics, p2p::METRICS_P2P,
    process::METRICS_PROCESS, transactions::METRICS_TX,
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
    match gather_default_metrics() {
        Ok(string) => ret_string.push_str(&string),
        Err(_) => {
            tracing::error!("Failed to gather default Prometheus metrics");
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
    match METRICS_P2P.gather_metrics() {
        Ok(s) => ret_string.push_str(&s),
        Err(_) => tracing::error!("Failed to register METRICS_P2P"),
    };

    ret_string
}
