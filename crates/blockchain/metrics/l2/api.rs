use axum::{Router, routing::get};

use crate::{MetricsApiError, api, l2::metrics::METRICS};

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
async fn get_metrics() -> String {
    let mut ret_string = api::get_metrics().await;

    ret_string.push('\n');
    match METRICS.gather_metrics() {
        Ok(string) => ret_string.push_str(&string),
        Err(e) => {
            tracing::error!("Failed to register METRICS_L2: {e}");
            return String::new();
        }
    }

    ret_string
}
