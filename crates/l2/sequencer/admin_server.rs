use crate::sequencer::block_producer::BlockProducer;
use crate::sequencer::l1_committer::L1Committer;
use crate::sequencer::l1_proof_sender::L1ProofSender;
use crate::sequencer::l1_watcher::L1Watcher;
#[cfg(feature = "metrics")]
use crate::sequencer::metrics::MetricsGatherer;
use crate::sequencer::state_updater::StateUpdater;
use axum::extract::{Path, State};
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::serve::WithGracefulShutdown;
use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, post},
};
use serde::Serialize;
use serde_json::{Map, Value};
use spawned_concurrency::error::ActorError;
use spawned_concurrency::message::Message;
use spawned_concurrency::tasks::ActorRef;
use thiserror::Error;
use tokio::net::TcpListener;

use crate::sequencer::block_producer::block_producer_protocol;
use crate::sequencer::l1_committer::l1_committer_protocol;
use crate::sequencer::l1_proof_sender::l1_proof_sender_protocol;
use crate::sequencer::l1_watcher::l1_watcher_protocol;
#[cfg(feature = "metrics")]
use crate::sequencer::metrics::metrics_gatherer_protocol;
use crate::sequencer::state_updater::state_updater_protocol;

#[derive(Debug, Error)]
pub enum AdminError {
    #[error("Internal Error: {0}")]
    Internal(String),
}

#[derive(Clone)]
pub struct Admin {
    pub l1_committer: Option<ActorRef<L1Committer>>,
    pub l1_watcher: Option<ActorRef<L1Watcher>>,
    pub l1_proof_sender: Option<ActorRef<L1ProofSender>>,
    pub block_producer: Option<ActorRef<BlockProducer>>,
    pub state_updater: Option<ActorRef<StateUpdater>>,
    #[cfg(feature = "metrics")]
    pub metrics_gatherer: Option<ActorRef<MetricsGatherer>>,
}

pub enum AdminErrorResponse {
    MessageError(String),
    ActorError(ActorError),
    NoHandle,
}

impl IntoResponse for AdminErrorResponse {
    fn into_response(self) -> axum::response::Response {
        let msg = match self {
            Self::MessageError(err) => err,
            AdminErrorResponse::ActorError(err) => err.to_string(),
            AdminErrorResponse::NoHandle => {
                "Admin server does not have the actor handle. Maybe its not running?".into()
            }
        };

        let body = Json::from(Value::String(msg));

        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

pub async fn start_api(
    http_addr: String,
    l1_committer: Option<ActorRef<L1Committer>>,
    l1_watcher: Option<ActorRef<L1Watcher>>,
    l1_proof_sender: Option<ActorRef<L1ProofSender>>,
    block_producer: Option<ActorRef<BlockProducer>>,
    state_updater: Option<ActorRef<StateUpdater>>,
    #[cfg(feature = "metrics")] metrics_gatherer: Option<ActorRef<MetricsGatherer>>,
) -> Result<WithGracefulShutdown<TcpListener, Router, Router, impl Future<Output = ()>>, AdminError>
{
    let admin = Admin {
        l1_committer,
        l1_watcher,
        l1_proof_sender,
        block_producer,
        state_updater,
        #[cfg(feature = "metrics")]
        metrics_gatherer,
    };

    let http_router = Router::new()
        .route("/committer/start", get(start_committer_default))
        .route("/committer/start/{delay}", get(start_committer))
        .route("/committer/stop", get(stop_committer))
        .route("/admin/health", get(admin_health))
        .route("/health", get(health))
        .route(
            "/state-updater/stop-at/{block_number}",
            post(set_sequencer_stop_at),
        )
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
    let Some(l1_committer) = admin.l1_committer else {
        return Err(AdminErrorResponse::NoHandle);
    };

    match l1_committer
        .request(l1_committer_protocol::StartCommitter { delay })
        .await
    {
        Ok(Ok(())) => Ok(Json::from(Value::String("ok".into()))),
        Ok(Err(err)) => Err(AdminErrorResponse::MessageError(err)),
        Err(err) => Err(AdminErrorResponse::ActorError(err)),
    }
}

async fn stop_committer(State(admin): State<Admin>) -> Result<Json<Value>, AdminErrorResponse> {
    let Some(l1_committer) = admin.l1_committer else {
        return Err(AdminErrorResponse::NoHandle);
    };

    match l1_committer
        .request(l1_committer_protocol::StopCommitter)
        .await
    {
        Ok(Ok(())) => Ok(Json::from(Value::String("ok".into()))),
        Ok(Err(err)) => Err(AdminErrorResponse::MessageError(err)),
        Err(err) => Err(AdminErrorResponse::ActorError(err)),
    }
}

async fn health(
    State(admin): State<Admin>,
) -> Result<Json<Map<String, Value>>, AdminErrorResponse> {
    let mut response = serde_json::Map::new();

    response.insert(
        "l1_committer".to_string(),
        actor_health(admin.l1_committer, l1_committer_protocol::Health).await,
    );

    response.insert(
        "l1_watcher".to_string(),
        actor_health(admin.l1_watcher, l1_watcher_protocol::Health).await,
    );

    response.insert(
        "l1_proof_sender".to_string(),
        actor_health(admin.l1_proof_sender, l1_proof_sender_protocol::Health).await,
    );

    response.insert(
        "block_producer".to_string(),
        actor_health(admin.block_producer, block_producer_protocol::Health).await,
    );

    #[cfg(feature = "metrics")]
    {
        response.insert(
            "metrics_gatherer".to_string(),
            actor_health(admin.metrics_gatherer, metrics_gatherer_protocol::Health).await,
        );
    }

    Ok(Json::from(response))
}

pub async fn actor_health<A, M>(actor_ref: Option<ActorRef<A>>, health_msg: M) -> Value
where
    A: spawned_concurrency::tasks::Actor + spawned_concurrency::tasks::Handler<M>,
    M: Message,
    M::Result: Serialize,
{
    if let Some(actor_ref) = actor_ref {
        match actor_ref.request(health_msg).await {
            Ok(health) => serde_json::to_value(health).unwrap_or_else(|err| {
                Value::String(format!("Failed to serialize health message {err}"))
            }),
            Err(err) => Value::String(format!("Actor health returned an error {err}")),
        }
    } else {
        Value::String(
            "Admin server does not have the actor handle. Maybe it's not running?".to_string(),
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

async fn set_sequencer_stop_at(
    State(admin): State<Admin>,
    Path(block_number): Path<u64>,
) -> Result<Json<Value>, AdminErrorResponse> {
    let Some(state_updater) = admin.state_updater else {
        return Err(AdminErrorResponse::NoHandle);
    };

    match state_updater
        .request(state_updater_protocol::StopAt { block_number })
        .await
    {
        Ok(_) => Ok(Json::from(Value::String("ok".into()))),
        Err(err) => Err(AdminErrorResponse::ActorError(err)),
    }
}
