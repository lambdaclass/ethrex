use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Mutex};
use tonic::transport::Channel;
use tracing::{debug, info, warn};

use super::errors::CircuitBreakerError;
use super::sidecar_proto::{
    self, sidecar_transport_client::SidecarTransportClient, CommitHead, Event,
    GetTransactionRequest, NewIteration, ResultStatus, Transaction, TransactionResult,
    TxExecutionId,
};

/// Configuration for the Circuit Breaker gRPC client.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// gRPC endpoint URL for the sidecar (e.g., "http://localhost:50051")
    pub sidecar_url: String,
    /// Timeout for waiting for a transaction result from the sidecar
    pub result_timeout: Duration,
    /// Timeout for the GetTransaction fallback poll
    pub poll_timeout: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            sidecar_url: "http://localhost:50051".to_string(),
            result_timeout: Duration::from_millis(500),
            poll_timeout: Duration::from_millis(200),
        }
    }
}

/// gRPC client for communicating with the Credible Layer Assertion Enforcer sidecar.
///
/// Maintains a persistent bidirectional `StreamEvents` gRPC stream. Events are sent
/// via an mpsc channel that feeds the stream. Transaction results are retrieved via
/// the `GetTransaction` unary RPC.
pub struct CircuitBreakerClient {
    config: CircuitBreakerConfig,
    /// Sender side of the persistent StreamEvents stream
    event_sender: mpsc::Sender<Event>,
    /// Monotonically increasing event ID counter
    event_id_counter: AtomicU64,
    /// Current iteration ID (incremented per block)
    iteration_id: AtomicU64,
    /// gRPC client for unary calls (GetTransaction)
    grpc_client: Arc<Mutex<SidecarTransportClient<Channel>>>,
}

impl CircuitBreakerClient {
    /// Create a new client with lazy connection to the sidecar.
    /// Opens a persistent StreamEvents bidirectional stream in the background.
    pub async fn connect(config: CircuitBreakerConfig) -> Result<Self, CircuitBreakerError> {
        info!(
            url = %config.sidecar_url,
            "Configuring Circuit Breaker sidecar client"
        );

        let channel = Channel::from_shared(config.sidecar_url.clone())
            .map_err(|e| CircuitBreakerError::Internal(format!("Invalid URL: {e}")))?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(5))
            .connect_lazy();

        let mut client = SidecarTransportClient::new(channel.clone());

        // Create the event channel. The sender goes to the client, the receiver
        // is owned by the background stream task.
        let (event_tx, mut event_rx) = mpsc::channel::<Event>(256);

        // Background task: maintains a persistent StreamEvents connection.
        // Reads events from the mpsc channel and forwards them to the gRPC stream.
        // Reconnects automatically if the connection drops.
        tokio::spawn(async move {
            loop {
                // Create a new gRPC-side channel for each connection attempt
                let (grpc_tx, grpc_rx) = mpsc::channel::<Event>(64);
                let grpc_stream = tokio_stream::wrappers::ReceiverStream::new(grpc_rx);

                match client.stream_events(grpc_stream).await {
                    Ok(response) => {
                        info!("StreamEvents stream connected to sidecar");
                        let mut ack_stream = response.into_inner();

                        // Send an initial CommitHead (block 0) as the first event on
                        // every new stream — the sidecar requires CommitHead first.
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
                                // Read events from the main channel and forward to gRPC
                                event = event_rx.recv() => {
                                    match event {
                                        Some(e) => {
                                            let event_type = match &e.event {
                                                Some(sidecar_proto::event::Event::CommitHead(_)) => "CommitHead",
                                                Some(sidecar_proto::event::Event::NewIteration(_)) => "NewIteration",
                                                Some(sidecar_proto::event::Event::Transaction(_)) => "Transaction",
                                                Some(sidecar_proto::event::Event::Reorg(_)) => "Reorg",
                                                None => "None",
                                            };
                                            debug!("Forwarding {event_type} event to gRPC stream (event_id={})", e.event_id);
                                            if grpc_tx.send(e).await.is_err() {
                                                warn!("gRPC stream send failed, reconnecting");
                                                break;
                                            }
                                        }
                                        None => {
                                            // Main channel closed — client dropped
                                            debug!("Event channel closed, stopping stream task");
                                            return;
                                        }
                                    }
                                }
                                // Read acks from sidecar
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
                        debug!(%status, "StreamEvents connect failed, retrying in 5s");
                    }
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });

        info!("Circuit Breaker client ready (persistent stream opened)");

        Ok(Self {
            config,
            event_sender: event_tx,
            event_id_counter: AtomicU64::new(1),
            iteration_id: AtomicU64::new(0),
            grpc_client: Arc::new(Mutex::new(SidecarTransportClient::new(channel))),
        })
    }

    /// Get the next event ID.
    fn next_event_id(&self) -> u64 {
        self.event_id_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Get the current iteration ID.
    pub fn current_iteration_id(&self) -> u64 {
        self.iteration_id.load(Ordering::Relaxed)
    }

    /// Send a CommitHead event (previous block finalized).
    pub async fn send_commit_head(
        &self,
        commit_head: CommitHead,
    ) -> Result<(), CircuitBreakerError> {
        let event = Event {
            event_id: self.next_event_id(),
            event: Some(sidecar_proto::event::Event::CommitHead(commit_head)),
        };
        self.event_sender
            .send(event)
            .await
            .map_err(|_| CircuitBreakerError::StreamClosed)
    }

    /// Send a NewIteration event (new block started) and increment the iteration ID.
    pub async fn send_new_iteration(
        &self,
        new_iteration: NewIteration,
    ) -> Result<(), CircuitBreakerError> {
        self.iteration_id.fetch_add(1, Ordering::Relaxed);
        let event = Event {
            event_id: self.next_event_id(),
            event: Some(sidecar_proto::event::Event::NewIteration(new_iteration)),
        };
        self.event_sender
            .send(event)
            .await
            .map_err(|_| CircuitBreakerError::StreamClosed)
    }

    /// Send a Transaction event and wait for the sidecar's verdict.
    ///
    /// Returns `true` if the transaction should be included, `false` if it should be dropped.
    /// On any error or timeout, returns `true` (permissive behavior).
    pub async fn evaluate_transaction(&self, transaction: Transaction) -> bool {
        let tx_exec_id = transaction.tx_execution_id.clone();
        let tx_hash = tx_exec_id
            .as_ref()
            .map(|id| id.tx_hash.clone())
            .unwrap_or_default();
        let block_number = tx_exec_id
            .as_ref()
            .map(|id| id.block_number.clone())
            .unwrap_or_default();
        let index = tx_exec_id.as_ref().map(|id| id.index).unwrap_or(0);

        // Send the transaction event on the persistent stream
        let event = Event {
            event_id: self.next_event_id(),
            event: Some(sidecar_proto::event::Event::Transaction(transaction)),
        };
        if self.event_sender.send(event).await.is_err() {
            warn!("StreamEvents channel closed, including tx (permissive)");
            return true;
        }

        // Poll for result with retries (sidecar evaluates async).
        // The sidecar needs time to receive the tx event (via async stream),
        // evaluate it, and make the result available.
        let poll_attempts = 10;
        let poll_interval = Duration::from_millis(200);
        for attempt in 0..poll_attempts {
            tokio::time::sleep(poll_interval).await;
            let result = self.poll_transaction_result(&tx_hash, &block_number, index).await;
            match result {
                PollResult::Found(include) => return include,
                PollResult::NotFound => {
                    debug!("GetTransaction poll attempt {}/{}: not found yet", attempt + 1, poll_attempts);
                    continue;
                }
                PollResult::Error => return true, // permissive
            }
        }
        warn!("GetTransaction: no result after {poll_attempts} attempts, including tx (permissive)");
        true
    }

    /// Poll for a transaction result via GetTransaction unary RPC.
    async fn poll_transaction_result(&self, tx_hash: &[u8], block_number: &[u8], index: u64) -> PollResult {
        let tx_exec_id = TxExecutionId {
            block_number: block_number.to_vec(),
            iteration_id: self.current_iteration_id(),
            tx_hash: tx_hash.to_vec(),
            index,
        };

        let request = GetTransactionRequest {
            tx_execution_id: Some(tx_exec_id),
        };

        let poll_result = tokio::time::timeout(self.config.poll_timeout, async {
            let mut client = self.grpc_client.lock().await;
            client.get_transaction(request).await
        })
        .await;

        match poll_result {
            Ok(Ok(response)) => {
                let inner = response.into_inner();
                match inner.outcome {
                    Some(sidecar_proto::get_transaction_response::Outcome::Result(result)) => {
                        PollResult::Found(!is_assertion_failed(&result))
                    }
                    Some(sidecar_proto::get_transaction_response::Outcome::NotFound(_)) => {
                        PollResult::NotFound
                    }
                    None => PollResult::NotFound,
                }
            }
            Ok(Err(status)) => {
                warn!(%status, "GetTransaction poll failed, including tx (permissive)");
                PollResult::Error
            }
            Err(_) => {
                warn!("GetTransaction poll timed out, including tx (permissive)");
                PollResult::Error
            }
        }
    }
}

enum PollResult {
    Found(bool), // true = include, false = drop
    NotFound,
    Error,
}

/// Check if a TransactionResult indicates assertion failure.
fn is_assertion_failed(result: &TransactionResult) -> bool {
    result.status() == ResultStatus::AssertionFailed
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::sequencer::circuit_breaker::sidecar_proto::{
        CommitHead, NewIteration, ResultStatus, TransactionResult, TxExecutionId,
    };

    #[test]
    fn config_default_url_is_localhost_50051() {
        let cfg = CircuitBreakerConfig::default();
        assert_eq!(cfg.sidecar_url, "http://localhost:50051");
    }

    #[test]
    fn config_default_result_timeout_is_500ms() {
        let cfg = CircuitBreakerConfig::default();
        assert_eq!(cfg.result_timeout, Duration::from_millis(500));
    }

    #[test]
    fn config_default_poll_timeout_is_200ms() {
        let cfg = CircuitBreakerConfig::default();
        assert_eq!(cfg.poll_timeout, Duration::from_millis(200));
    }

    fn make_result(status: ResultStatus) -> TransactionResult {
        TransactionResult {
            tx_execution_id: None,
            status: status as i32,
            gas_used: 0,
            error: String::new(),
        }
    }

    #[test]
    fn assertion_failed_returns_true_for_assertion_failed_status() {
        assert!(is_assertion_failed(&make_result(ResultStatus::AssertionFailed)));
    }

    #[test]
    fn assertion_failed_returns_false_for_success_status() {
        assert!(!is_assertion_failed(&make_result(ResultStatus::Success)));
    }

    #[test]
    fn assertion_failed_returns_false_for_reverted_status() {
        assert!(!is_assertion_failed(&make_result(ResultStatus::Reverted)));
    }

    #[test]
    fn assertion_failed_returns_false_for_halted_status() {
        assert!(!is_assertion_failed(&make_result(ResultStatus::Halted)));
    }

    #[test]
    fn assertion_failed_returns_false_for_failed_status() {
        assert!(!is_assertion_failed(&make_result(ResultStatus::Failed)));
    }

    #[test]
    fn assertion_failed_returns_false_for_unspecified_status() {
        assert!(!is_assertion_failed(&make_result(ResultStatus::Unspecified)));
    }

    #[test]
    fn commit_head_fields_are_set_correctly() {
        let block_number: Vec<u8> = std::iter::repeat(0u8).take(31).chain(std::iter::once(42u8)).collect();
        let timestamp: Vec<u8> = std::iter::repeat(0u8).take(31).chain(std::iter::once(1u8)).collect();
        let ch = CommitHead {
            last_tx_hash: None, n_transactions: 5, block_number: block_number.clone(),
            selected_iteration_id: 3, block_hash: None, parent_beacon_block_root: None,
            timestamp: timestamp.clone(),
        };
        assert_eq!(ch.n_transactions, 5);
        assert_eq!(ch.block_number, block_number);
        assert_eq!(ch.selected_iteration_id, 3);
    }

    #[test]
    fn tx_execution_id_fields_are_set_correctly() {
        let id = TxExecutionId { block_number: vec![0; 32], iteration_id: 7, tx_hash: vec![0xab; 32], index: 2 };
        assert_eq!(id.iteration_id, 7);
        assert_eq!(id.index, 2);
    }

    #[test]
    fn new_iteration_has_expected_iteration_id() {
        use crate::sequencer::circuit_breaker::sidecar_proto::BlockEnv;
        let ni = NewIteration {
            block_env: Some(BlockEnv { number: vec![0; 32], beneficiary: vec![0; 20], timestamp: vec![0; 32], gas_limit: 30_000_000, basefee: 1_000_000_000, difficulty: vec![0; 32], prevrandao: None, blob_excess_gas_and_price: None }),
            iteration_id: 42, parent_block_hash: None, parent_beacon_block_root: None,
        };
        assert_eq!(ni.iteration_id, 42);
    }

    #[test]
    fn event_id_counter_increments() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let c = AtomicU64::new(1);
        assert_eq!(c.fetch_add(1, Ordering::Relaxed), 1);
        assert_eq!(c.fetch_add(1, Ordering::Relaxed), 2);
        assert_eq!(c.fetch_add(1, Ordering::Relaxed), 3);
    }
}
