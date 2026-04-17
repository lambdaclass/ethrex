/// Mock Credible Layer Sidecar for end-to-end testing.
///
/// Implements the sidecar.proto gRPC protocol and rejects any transaction
/// that calls the `transferOwnership(address)` function selector (0xf2fde38b).
///
/// Usage:
///   cargo run --bin mock-sidecar
///
/// The mock listens on 0.0.0.0:50051 (same as the real sidecar).
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tonic::{Request, Response, Status, transport::Server};

// Include the generated protobuf code
pub mod sidecar_proto {
    tonic::include_proto!("sidecar.transport.v1");
}

use sidecar_proto::{
    Event, GetTransactionRequest, GetTransactionResponse, GetTransactionsRequest,
    GetTransactionsResponse, ResultStatus, StreamAck, SubscribeResultsRequest, TransactionResult,
    get_transaction_response::Outcome,
    sidecar_transport_server::{SidecarTransport, SidecarTransportServer},
};

/// The `transferOwnership(address)` function selector
const TRANSFER_OWNERSHIP_SELECTOR: [u8; 4] = [0xf2, 0xfd, 0xe3, 0x8b];

/// Shared state: stores transaction results keyed by tx_hash
type ResultStore = Arc<Mutex<HashMap<Vec<u8>, TransactionResult>>>;

pub struct MockSidecar {
    results: ResultStore,
}

impl MockSidecar {
    fn new() -> Self {
        Self {
            results: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Evaluate a transaction: returns ASSERTION_FAILED if it calls transferOwnership,
/// SUCCESS otherwise.
fn evaluate_tx(event: &sidecar_proto::Transaction) -> ResultStatus {
    let calldata = event
        .tx_env
        .as_ref()
        .map(|env| &env.data[..])
        .unwrap_or(&[]);

    if calldata.len() >= 4 && calldata[..4] == TRANSFER_OWNERSHIP_SELECTOR {
        ResultStatus::AssertionFailed
    } else {
        ResultStatus::Success
    }
}

#[tonic::async_trait]
impl SidecarTransport for MockSidecar {
    type StreamEventsStream =
        Pin<Box<dyn Stream<Item = Result<StreamAck, Status>> + Send + 'static>>;
    type SubscribeResultsStream =
        Pin<Box<dyn Stream<Item = Result<TransactionResult, Status>> + Send + 'static>>;

    async fn stream_events(
        &self,
        request: Request<tonic::Streaming<Event>>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let mut stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);
        let results = self.results.clone();

        tokio::spawn(async move {
            let mut events_processed: u64 = 0;
            while let Some(Ok(event)) = stream.next().await {
                let event_id = event.event_id;
                events_processed += 1;

                match &event.event {
                    Some(sidecar_proto::event::Event::CommitHead(ch)) => {
                        eprintln!("[MOCK] CommitHead: n_transactions={}", ch.n_transactions);
                    }
                    Some(sidecar_proto::event::Event::NewIteration(ni)) => {
                        eprintln!("[MOCK] NewIteration: iteration_id={}", ni.iteration_id);
                    }
                    Some(sidecar_proto::event::Event::Transaction(t)) => {
                        let tx_hash = t
                            .tx_execution_id
                            .as_ref()
                            .map(|id| id.tx_hash.clone())
                            .unwrap_or_default();
                        let tx_hash_hex = hex::encode(&tx_hash);

                        let status = evaluate_tx(t);

                        let result = TransactionResult {
                            tx_execution_id: t.tx_execution_id.clone(),
                            status: status as i32,
                            gas_used: 21000,
                            error: String::new(),
                        };

                        // Store the result so GetTransaction can find it
                        {
                            let mut store = results.lock().await;
                            store.insert(tx_hash.clone(), result);
                        }

                        match status {
                            ResultStatus::AssertionFailed => {
                                eprintln!(
                                    "[MOCK] TX {}: ASSERTION_FAILED (transferOwnership)",
                                    &tx_hash_hex[..std::cmp::min(16, tx_hash_hex.len())]
                                );
                            }
                            _ => {
                                eprintln!(
                                    "[MOCK] TX {}: SUCCESS",
                                    &tx_hash_hex[..std::cmp::min(16, tx_hash_hex.len())]
                                );
                            }
                        }
                    }
                    Some(sidecar_proto::event::Event::Reorg(_)) => {
                        eprintln!("[MOCK] ReorgEvent received");
                    }
                    None => {}
                }

                let ack = StreamAck {
                    success: true,
                    message: String::new(),
                    events_processed,
                    event_id,
                };
                if tx.send(Ok(ack)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn subscribe_results(
        &self,
        _request: Request<SubscribeResultsRequest>,
    ) -> Result<Response<Self::SubscribeResultsStream>, Status> {
        // Return an empty stream — results are delivered via GetTransaction
        let (_tx, rx) = mpsc::channel(1);
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn get_transactions(
        &self,
        _request: Request<GetTransactionsRequest>,
    ) -> Result<Response<GetTransactionsResponse>, Status> {
        Ok(Response::new(GetTransactionsResponse {
            results: vec![],
            not_found: vec![],
        }))
    }

    async fn get_transaction(
        &self,
        request: Request<GetTransactionRequest>,
    ) -> Result<Response<GetTransactionResponse>, Status> {
        let req = request.into_inner();
        let tx_hash = req
            .tx_execution_id
            .as_ref()
            .map(|id| id.tx_hash.clone())
            .unwrap_or_default();

        // Look up stored result from stream_events processing
        let store = self.results.lock().await;
        if let Some(result) = store.get(&tx_hash) {
            let status_name = match ResultStatus::try_from(result.status) {
                Ok(ResultStatus::AssertionFailed) => "ASSERTION_FAILED",
                Ok(ResultStatus::Success) => "SUCCESS",
                _ => "OTHER",
            };
            eprintln!(
                "[MOCK] GetTransaction {}: returning {}",
                &hex::encode(&tx_hash)[..std::cmp::min(16, tx_hash.len() * 2)],
                status_name
            );
            Ok(Response::new(GetTransactionResponse {
                outcome: Some(Outcome::Result(result.clone())),
            }))
        } else {
            // Not found yet — the stream_events call may not have completed
            eprintln!(
                "[MOCK] GetTransaction {}: NOT_FOUND",
                &hex::encode(&tx_hash)[..std::cmp::min(16, tx_hash.len() * 2)]
            );
            Ok(Response::new(GetTransactionResponse {
                outcome: Some(Outcome::NotFound(tx_hash)),
            }))
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:50051".parse()?;
    eprintln!("[MOCK SIDECAR] Starting on {addr}");
    eprintln!(
        "[MOCK SIDECAR] Will reject transactions calling transferOwnership(address) [0xf2fde38b]"
    );

    Server::builder()
        .add_service(SidecarTransportServer::new(MockSidecar::new()))
        .serve(addr)
        .await?;

    Ok(())
}
