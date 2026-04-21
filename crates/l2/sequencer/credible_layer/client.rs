use std::time::Duration;

use ethrex_common::types::{BlockHeader, Transaction};
use ethrex_common::{Address, H256};
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{
        Actor, ActorRef, ActorStart as _, Context, Handler, Response, send_after, spawn_listener,
    },
};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tracing::{debug, info, warn};

use super::errors::CredibleLayerError;
use super::sidecar_proto::{
    self, BlobExcessGasAndPrice, BlockEnv, CommitHead, Event, GetTransactionRequest, NewIteration,
    ResultStatus, Transaction as SidecarTransaction, TxExecutionId,
    sidecar_transport_client::SidecarTransportClient,
};

#[protocol]
pub trait CredibleLayerProtocol: Send + Sync {
    /// Notify the sidecar that a new block iteration has started.
    fn new_iteration(&self, header: BlockHeader) -> Result<(), ActorError>;

    /// Notify the sidecar that a block has been committed.
    fn commit_head(
        &self,
        block_number: u64,
        block_hash: H256,
        timestamp: u64,
        tx_count: u64,
        last_tx_hash: Option<H256>,
    ) -> Result<(), ActorError>;

    /// Send a transaction event to the sidecar (pre-execution, fire-and-forget).
    fn send_transaction(
        &self,
        tx_hash: H256,
        block_number: u64,
        tx_index: u64,
        sender: Address,
        tx: Transaction,
    ) -> Result<(), ActorError>;

    /// Poll the sidecar for a transaction verdict (post-execution).
    /// Returns `true` if the transaction should be included, `false` if it should be dropped.
    /// On any error or timeout, returns `true` (permissive — liveness over safety).
    fn check_transaction(&self, tx_hash: H256, block_number: u64, tx_index: u64) -> Response<bool>;

    /// Attempt to (re)connect to the sidecar. Scheduled by `send_after` on failure.
    fn reconnect(&self) -> Result<(), ActorError>;

    /// Process a stream ack from the sidecar. When `disconnected` is true,
    /// the stream has ended and a reconnect is scheduled.
    fn stream_ack(
        &self,
        disconnected: bool,
        success: bool,
        event_id: u64,
        message: String,
    ) -> Result<(), ActorError>;
}

/// gRPC client actor for communicating with the Credible Layer Assertion Enforcer sidecar.
///
/// Maintains a persistent bidirectional `StreamEvents` gRPC stream. Connection is
/// established in `#[started]` and ack messages are bridged into the actor via
/// `spawn_listener`. Reconnection on failure is scheduled with `send_after`.
pub struct CredibleLayerClient {
    /// Write handle for the gRPC StreamEvents bidirectional stream (Rust equivalent
    /// of Java gRPC's `StreamObserver.onNext()`). None when disconnected.
    sidecar_tx: Option<mpsc::Sender<Event>>,
    /// Monotonically increasing event ID counter
    event_id_counter: u64,
    /// Current iteration ID (incremented per block)
    iteration_id: u64,
    /// gRPC client for sidecar RPCs (StreamEvents, GetTransaction).
    sidecar_client: SidecarTransportClient<Channel>,
    /// Whether the StreamEvents stream is currently active.
    stream_connected: bool,
}

#[actor(protocol = CredibleLayerProtocol)]
impl CredibleLayerClient {
    /// Spawn the Credible Layer client actor.
    pub async fn spawn(sidecar_url: String) -> Result<ActorRef<Self>, CredibleLayerError> {
        let client = Self::new(sidecar_url)?;
        Ok(client.start())
    }

    #[allow(clippy::result_large_err)]
    fn new(sidecar_url: String) -> Result<Self, CredibleLayerError> {
        info!(url = %sidecar_url, "Configuring Credible Layer sidecar client");

        let channel = Channel::from_shared(sidecar_url)
            .map_err(|e| CredibleLayerError::Internal(format!("Invalid URL: {e}")))?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(5))
            .connect_lazy();

        Ok(Self {
            sidecar_client: SidecarTransportClient::new(channel),
            sidecar_tx: None,
            event_id_counter: 1,
            iteration_id: 0,
            stream_connected: false,
        })
    }

    /// Attempt to establish a bidirectional StreamEvents connection with the sidecar.
    /// On success, stores the event sender and spawns an ack listener.
    /// On failure, schedules a retry via `send_after`.
    async fn open_event_stream(&mut self, ctx: &Context<Self>) {
        let (tx, rx) = mpsc::channel::<Event>(64);
        let grpc_stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        match self.sidecar_client.stream_events(grpc_stream).await {
            Ok(response) => {
                info!("StreamEvents stream stream_connected to sidecar");

                // The sidecar requires CommitHead as the first event on every new stream.
                let init_commit = Event {
                    event_id: 0,
                    event: Some(sidecar_proto::event::Event::CommitHead(CommitHead {
                        last_tx_hash: None,
                        n_transactions: 0,
                        block_number: vec![0u8; 32],
                        selected_iteration_id: 0,
                        block_hash: Some(vec![0u8; 32]),
                        parent_beacon_block_root: None,
                        timestamp: vec![0u8; 32],
                    })),
                };
                if tx.send(init_commit).await.is_err() {
                    warn!("Failed to send initial CommitHead, scheduling reconnect");
                    send_after(
                        Duration::from_secs(5),
                        ctx.clone(),
                        credible_layer_protocol::Reconnect,
                    );
                    return;
                }

                self.sidecar_tx = Some(tx);
                self.stream_connected = true;

                // Bridge the ack stream into actor messages via spawn_listener.
                // When the stream ends or errors, a final "disconnected" message is sent.
                let ack_stream = response.into_inner();
                let mapped = ack_stream
                    .map(|result| match result {
                        Ok(ack) => credible_layer_protocol::StreamAck {
                            disconnected: false,
                            success: ack.success,
                            event_id: ack.event_id,
                            message: ack.message,
                        },
                        Err(status) => credible_layer_protocol::StreamAck {
                            disconnected: true,
                            success: false,
                            event_id: 0,
                            message: status.to_string(),
                        },
                    })
                    .chain(tokio_stream::once(credible_layer_protocol::StreamAck {
                        disconnected: true,
                        success: false,
                        event_id: 0,
                        message: "ack stream ended".into(),
                    }));

                spawn_listener(ctx.clone(), mapped);
            }
            Err(status) => {
                warn!(%status, "StreamEvents connect failed, retrying in 5s");
                send_after(
                    Duration::from_secs(5),
                    ctx.clone(),
                    credible_layer_protocol::Reconnect,
                );
            }
        }
    }

    fn next_event_id(&mut self) -> u64 {
        let id = self.event_id_counter;
        self.event_id_counter += 1;
        id
    }

    /// Send an event on the active gRPC stream. If the channel is closed,
    /// marks the connection as disconnected.
    async fn send_event(&mut self, event_payload: sidecar_proto::event::Event) {
        let event = Event {
            event_id: self.next_event_id(),
            event: Some(event_payload),
        };
        if let Some(ref sender) = self.sidecar_tx
            && sender.send(event).await.is_err()
        {
            warn!("Event channel closed, marking disconnected");
            self.stream_connected = false;
            self.sidecar_tx = None;
        }
    }

    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        self.open_event_stream(ctx).await;
    }

    #[send_handler]
    async fn handle_reconnect(
        &mut self,
        _msg: credible_layer_protocol::Reconnect,
        ctx: &Context<Self>,
    ) {
        if self.stream_connected {
            return;
        }
        self.open_event_stream(ctx).await;
    }

    #[send_handler]
    async fn handle_stream_ack(
        &mut self,
        msg: credible_layer_protocol::StreamAck,
        ctx: &Context<Self>,
    ) {
        if msg.disconnected {
            if !self.stream_connected {
                return;
            }
            info!("Sidecar stream disconnected: {}", msg.message);
            self.stream_connected = false;
            self.sidecar_tx = None;
            send_after(
                Duration::from_secs(5),
                ctx.clone(),
                credible_layer_protocol::Reconnect,
            );
        } else if !msg.success {
            warn!(
                event_id = msg.event_id,
                msg = %msg.message,
                "Sidecar rejected event"
            );
        }
    }

    #[send_handler]
    async fn handle_new_iteration(
        &mut self,
        msg: credible_layer_protocol::NewIteration,
        _ctx: &Context<Self>,
    ) {
        self.iteration_id += 1;
        let header = &msg.header;

        let block_env = BlockEnv {
            number: u64_to_u256_bytes(header.number),
            beneficiary: header.coinbase.as_bytes().to_vec(),
            timestamp: u64_to_u256_bytes(header.timestamp),
            gas_limit: header.gas_limit,
            basefee: header.base_fee_per_gas.unwrap_or(0),
            difficulty: header.difficulty.to_big_endian().to_vec(),
            prevrandao: Some(header.prev_randao.to_fixed_bytes().to_vec()),
            blob_excess_gas_and_price: Some(BlobExcessGasAndPrice {
                excess_blob_gas: 0,
                blob_gasprice: vec![0u8; 16],
            }),
        };
        let new_iteration = NewIteration {
            block_env: Some(block_env),
            iteration_id: self.iteration_id,
            parent_block_hash: Some(header.parent_hash.to_fixed_bytes().to_vec()),
            parent_beacon_block_root: header
                .parent_beacon_block_root
                .map(|h| h.to_fixed_bytes().to_vec()),
        };
        self.send_event(sidecar_proto::event::Event::NewIteration(new_iteration))
            .await;
    }

    #[send_handler]
    async fn handle_commit_head(
        &mut self,
        msg: credible_layer_protocol::CommitHead,
        _ctx: &Context<Self>,
    ) {
        let commit_head = CommitHead {
            last_tx_hash: msg.last_tx_hash.map(|h| h.to_fixed_bytes().to_vec()),
            n_transactions: msg.tx_count,
            block_number: u64_to_u256_bytes(msg.block_number),
            selected_iteration_id: self.iteration_id,
            block_hash: Some(msg.block_hash.to_fixed_bytes().to_vec()),
            parent_beacon_block_root: None,
            timestamp: u64_to_u256_bytes(msg.timestamp),
        };
        self.send_event(sidecar_proto::event::Event::CommitHead(commit_head))
            .await;
    }

    #[send_handler]
    async fn handle_send_transaction(
        &mut self,
        msg: credible_layer_protocol::SendTransaction,
        _ctx: &Context<Self>,
    ) {
        if !self.stream_connected {
            return;
        }
        let tx_execution_id = Some(TxExecutionId {
            block_number: u64_to_u256_bytes(msg.block_number),
            iteration_id: self.iteration_id,
            tx_hash: msg.tx_hash.as_bytes().to_vec(),
            index: msg.tx_index,
        });
        let tx_env = (msg.tx, msg.sender).into();
        let sidecar_tx = SidecarTransaction {
            tx_execution_id,
            tx_env: Some(tx_env),
            prev_tx_hash: None,
        };
        self.send_event(sidecar_proto::event::Event::Transaction(sidecar_tx))
            .await;
    }

    #[request_handler]
    async fn handle_check_transaction(
        &mut self,
        msg: credible_layer_protocol::CheckTransaction,
        _ctx: &Context<Self>,
    ) -> bool {
        if !self.stream_connected {
            return true;
        }

        let tx_exec_id = TxExecutionId {
            block_number: u64_to_u256_bytes(msg.block_number),
            iteration_id: self.iteration_id,
            tx_hash: msg.tx_hash.as_bytes().to_vec(),
            index: msg.tx_index,
        };

        let poll_attempts = 10;
        let poll_interval = Duration::from_millis(200);
        let poll_timeout = Duration::from_millis(200);

        for attempt in 0..poll_attempts {
            tokio::time::sleep(poll_interval).await;
            let request = GetTransactionRequest {
                tx_execution_id: Some(tx_exec_id.clone()),
            };
            match tokio::time::timeout(poll_timeout, self.sidecar_client.get_transaction(request))
                .await
            {
                Ok(Ok(response)) => {
                    let inner = response.into_inner();
                    match inner.outcome {
                        Some(sidecar_proto::get_transaction_response::Outcome::Result(result)) => {
                            return result.status() != ResultStatus::AssertionFailed;
                        }
                        Some(sidecar_proto::get_transaction_response::Outcome::NotFound(_)) => {
                            debug!(
                                "GetTransaction poll attempt {}/{}: not found yet",
                                attempt + 1,
                                poll_attempts
                            );
                            continue;
                        }
                        None => continue,
                    }
                }
                Ok(Err(status)) => {
                    warn!(%status, "GetTransaction poll failed, including tx (permissive)");
                    return true;
                }
                Err(_) => {
                    warn!("GetTransaction poll timed out, including tx (permissive)");
                    return true;
                }
            }
        }
        warn!(
            "GetTransaction: no result after {poll_attempts} attempts, including tx (permissive)"
        );
        true
    }
}

/// Encode a u64 as a 32-byte big-endian U256 for protobuf fields.
fn u64_to_u256_bytes(value: u64) -> Vec<u8> {
    let mut buf = [0u8; 32];
    buf[24..].copy_from_slice(&value.to_be_bytes());
    buf.to_vec()
}
