use crate::sequencer::l1_committer::{CallMessage, L1Committer, OutMessage};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::serve::WithGracefulShutdown;
use axum::{Json, Router, http::StatusCode, routing::get};
use serde_json::Value;
use spawned_concurrency::error::GenServerError;
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

pub enum AdminErrorResponse {
    MessageError(String),
    UnexpectedResponse { component: String },
    GenServerError(GenServerError),
}

impl IntoResponse for AdminErrorResponse {
    fn into_response(self) -> axum::response::Response {
        let msg = match self {
            AdminErrorResponse::UnexpectedResponse { component } => {
                format!("Unexpected response from {component}")
            }
            Self::MessageError(err) => err,
            AdminErrorResponse::GenServerError(err) => err.to_string(),
        };

        let body = Json::from(Value::String(msg));

        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

pub async fn start_api(
    http_addr: String,
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
        .route("/committer/start", get(start_committer_default))
        .route("/committer/start/{delay}", get(start_committer))
        .route("/committer/stop", get(stop_committer))
        .layer(cors)
        .with_state(admin.clone());
    let http_listener = TcpListener::bind(http_addr)
        .await
        .map_err(|error| AdminError::Internal(error.to_string()))?;
    let http_server = axum::serve(http_listener, http_router)
        .with_graceful_shutdown(ethrex_rpc::shutdown_signal());

    Ok(http_server)
}

async fn start_committer_default(
    State(admin): State<Admin>,
) -> Result<Json<Value>, AdminErrorResponse> {
    start_committer(State(admin), Path(0)).await
}

async fn start_committer(
    State(mut admin): State<Admin>,
    Path(delay): Path<u64>,
) -> Result<Json<Value>, AdminErrorResponse> {
    match admin.l1_committer.call(CallMessage::Start(delay)).await {
        Ok(ok) => match ok {
            OutMessage::Started => Ok(Json::from(Value::String("ok".into()))),
            OutMessage::Error(err) => Err(AdminErrorResponse::MessageError(err)),
            _ => Err(AdminErrorResponse::UnexpectedResponse {
                component: "l1_committer".into(),
            }),
        },
        Err(err) => Err(AdminErrorResponse::GenServerError(err)),
    }
}

async fn stop_committer(State(mut admin): State<Admin>) -> Result<Json<Value>, AdminErrorResponse> {
    match admin.l1_committer.call(CallMessage::Stop).await {
        Ok(ok) => match ok {
            OutMessage::Stopped => Ok(Json::from(Value::String("ok".into()))),
            OutMessage::Error(err) => Err(AdminErrorResponse::MessageError(err)),
            _ => Err(AdminErrorResponse::UnexpectedResponse {
                component: "l1_committer".into(),
            }),
        },
        Err(err) => Err(AdminErrorResponse::GenServerError(err)),
    }
}
