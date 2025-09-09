use crate::sequencer::l1_committer::{CallMessage, L1Committer, OutMessage};
use axum::extract::{Path, State};
use axum::serve::WithGracefulShutdown;
use axum::{Json, Router, http::StatusCode, routing::post};
use serde_json::Value;
use spawned_concurrency::tasks::GenServerHandle;
use thiserror::Error;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

#[derive(Debug, Error)]
pub enum AdminError {
    #[error("Internal Error: {0}")]
    Internal(String),
}

#[derive(Clone)]
pub struct Admin {
    pub l1_committer: GenServerHandle<L1Committer>,
}

pub async fn start_api(
    l1_committer: GenServerHandle<L1Committer>,
) -> Result<WithGracefulShutdown<TcpListener, Router, Router, impl Future<Output = ()>>, AdminError>
{
    let admin = Admin { l1_committer };

    // All request headers allowed.
    // All methods allowed.
    // All origins allowed.
    // All headers exposed.
    let cors = CorsLayer::permissive();

    let http_router = Router::new()
        .route("/committer/start", post(start_committer_default))
        .route("/committer/start/{delay}", post(start_committer))
        .route("/committer/stop", post(stop_committer))
        .layer(cors)
        .with_state(admin.clone());
    let http_listener = TcpListener::bind("0.0.0.0:5555")
        .await
        .map_err(|error| AdminError::Internal(error.to_string()))?;
    let http_server = axum::serve(http_listener, http_router)
        .with_graceful_shutdown(ethrex_rpc::shutdown_signal());

    Ok(http_server)
}

async fn start_committer_default(State(admin): State<Admin>) -> Result<Json<Value>, StatusCode> {
    start_committer(State(admin), Path(0)).await
}

async fn start_committer(
    State(mut admin): State<Admin>,
    Path(delay): Path<u64>,
) -> Result<Json<Value>, StatusCode> {
    dbg!(delay);
    let response = match admin.l1_committer.call(CallMessage::Start(delay)).await {
        Ok(ok) => match ok {
            OutMessage::Started => "ok".to_string(),
            OutMessage::Error(err) => err,
            _ => "unexpected response from l1 committer".to_string(),
        },
        Err(err) => err.to_string(),
    };
    Ok(axum::Json::from(serde_json::Value::String(response)))
}

async fn stop_committer(State(mut admin): State<Admin>) -> Result<Json<Value>, StatusCode> {
    let response = match admin.l1_committer.call(CallMessage::Stop).await {
        Ok(ok) => match ok {
            OutMessage::Stopped => "ok".to_string(),
            OutMessage::Error(err) => err,
            _ => "unexpected response from l1 committer".to_string(),
        },
        Err(err) => err.to_string(),
    };

    Ok(axum::Json::from(serde_json::Value::String(response)))
}
