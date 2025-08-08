use axum::{Router, routing::get};

use crate::profiling::gather_profiling_metrics;

use crate::{MetricsApiError, metrics_blocks::METRICS_BLOCKS, metrics_transactions::METRICS_TX};

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

    ret_string
}
