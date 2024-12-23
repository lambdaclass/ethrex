use axum::{routing::get, Router};

use crate::{metrics_transactions::METRICS_TX, MetricsApiError};

pub async fn start_prometheus_metrics_api(port: String) -> Result<(), MetricsApiError> {
    let app = Router::new()
        .route("/metrics", get(get_metrics))
        .route("/health", get("Service Up"));

    // Start the axum app
    let listener = tokio::net::TcpListener::bind(&format!("0.0.0.0:{port}")).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn get_metrics() -> String {
    METRICS_TX.gather_metrics()
}
