use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ethrex_common::types::{BlockHeader, Transaction, TxKind};
use ethrex_common::{Address, H256};
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, Response},
};
use tokio::sync::mpsc;
use tonic::transport::Channel;
use tracing::{debug, info, warn};

use super::sidecar_proto::{
    self, AccessListItem, Authorization, BlobExcessGasAndPrice, BlockEnv, CommitHead, Event,
    GetTransactionRequest, NewIteration, ResultStatus, Transaction as SidecarTransaction,
    TransactionEnv, TxExecutionId, sidecar_transport_client::SidecarTransportClient,
};

use super::errors::CredibleLayerError;

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
}

/// gRPC client actor for communicating with the Credible Layer Assertion Enforcer sidecar.
///
/// Maintains a persistent bidirectional `StreamEvents` gRPC stream via a background task.
/// Events are sent through an mpsc channel that feeds the stream. Transaction results
/// are retrieved via the `GetTransaction` unary RPC.
pub struct CredibleLayerClient {
    /// Sender side of the persistent StreamEvents stream
    event_sender: mpsc::Sender<Event>,
    /// Monotonically increasing event ID counter
    event_id_counter: u64,
    /// Current iteration ID (incremented per block)
    iteration_id: u64,
    /// gRPC client for unary calls (GetTransaction)
    grpc_client: SidecarTransportClient<Channel>,
    /// Whether the StreamEvents stream is currently connected.
    /// When false, send handlers skip immediately (permissive).
    stream_connected: Arc<AtomicBool>,
}

#[actor(protocol = CredibleLayerProtocol)]
impl CredibleLayerClient {
    /// Spawn the Credible Layer client actor.
    pub async fn spawn(sidecar_url: String) -> Result<ActorRef<Self>, CredibleLayerError> {
        let client = Self::new(sidecar_url).await?;
        Ok(client.start())
    }

    async fn new(sidecar_url: String) -> Result<Self, CredibleLayerError> {
        info!(url = %sidecar_url, "Configuring Credible Layer sidecar client");

        let channel = Channel::from_shared(sidecar_url)
            .map_err(|e| CredibleLayerError::Internal(format!("Invalid URL: {e}")))?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(5))
            .connect_lazy();

        let mut stream_client = SidecarTransportClient::new(channel.clone());
        let stream_connected = Arc::new(AtomicBool::new(false));
        let stream_connected_bg = stream_connected.clone();

        let (event_tx, mut event_rx) = mpsc::channel::<Event>(256);

        // Background task: maintains a persistent StreamEvents connection.
        // Reads events from the mpsc channel and forwards them to the gRPC stream.
        // Reconnects automatically if the connection drops.
        tokio::spawn(async move {
            loop {
                let (grpc_tx, grpc_rx) = mpsc::channel::<Event>(64);
                let grpc_stream = tokio_stream::wrappers::ReceiverStream::new(grpc_rx);

                match stream_client.stream_events(grpc_stream).await {
                    Ok(response) => {
                        info!("StreamEvents stream connected to sidecar");
                        stream_connected_bg.store(true, Ordering::Relaxed);
                        let mut ack_stream = response.into_inner();

                        // Send an initial CommitHead (block 0) — the sidecar requires
                        // CommitHead as the first event on every new stream.
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
                        if grpc_tx.send(init_commit).await.is_err() {
                            warn!("Failed to send initial CommitHead");
                            continue;
                        }

                        // Forward events from the main channel to the gRPC stream
                        // while also reading acks
                        loop {
                            tokio::select! {
                                event = event_rx.recv() => {
                                    match event {
                                        Some(e) => {
                                            if grpc_tx.send(e).await.is_err() {
                                                warn!("gRPC stream send failed, reconnecting");
                                                break;
                                            }
                                        }
                                        None => {
                                            warn!("Event channel closed, stopping stream task");
                                            return;
                                        }
                                    }
                                }
                                ack = ack_stream.message() => {
                                    match ack {
                                        Ok(Some(a)) => {
                                            if !a.success {
                                                warn!(event_id = a.event_id, msg = %a.message, "Sidecar rejected event");
                                            }
                                        }
                                        Ok(None) => {
                                            info!("StreamEvents ack stream ended, reconnecting");
                                            break;
                                        }
                                        Err(status) => {
                                            warn!(%status, "StreamEvents ack error, reconnecting");
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(status) => {
                        warn!(%status, "StreamEvents connect failed, retrying in 5s");
                    }
                }
                stream_connected_bg.store(false, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });

        info!("Credible Layer client ready (persistent stream opened)");

        Ok(Self {
            event_sender: event_tx,
            event_id_counter: 1,
            iteration_id: 0,
            grpc_client: SidecarTransportClient::new(channel),
            stream_connected,
        })
    }

    fn next_event_id(&mut self) -> u64 {
        let id = self.event_id_counter;
        self.event_id_counter += 1;
        id
    }

    async fn send_event(&mut self, event_payload: sidecar_proto::event::Event) {
        let event = Event {
            event_id: self.next_event_id(),
            event: Some(event_payload),
        };
        if self.event_sender.send(event).await.is_err() {
            warn!("Failed to send event: stream channel closed");
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
        if !self.stream_connected.load(Ordering::Relaxed) {
            return;
        }
        let tx_execution_id = Some(TxExecutionId {
            block_number: u64_to_u256_bytes(msg.block_number),
            iteration_id: self.iteration_id,
            tx_hash: msg.tx_hash.as_bytes().to_vec(),
            index: msg.tx_index,
        });
        let tx_env = build_transaction_env(&msg.tx, msg.sender);
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
        if !self.stream_connected.load(Ordering::Relaxed) {
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
            match tokio::time::timeout(poll_timeout, self.grpc_client.get_transaction(request))
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

/// Build a `TransactionEnv` protobuf message from an ethrex transaction and its sender.
fn build_transaction_env(tx: &Transaction, sender: Address) -> TransactionEnv {
    let transact_to = match tx.to() {
        TxKind::Call(addr) => addr.as_bytes().to_vec(),
        TxKind::Create => vec![],
    };

    let value_bytes = tx.value().to_big_endian();

    let mut gas_price_bytes = [0u8; 16];
    let gas_price_u128 = tx.gas_price().as_u128();
    gas_price_bytes.copy_from_slice(&gas_price_u128.to_be_bytes());

    let gas_priority_fee = tx.max_priority_fee().map(|fee| {
        let mut buf = [0u8; 16];
        buf[8..].copy_from_slice(&fee.to_be_bytes());
        buf.to_vec()
    });

    let access_list = tx
        .access_list()
        .iter()
        .map(|(addr, keys)| AccessListItem {
            address: addr.as_bytes().to_vec(),
            storage_keys: keys.iter().map(|k| k.as_bytes().to_vec()).collect(),
        })
        .collect();

    let authorization_list = tx
        .authorization_list()
        .map(|list| {
            list.iter()
                .map(|auth| {
                    let chain_id_bytes = auth.chain_id.to_big_endian();
                    let r_bytes = auth.r_signature.to_big_endian();
                    let s_bytes = auth.s_signature.to_big_endian();
                    Authorization {
                        chain_id: chain_id_bytes.to_vec(),
                        address: auth.address.as_bytes().to_vec(),
                        nonce: auth.nonce,
                        y_parity: auth.y_parity.as_u32(),
                        r: r_bytes.to_vec(),
                        s: s_bytes.to_vec(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    #[allow(clippy::as_conversions)]
    TransactionEnv {
        tx_type: u8::from(tx.tx_type()) as u32,
        caller: sender.as_bytes().to_vec(),
        gas_limit: tx.gas_limit(),
        gas_price: gas_price_bytes.to_vec(),
        transact_to,
        value: value_bytes.to_vec(),
        data: tx.data().to_vec(),
        nonce: tx.nonce(),
        chain_id: tx.chain_id(),
        access_list,
        gas_priority_fee,
        blob_hashes: tx
            .blob_versioned_hashes()
            .iter()
            .map(|h| h.as_bytes().to_vec())
            .collect(),
        max_fee_per_blob_gas: vec![0u8; 16],
        authorization_list,
    }
}
