use crate::based::{
    block_fetcher::{
        BlockFetcher, CallMessage as BlockFetcherCallMessage, OutMessage as BlockFetcherOutMessage,
    },
    state_updater::{
        CallMessage as StateUpdaterCallMessage, OutMessage as StateUpdaterOutMessage, StateUpdater,
    },
};
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
#[cfg(feature = "metrics")]
use crate::sequencer::metrics::{
    CallMessage as MetricsCallMessage, MetricsGatherer, OutMessage as MetricsOutMessage,
};
use axum::extract::{Path, State};
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::serve::WithGracefulShutdown;
use axum::{Json, Router, http::StatusCode, routing::get};
use serde::Serialize;
use serde_json::{Map, Value};
use spawned_concurrency::error::GenServerError;
use spawned_concurrency::tasks::{GenServer, GenServerHandle};
use thiserror::Error;
use tokio::net::TcpListener;

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
    #[cfg(feature = "metrics")]
    pub metrics_gatherer: Option<GenServerHandle<MetricsGatherer>>,
    pub state_updater: Option<GenServerHandle<StateUpdater>>,
    pub block_fetcher: Option<GenServerHandle<BlockFetcher>>,
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

#[allow(clippy::too_many_arguments)]
pub async fn start_api(
    http_addr: String,
    l1_committer: Option<GenServerHandle<L1Committer>>,
    l1_watcher: Option<GenServerHandle<L1Watcher>>,
    l1_proof_sender: Option<GenServerHandle<L1ProofSender>>,
    block_producer: Option<GenServerHandle<BlockProducer>>,
    #[cfg(feature = "metrics")] metrics_gatherer: Option<GenServerHandle<MetricsGatherer>>,
    state_updater: Option<GenServerHandle<StateUpdater>>,
    block_fetcher: Option<GenServerHandle<BlockFetcher>>,
) -> Result<WithGracefulShutdown<TcpListener, Router, Router, impl Future<Output = ()>>, AdminError>
{
    let admin = Admin {
        l1_committer,
        l1_watcher,
        l1_proof_sender,
        block_producer,
        #[cfg(feature = "metrics")]
        metrics_gatherer,
        state_updater,
        block_fetcher,
    };

    let http_router = Router::new()
        .route("/committer/start", get(start_committer_default))
        .route("/committer/start/{delay}", get(start_committer))
        .route("/committer/stop", get(stop_committer))
        .route("/admin/health", get(admin_health))
        .route("/health", get(health))
        .with_state(admin.clone())
        .fallback(not_found);
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

    response.insert(
        "l1_committer".to_string(),
        genserver_health(admin.l1_committer, CommitterCallMessage::Health, |msg| {
            Some(match msg {
                CommitterOutMessage::Health(h) => h,
                _ => return None,
            })
        })
        .await,
    );

    response.insert(
        "l1_watcher".to_string(),
        genserver_health(admin.l1_watcher, WatcherCallMessage::Health, |msg| {
            Some(match msg {
                WatcherOutMessage::Health(h) => h,
                _ => return None,
            })
        })
        .await,
    );

    response.insert(
        "l1_proof_sender".to_string(),
        genserver_health(
            admin.l1_proof_sender,
            ProofSenderCallMessage::Health,
            |msg| {
                Some(match msg {
                    ProofSenderOutMessage::Health(h) => h,
                    _ => return None,
                })
            },
        )
        .await,
    );

    response.insert(
        "block_producer".to_string(),
        genserver_health(
            admin.block_producer,
            BlockProducerCallMessage::Health,
            |msg| {
                Some(match msg {
                    BlockProducerOutMessage::Health(h) => h,
                    _ => return None,
                })
            },
        )
        .await,
    );

    #[cfg(feature = "metrics")]
    {
        response.insert(
            "metrics_gatherer".to_string(),
            genserver_health(admin.metrics_gatherer, MetricsCallMessage::Health, |msg| {
                Some(match msg {
                    MetricsOutMessage::Health(h) => h,
                    _ => return None,
                })
            })
            .await,
        );
    }

    // Only include based components if they are actually running
    if admin.state_updater.is_some() {
        response.insert(
            "state_updater".to_string(),
            genserver_health(
                admin.state_updater,
                StateUpdaterCallMessage::Health,
                |msg| {
                    Some(match msg {
                        StateUpdaterOutMessage::Health(h) => h,
                        _ => return None,
                    })
                },
            )
            .await,
        );
    }

    if admin.block_fetcher.is_some() {
        response.insert(
            "block_fetcher".to_string(),
            genserver_health(
                admin.block_fetcher,
                BlockFetcherCallMessage::Health,
                |msg| {
                    Some(match msg {
                        BlockFetcherOutMessage::Health(h) => h,
                        _ => return None,
                    })
                },
            )
            .await,
        );
    }

    Ok(Json::from(response))
}

pub async fn genserver_health<S, CallMsg, OutMsg, Health>(
    mut genserver: Option<GenServerHandle<S>>,
    health_msg: CallMsg,
    extract: impl Fn(OutMsg) -> Option<Health>,
) -> Value
where
    S: GenServer<CallMsg = CallMsg, OutMsg = OutMsg>,
    Health: Serialize,
{
    if let Some(handle) = &mut genserver {
        match handle.call(health_msg).await {
            Ok(out) => {
                if let Some(health) = extract(out) {
                    serde_json::to_value(health).unwrap_or_else(|err| {
                        Value::String(format!("Failed to serialize health message {err}"))
                    })
                } else {
                    Value::String("Genserver returned an unexpected message".into())
                }
            }
            Err(err) => Value::String(format!("Genserver health returned an error {err}")),
        }
    } else {
        Value::String(
            "Admin server does not have the genserver handle. Maybe it's not running?".to_string(),
        )
    }
}

pub async fn admin_health(State(_admin): State<Admin>) -> axum::response::Response {
    (StatusCode::OK, "OK".to_string()).into_response()
}

async fn not_found(uri: Uri) -> (StatusCode, String) {
    (
        StatusCode::NOT_FOUND,
        format!("Method {uri} does not exist"),
    )
}
