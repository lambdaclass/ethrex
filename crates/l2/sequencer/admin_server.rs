use crate::sequencer::block_producer::{
    BlockProducer, CallMessage as BlockProducerCallMessage, OutMessage as BlockProducerOutMessage,
};
use crate::sequencer::l1_committer::{
    CallMessage as CommitterCallMessage, L1Committer, OutMessage as CommitterOutMessage,
};
use crate::sequencer::l1_proof_sender::{
    CallMessage as ProofSenderCallMessage, L1ProofSender, OutMessage as ProofSenderOutMessage,
};
use crate::sequencer::l1_watcher::{
    CallMessage as WatcherCallMessage, L1Watcher, OutMessage as WatcherOutMessage,
};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::serve::WithGracefulShutdown;
use axum::{Json, Router, http::StatusCode, routing::get};
use serde_json::{Map, Value};
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
    pub l1_committer: Option<GenServerHandle<L1Committer>>,
    pub l1_watcher: Option<GenServerHandle<L1Watcher>>,
    pub l1_proof_sender: Option<GenServerHandle<L1ProofSender>>,
    pub block_producer: Option<GenServerHandle<BlockProducer>>,
}

pub enum AdminErrorResponse {
    MessageError(String),
    UnexpectedResponse { component: String },
    GenServerError(GenServerError),
    NoHandle,
}

impl IntoResponse for AdminErrorResponse {
    fn into_response(self) -> axum::response::Response {
        let msg = match self {
            AdminErrorResponse::UnexpectedResponse { component } => {
                format!("Unexpected response from {component}")
            }
            Self::MessageError(err) => err,
            AdminErrorResponse::GenServerError(err) => err.to_string(),
            AdminErrorResponse::NoHandle => {
                "Admin server does not have the genserver handle. Maybe its not running?".into()
            }
        };

        let body = Json::from(Value::String(msg));

        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

pub async fn start_api(
    http_addr: String,
    l1_committer: Option<GenServerHandle<L1Committer>>,
    l1_watcher: Option<GenServerHandle<L1Watcher>>,
    l1_proof_sender: Option<GenServerHandle<L1ProofSender>>,
    block_producer: Option<GenServerHandle<BlockProducer>>,
) -> Result<WithGracefulShutdown<TcpListener, Router, Router, impl Future<Output = ()>>, AdminError>
{
    let admin = Admin {
        l1_committer,
        l1_watcher,
        l1_proof_sender,
        block_producer,
    };

    // All request headers allowed.
    // All methods allowed.
    // All origins allowed.
    // All headers exposed.
    let cors = CorsLayer::permissive();

    let http_router = Router::new()
        .route("/committer/start", get(start_committer_default))
        .route("/committer/start/{delay}", get(start_committer))
        .route("/committer/stop", get(stop_committer))
        .route("/health", get(health))
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
    State(admin): State<Admin>,
    Path(delay): Path<u64>,
) -> Result<Json<Value>, AdminErrorResponse> {
    let Some(mut l1_committer) = admin.l1_committer else {
        return Err(AdminErrorResponse::NoHandle);
    };

    match l1_committer.call(CommitterCallMessage::Start(delay)).await {
        Ok(ok) => match ok {
            CommitterOutMessage::Started => Ok(Json::from(Value::String("ok".into()))),
            CommitterOutMessage::Error(err) => Err(AdminErrorResponse::MessageError(err)),
            _ => Err(AdminErrorResponse::UnexpectedResponse {
                component: "l1_committer".into(),
            }),
        },
        Err(err) => Err(AdminErrorResponse::GenServerError(err)),
    }
}

async fn stop_committer(State(admin): State<Admin>) -> Result<Json<Value>, AdminErrorResponse> {
    let Some(mut l1_committer) = admin.l1_committer else {
        return Err(AdminErrorResponse::NoHandle);
    };

    match l1_committer.call(CommitterCallMessage::Stop).await {
        Ok(ok) => match ok {
            CommitterOutMessage::Stopped => Ok(Json::from(Value::String("ok".into()))),
            CommitterOutMessage::Error(err) => Err(AdminErrorResponse::MessageError(err)),
            _ => Err(AdminErrorResponse::UnexpectedResponse {
                component: "l1_committer".into(),
            }),
        },
        Err(err) => Err(AdminErrorResponse::GenServerError(err)),
    }
}

async fn health(
    State(admin): State<Admin>,
) -> Result<Json<Map<String, Value>>, AdminErrorResponse> {
    let mut response = serde_json::Map::new();

    let l1_committer_response = if let Some(mut l1_committer) = admin.l1_committer {
        let l1_committer_health = l1_committer.call(CommitterCallMessage::Health).await;

        match l1_committer_health {
            Ok(CommitterOutMessage::Health(health)) => {
                serde_json::to_value(health).unwrap_or_else(|err| {
                    Value::String(format!("Failed to serialize health message {err}"))
                })
            }
            Ok(_) => Value::String("Genserver returned an unexpected message".into()),
            Err(err) => Value::String(format!("Genserver health returned an error {err}")),
        }
    } else {
        Value::String(
            "Admin server does not have the genserver handle. Maybe its not running?".to_string(),
        )
    };
    response.insert("l1_committer".to_string(), l1_committer_response);

    let l1_watcher_response = if let Some(mut l1_watcher) = admin.l1_watcher {
        let l1_watcher_health = l1_watcher.call(WatcherCallMessage::Health).await;

        match l1_watcher_health {
            Ok(WatcherOutMessage::Health(health)) => {
                serde_json::to_value(health).unwrap_or_else(|err| {
                    Value::String(format!("Failed to serialize health message {err}"))
                })
            }
            Ok(_) => Value::String("Genserver returned an unexpected message".into()),
            Err(err) => Value::String(format!("Genserver health returned an error {err}")),
        }
    } else {
        Value::String(
            "Admin server does not have the genserver handle. Maybe its not running?".to_string(),
        )
    };
    response.insert("l1_watcher".to_string(), l1_watcher_response);

    let l1_proof_sender_response = if let Some(mut l1_proof_sender) = admin.l1_proof_sender {
        let l1_proof_sender_health = l1_proof_sender.call(ProofSenderCallMessage::Health).await;

        match l1_proof_sender_health {
            Ok(ProofSenderOutMessage::Health(health)) => serde_json::to_value(health)
                .unwrap_or_else(|err| {
                    Value::String(format!("Failed to serialize health message {err}"))
                }),
            Ok(_) => Value::String("Genserver returned an unexpected message".into()),
            Err(err) => Value::String(format!("Genserver health returned an error {err}")),
        }
    } else {
        Value::String(
            "Admin server does not have the genserver handle. Maybe its not running?".to_string(),
        )
    };
    response.insert("l1_proof_sender".to_string(), l1_proof_sender_response);

    let block_producer_response = if let Some(mut block_producer) = admin.block_producer {
        let block_producer_health = block_producer.call(BlockProducerCallMessage::Health).await;

        match block_producer_health {
            Ok(BlockProducerOutMessage::Health(health)) => serde_json::to_value(health)
                .unwrap_or_else(|err| {
                    Value::String(format!("Failed to serialize health message {err}"))
                }),
            Ok(_) => Value::String("Genserver returned an unexpected message".into()),
            Err(err) => Value::String(format!("Genserver health returned an error {err}")),
        }
    } else {
        Value::String(
            "Admin server does not have the genserver handle. Maybe its not running?".to_string(),
        )
    };
    response.insert("block_producer".to_string(), block_producer_response);

    Ok(Json::from(response))
}
