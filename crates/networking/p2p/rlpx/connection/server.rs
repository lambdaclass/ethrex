#[cfg(feature = "l2")]
use crate::rlpx::l2::{
    PERIODIC_BATCH_BROADCAST_INTERVAL, PERIODIC_BLOCK_BROADCAST_INTERVAL,
    l2_connection::{
        self, L2Cast, L2ConnState, handle_based_capability_message, handle_l2_broadcast,
    },
};
use crate::{
    backend,
    metrics::METRICS,
    network::P2PContext,
    peer_table::{PeerTable, PeerTableServerProtocol as _},
    rlpx::{
        Message,
        connection::{codec::RLPxCodec, handshake},
        error::PeerConnectionError,
        eth::{
            block_access_lists::{BlockAccessLists, GetBlockAccessLists},
            blocks::{BlockBodies, BlockHeaders},
            receipts::{
                GetReceipts68, GetReceipts70, Receipts68, Receipts69, Receipts70,
                SOFT_RESPONSE_LIMIT,
            },
            status::{StatusMessage68, StatusMessage69, StatusMessage70, StatusMessage71},
            transactions::{GetPooledTransactions, NewPooledTransactionHashes},
            update::BlockRangeUpdate,
        },
        message::EthCapVersion,
        p2p::{
            self, Capability, DisconnectMessage, DisconnectReason, PingMessage, PongMessage,
            SUPPORTED_ETH_CAPABILITIES, SUPPORTED_SNAP_CAPABILITIES,
        },
        snap::TrieNodes,
    },
    snap::{
        process_account_range_request, process_byte_codes_request, process_storage_ranges_request,
        process_trie_nodes_request,
    },
    tx_broadcaster::{TxBroadcaster, TxBroadcasterProtocol as _, send_tx_hashes},
    types::Node,
};
use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
#[cfg(feature = "l2")]
use ethrex_common::types::Transaction;
use ethrex_common::types::{MempoolTransaction, P2PTransaction, Receipt};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::TrieError;
use futures::{SinkExt as _, Stream, stream::SplitSink};
use rand::random;
use rustc_hash::FxHashMap;
use secp256k1::{PublicKey, SecretKey};
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorRef, ActorStart as _, Context, Handler, send_interval, spawn_listener},
};
use spawned_rt::tasks::BroadcastStream;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tokio::{
    net::TcpStream,
    sync::{broadcast, oneshot},
    task::{self, Id},
};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::{debug, error, trace, warn};

const PING_INTERVAL: Duration = Duration::from_secs(10);
const BLOCK_RANGE_UPDATE_INTERVAL: Duration = Duration::from_secs(60);
const INFLIGHT_TX_SWEEP_INTERVAL: Duration = Duration::from_secs(15);
const INFLIGHT_TX_TIMEOUT: Duration = Duration::from_secs(30);
/// Max time a single outbound frame may take to flush to a peer's socket before we
/// consider the peer wedged and drop it. Without this bound, a slow/half-dead peer
/// (TCP send window full, never draining) blocks the connection actor's serial drain
/// indefinitely while its unbounded mailbox keeps growing — the timer/broadcast
/// producers below enqueue regardless of drain progress — leaking memory that is
/// never reclaimed (the actor cannot be stopped while a handler is wedged in `.await`).
const OUTBOUND_SEND_TIMEOUT: Duration = Duration::from_secs(30);
/// Max time the RLPx handshake may take before we abandon the connection. Without
/// this, an inbound peer that opens TCP and then stalls before/at the auth read
/// parks a connection actor + socket indefinitely (the handshake runs inside
/// `started()` with no timeout), so established sockets accumulate far beyond the
/// peer target.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// If a peer fails to answer this many consecutive pings, consider it dead and drop it.
/// Catches application-dead-but-TCP-alive peers that would otherwise never be removed.
const MAX_MISSED_PONGS: u8 = 3;
/// Capacity of the per-connection bounded outbound queue. The writer task drains this into
/// the socket; if a peer can't keep up the queue fills and the peer is dropped, so outbound
/// buffering per connection is bounded to this many messages instead of growing unbounded.
const OUTBOUND_QUEUE_CAP: usize = 1024;
/// How often to flush buffered transaction hash requests into a single
/// batched GetPooledTransactions message.
const TX_REQUEST_BATCH_INTERVAL: Duration = Duration::from_millis(50);
/// Fixed (tumbling) time window for incoming request rate limiting.
const SERVE_REQUEST_WINDOW: Duration = Duration::from_secs(60);
/// Maximum number of data-serving requests allowed per peer within the rate-limit window.
const MAX_SERVE_REQUESTS_PER_WINDOW: u64 = 500;
/// Number of transactions sent to a peer before checking for leeching behaviour.
const LEECH_TX_SENT_THRESHOLD: u64 = 10_000;

pub(crate) type PeerConnBroadcastSender = broadcast::Sender<(tokio::task::Id, Arc<Message>)>;

#[protocol]
pub trait PeerConnectionServerProtocol: Send + Sync {
    fn incoming_message(&self, message: Message) -> Result<(), ActorError>;
    fn outgoing_message(&self, message: Message) -> Result<(), ActorError>;
    fn outgoing_request(
        &self,
        message: Message,
        sender: Arc<oneshot::Sender<Message>>,
    ) -> Result<(), ActorError>;
    fn request_timeout(&self, id: u64) -> Result<(), ActorError>;
    fn send_ping(&self) -> Result<(), ActorError>;
    fn block_range_update(&self) -> Result<(), ActorError>;
    fn broadcast_message(&self, task_id: Id, msg: Arc<Message>) -> Result<(), ActorError>;
    fn sweep_inflight_txs(&self) -> Result<(), ActorError>;
    fn flush_pending_tx_requests(&self) -> Result<(), ActorError>;
    fn enqueue_tx_requests(
        &self,
        announcement: NewPooledTransactionHashes,
        hashes: Vec<H256>,
    ) -> Result<(), ActorError>;
    /// Inbound RLPx stream failed (codec/decode/IO). The connection must be dropped:
    /// after a `Framed` decoder error the read half is finished and the peer can no
    /// longer answer requests.
    fn inbound_stream_failed(
        &self,
        reason: DisconnectReason,
        detail: String,
    ) -> Result<(), ActorError>;
}

#[cfg(feature = "l2")]
#[derive(Clone)]
pub struct L2Message {
    pub msg: L2Cast,
}

#[cfg(feature = "l2")]
impl spawned_concurrency::message::Message for L2Message {
    type Result = ();
}

#[derive(Clone, Debug)]
pub struct PeerConnection {
    handle: ActorRef<PeerConnectionServer>,
}

impl PeerConnection {
    pub fn spawn_as_receiver(
        context: P2PContext,
        peer_addr: SocketAddr,
        stream: TcpStream,
        admission_permit: tokio::sync::OwnedSemaphorePermit,
    ) -> PeerConnection {
        let state = ConnectionState::Receiver(Receiver {
            context,
            peer_addr,
            stream: Arc::new(stream),
        });
        let connection = PeerConnectionServer {
            state,
            _admission_permit: Some(admission_permit),
        };
        Self {
            handle: connection.start(),
        }
    }

    pub fn spawn_as_initiator(context: P2PContext, node: &Node) -> PeerConnection {
        let state = ConnectionState::Initiator(Initiator {
            context,
            node: node.clone(),
        });
        // Outbound dials are not admission-capped (we initiate them); inbound is the attack surface.
        let connection = PeerConnectionServer {
            state,
            _admission_permit: None,
        };
        Self {
            handle: connection.start(),
        }
    }

    pub async fn outgoing_message(&mut self, message: Message) -> Result<(), PeerConnectionError> {
        self.handle
            .outgoing_message(message)
            .map_err(|err| PeerConnectionError::InternalError(err.to_string()))
    }

    /// Queue tx hashes (with the originating announcement metadata) to be
    /// requested on the next flush tick. Used as a fallback when an in-flight
    /// request on another peer fails.
    pub fn enqueue_tx_requests(
        &self,
        announcement: NewPooledTransactionHashes,
        hashes: Vec<H256>,
    ) -> Result<(), PeerConnectionError> {
        self.handle
            .enqueue_tx_requests(announcement, hashes)
            .map_err(|err| PeerConnectionError::InternalError(err.to_string()))
    }

    pub async fn outgoing_request(
        &mut self,
        message: Message,
        timeout: Duration,
    ) -> Result<Message, PeerConnectionError> {
        let id = message
            .request_id()
            .expect("Cannot wait on request without id");
        let (oneshot_tx, oneshot_rx) = oneshot::channel::<Message>();

        self.handle
            .outgoing_request(message, Arc::new(oneshot_tx))
            .map_err(|err| PeerConnectionError::InternalError(err.to_string()))?;

        // Wait for the response or timeout. This blocks the calling task (and not the ConnectionServer task)
        match tokio::time::timeout(timeout, oneshot_rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(PeerConnectionError::RecvError(error.to_string())),
            Err(_timeout) => {
                // Notify timeout on request id
                self.handle
                    .request_timeout(id)
                    .map_err(|err| PeerConnectionError::InternalError(err.to_string()))?;
                // Return timeout error
                Err(PeerConnectionError::Timeout)
            }
        }
    }
}

#[derive(Debug)]
pub struct Initiator {
    pub(crate) context: P2PContext,
    pub(crate) node: Node,
}

#[derive(Debug)]
pub struct Receiver {
    pub(crate) context: P2PContext,
    pub(crate) peer_addr: SocketAddr,
    pub(crate) stream: Arc<TcpStream>,
}

#[derive(Debug)]
pub struct Established {
    pub(crate) signer: SecretKey,
    // Bounded outbound queue to the per-connection writer task (which owns the socket sink).
    // A bounded queue + dropping the peer on overflow keeps per-connection outbound buffering
    // bounded instead of letting the actor mailbox grow when a peer can't keep up. The receiving
    // part of the TcpStream is owned by the stream listen loop task. See `spawn_outbound_writer`.
    pub(crate) outbound_tx: tokio::sync::mpsc::Sender<Message>,
    /// Set by the writer task when it exits because a single send exceeded `OUTBOUND_SEND_TIMEOUT`
    /// (a wedged peer), so `send` can surface `OutboundSendTimeout` once the bounded queue closes.
    pub(crate) outbound_writer_timed_out: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) node: Node,
    // Whether the remote peer initiated this connection (we acted as Receiver).
    // `false` when we initiated it (we acted as Initiator).
    pub(crate) is_inbound: bool,
    pub(crate) storage: Store,
    pub(crate) blockchain: Arc<Blockchain>,
    pub(crate) capabilities: Vec<Capability>,
    pub(crate) negotiated_eth_capability: Option<Capability>,
    pub(crate) negotiated_snap_capability: Option<Capability>,
    pub(crate) last_block_range_update_block: u64,
    /// Maps request ID to (original announcement, actually requested hashes, request time).
    /// The announcement is kept for response validation; the hashes track in-flight state.
    pub(crate) requested_pooled_txs: HashMap<u64, (NewPooledTransactionHashes, Vec<H256>, Instant)>,
    /// Buffered transaction requests waiting to be flushed as a single batch.
    /// Accumulated between flush ticks (TX_REQUEST_BATCH_INTERVAL).
    pub(crate) pending_tx_requests: Vec<(NewPooledTransactionHashes, Vec<H256>)>,
    pub(crate) client_version: String,
    //// Send end of the channel used to broadcast messages
    //// to other connected peers, is ok to have it here,
    //// since internally it's an Arc.
    //// The ID is to ignore the message sent from the same task.
    //// This is used both to send messages and to received broadcasted
    //// messages from other connections (sent from other peers).
    //// The receive end is instantiated after the handshake is completed
    //// under `handle_peer`.
    /// TODO: Improve this mechanism
    /// See https://github.com/lambdaclass/ethrex/issues/3388
    pub(crate) connection_broadcast_send: PeerConnBroadcastSender,
    pub(crate) peer_table: PeerTable,
    #[cfg(feature = "l2")]
    pub(crate) l2_state: L2ConnState,
    pub(crate) tx_broadcaster: ActorRef<TxBroadcaster>,
    pub(crate) current_requests: HashMap<u64, (String, oneshot::Sender<Message>)>,
    // We store the disconnection reason to handle it in the teardown
    pub(crate) disconnect_reason: Option<DisconnectReason>,
    // Indicates if the peer has been validated (ie. the connection was established successfully)
    pub(crate) is_validated: bool,
    // Rate limiting: start of the current incoming-request window
    pub(crate) serve_request_window_start: Instant,
    // Rate limiting: number of data-serving requests received in the current window
    pub(crate) serve_requests_in_window: u64,
    // Leech detection: total transactions sent to this peer via GetPooledTransactions responses
    pub(crate) txs_sent_to_peer: u64,
    // Leech detection: whether we have received any transactions from this peer
    pub(crate) received_txs_from_peer: bool,
    // Liveness: consecutive pings sent without a matching pong. Reset to 0 on every Pong,
    // incremented on each Ping we send; at MAX_MISSED_PONGS the peer is considered dead
    // and dropped (catches application-dead-but-TCP-alive peers that otherwise linger).
    pub(crate) missed_pongs: u8,
}

impl Established {
    async fn teardown(&mut self) {
        // Clear any in-flight transaction hashes so other connections can re-request them,
        // then try to re-issue each pending request to an alternate announcer.
        // Order matters: clear first so the alternate's reserve_unknown_hashes sees the
        // hashes as free; otherwise the actor handler can race with clear_in_flight_txs
        // and silently no-op the retry while consuming an alternates slot.
        for (_, (_announced, requested_hashes, _)) in self.requested_pooled_txs.drain() {
            if let Err(e) = self
                .blockchain
                .mempool
                .clear_in_flight_txs(&requested_hashes)
            {
                warn!(error = %e, "Failed to clear in-flight transaction tracking during peer teardown");
            }
            retry_on_alternates(&self.blockchain, &self.peer_table, &requested_hashes).await;
        }
        // Also clear hashes that were buffered but not yet sent.
        for (_announced, pending_hashes) in self.pending_tx_requests.drain(..) {
            if let Err(e) = self.blockchain.mempool.clear_in_flight_txs(&pending_hashes) {
                warn!(error = %e, "Failed to clear in-flight transaction tracking during peer teardown");
            }
            retry_on_alternates(&self.blockchain, &self.peer_table, &pending_hashes).await;
        }
        // The socket sink is owned by the per-connection writer task (`spawn_outbound_writer`),
        // which closes it when this `Established` is dropped (its `outbound_tx` is the only sender).
    }
}

#[derive(Debug)]
pub enum ConnectionState {
    HandshakeFailed,
    Initiator(Initiator),
    Receiver(Receiver),
    Established(Box<Established>),
}

#[derive(Debug)]
pub struct PeerConnectionServer {
    state: ConnectionState,
    /// Inbound-admission permit (Some for inbound connections, None for outbound dials we
    /// initiate). Held for the actor's lifetime and released when the actor is dropped, so the
    /// inbound connection count stays bounded. See `P2PContext::inbound_admission`.
    _admission_permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

#[actor(protocol = PeerConnectionServerProtocol)]
impl PeerConnectionServer {
    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        // Set a default eth version that we can update after we negotiate peer capabilities
        // This eth version will only be used to encode & decode the initial `Hello` messages.
        let eth_version = Arc::new(RwLock::new(EthCapVersion::default()));
        // Take ownership of the state, replacing with HandshakeFailed as placeholder
        let state = std::mem::replace(&mut self.state, ConnectionState::HandshakeFailed);
        // Bound the handshake: a peer that opens TCP and then stalls must not park this
        // actor + socket forever (otherwise established sockets accumulate above the peer
        // target). On timeout the handshake future is dropped, closing the socket.
        let handshake_result = match tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            handshake::perform(state, eth_version.clone()),
        )
        .await
        {
            Ok(result) => result,
            Err(_elapsed) => {
                debug!("Handshake timed out on RLPx connection");
                self.state = ConnectionState::HandshakeFailed;
                ctx.stop();
                return;
            }
        };
        match handshake_result {
            Ok((mut established_state, stream)) => {
                trace!(peer=%established_state.node, "Starting RLPx connection");
                if let Err(reason) =
                    initialize_connection(ctx, &mut established_state, stream, eth_version).await
                {
                    match &reason {
                        PeerConnectionError::NoMatchingCapabilities
                        | PeerConnectionError::HandshakeError(_) => {
                            if let Err(e) = established_state
                                .peer_table
                                .set_unwanted(established_state.node.node_id())
                            {
                                debug!("Failed to set peer as unwanted: {e}");
                            }
                        }
                        _ => {}
                    }
                    connection_failed(
                        &mut established_state,
                        "Failed to initialize RLPx connection",
                        &reason,
                    )
                    .await;

                    METRICS.record_new_rlpx_conn_failure(reason).await;

                    self.state = ConnectionState::Established(Box::new(established_state));
                    ctx.stop();
                } else {
                    METRICS
                        .record_new_rlpx_conn_established(
                            &established_state
                                .node
                                .version
                                .clone()
                                .unwrap_or("Unknown".to_string()),
                        )
                        .await;
                    established_state.is_validated = true;
                    // New state
                    self.state = ConnectionState::Established(Box::new(established_state));
                }
            }
            Err(err) => {
                // Handshake failed, just log a debug message.
                // No connection was established so no need to perform any other action
                debug!("Failed Handshake on RLPx connection {err}");
                self.state = ConnectionState::HandshakeFailed;
                ctx.stop();
            }
        }
    }

    #[stopped]
    async fn stopped(&mut self, _ctx: &Context<Self>) {
        match std::mem::replace(&mut self.state, ConnectionState::HandshakeFailed) {
            ConnectionState::Established(mut established_state) => {
                trace!(peer=%established_state.node, "Closing connection with established peer");
                if established_state.is_validated {
                    // If its validated the peer was connected, so we record the disconnection.
                    let reason = established_state
                        .disconnect_reason
                        .unwrap_or(DisconnectReason::NetworkError);
                    METRICS
                        .record_new_rlpx_conn_disconnection(
                            &established_state
                                .node
                                .version
                                .clone()
                                .unwrap_or("Unknown".to_string()),
                            reason,
                        )
                        .await;
                }
                if let Err(e) = established_state
                    .peer_table
                    .remove_peer(established_state.node.node_id())
                {
                    debug!("Failed to remove peer from table: {e}");
                }
                // Free the peer's tx-broadcaster index (and clear its bit across known txs) so
                // the broadcaster's per-peer index map / PeerMask widths stay bounded to live peers.
                if let Err(e) = established_state
                    .tx_broadcaster
                    .remove_peer(established_state.node.node_id())
                {
                    debug!("Failed to remove peer from tx broadcaster: {e}");
                }
                established_state.teardown().await;
            }
            _ => {
                // Nothing to do if the connection was not established
            }
        };
    }

    #[send_handler]
    async fn handle_incoming_message(
        &mut self,
        msg: peer_connection_server_protocol::IncomingMessage,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            trace!(
                peer=%established_state.node,
                message=%msg.message,
                "Received incoming message",
            );
            let result = handle_incoming_message(established_state, msg.message).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_outgoing_message(
        &mut self,
        msg: peer_connection_server_protocol::OutgoingMessage,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            trace!(
                peer=%established_state.node,
                message=%msg.message,
                "Received outgoing request",
            );
            let result = handle_outgoing_message(established_state, msg.message).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_outgoing_request(
        &mut self,
        msg: peer_connection_server_protocol::OutgoingRequest,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            trace!(
                peer=%established_state.node,
                message=%msg.message,
                "Received outgoing request",
            );
            let Some(sender) = Arc::<oneshot::Sender<Message>>::into_inner(msg.sender) else {
                debug!("Could not obtain sender channel: Arc has multiple references");
                return;
            };
            let result = handle_outgoing_request(established_state, msg.message, sender).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_request_timeout(
        &mut self,
        msg: peer_connection_server_protocol::RequestTimeout,
        _ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            // Discard the request from current requests
            if let Some((msg_type, _)) = established_state.current_requests.remove(&msg.id) {
                debug!(
                    peer=%established_state.node,
                    %msg_type,
                    id=%msg.id,
                    "Request timedout",
                );
            }
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_send_ping(
        &mut self,
        _msg: peer_connection_server_protocol::SendPing,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            // Liveness: if the peer hasn't answered the last MAX_MISSED_PONGS pings, treat it
            // as dead and drop it instead of pinging a corpse (and keeping its actor) forever.
            if established_state.missed_pongs >= MAX_MISSED_PONGS {
                debug!(peer=%established_state.node, missed=MAX_MISSED_PONGS, "Peer missed max pongs, dropping");
                // Attribute the drop as `PingTimeout` so an unresponsive-peer drop is
                // distinguishable from a generic `NetworkError` in `ethrex_p2p_disconnections`.
                established_state.disconnect_reason = Some(DisconnectReason::PingTimeout);
                ctx.stop();
                return;
            }
            established_state.missed_pongs = established_state.missed_pongs.saturating_add(1);
            let result = send(established_state, Message::Ping(PingMessage {})).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_block_range_update(
        &mut self,
        _msg: peer_connection_server_protocol::BlockRangeUpdate,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            trace!(
                peer=%established_state.node,
                "Block Range Update"
            );
            let result = handle_block_range_update(established_state).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_sweep_inflight_txs(
        &mut self,
        _msg: peer_connection_server_protocol::SweepInflightTxs,
        _ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut state) = self.state {
            let now = Instant::now();
            let stale_ids: Vec<u64> = state
                .requested_pooled_txs
                .iter()
                .filter(|(_, (_, _, ts))| now.duration_since(*ts) > INFLIGHT_TX_TIMEOUT)
                .map(|(id, _)| *id)
                .collect();
            for id in stale_ids {
                if let Some((_announced, hashes, _)) = state.requested_pooled_txs.remove(&id) {
                    // Clear in-flight before retry so the alternate's reserve_unknown_hashes
                    // doesn't race against still-in-flight state and silently no-op.
                    if let Err(e) = state.blockchain.mempool.clear_in_flight_txs(&hashes) {
                        warn!(error = %e, "Failed to clear in-flight transaction tracking while sweeping stale requests");
                    }
                    retry_on_alternates(&state.blockchain, &state.peer_table, &hashes).await;
                }
            }
        }
    }

    #[send_handler]
    async fn handle_flush_pending_tx_requests(
        &mut self,
        _msg: peer_connection_server_protocol::FlushPendingTxRequests,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            let result = flush_pending_tx_requests(established_state).await;
            Self::process_cast_error(&self.state, result, ctx);
        }
    }

    #[send_handler]
    async fn handle_enqueue_tx_requests(
        &mut self,
        msg: peer_connection_server_protocol::EnqueueTxRequests,
        _ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut state) = self.state {
            // Re-reserve in-flight against this peer. If any hashes are already
            // in-flight (race), drop them; we don't want duplicate requests.
            let to_request: Vec<H256> = match state.blockchain.mempool.reserve_unknown_hashes(
                &msg.announcement.transaction_hashes,
                &msg.announcement.transaction_types,
                &msg.announcement.transaction_sizes,
                state.node.node_id(),
            ) {
                Ok(unknown) => unknown,
                Err(_) => return,
            };
            if to_request.is_empty() {
                return;
            }
            let trimmed = msg.announcement.filter_to(&to_request);
            state.pending_tx_requests.push((trimmed, to_request));
        }
    }

    #[send_handler]
    async fn handle_broadcast_message(
        &mut self,
        msg: peer_connection_server_protocol::BroadcastMessage,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            trace!(
                peer=%established_state.node,
                message=%msg.msg,
                "Received broadcasted message",
            );
            let result = handle_broadcast(established_state, (msg.task_id, msg.msg)).await;
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[cfg(feature = "l2")]
    #[send_handler]
    async fn handle_l2_message(&mut self, msg: L2Message, ctx: &Context<Self>) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            let peer_supports_l2 = established_state.l2_state.connection_state().is_ok();
            let result = if peer_supports_l2 {
                trace!(
                    peer=%established_state.node,
                    message=?msg.msg,
                    "Handling cast for L2 msg"
                );
                match msg.msg {
                    L2Cast::BatchBroadcast => {
                        let res = l2_connection::send_sealed_batch(established_state).await;
                        res.and(l2_connection::process_batches_on_queue(established_state).await)
                    }
                    L2Cast::BlockBroadcast => {
                        let res = l2_connection::send_new_block(established_state).await;
                        res.and(l2_connection::process_blocks_on_queue(established_state).await)
                    }
                }
            } else {
                Err(PeerConnectionError::MessageNotHandled(
                    "Unknown message or capability not handled".to_string(),
                ))
            };
            Self::process_cast_error(&self.state, result, ctx);
        } else {
            debug!("Connection not yet established");
        }
    }

    #[send_handler]
    async fn handle_inbound_stream_failed(
        &mut self,
        msg: peer_connection_server_protocol::InboundStreamFailed,
        ctx: &Context<Self>,
    ) {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            apply_inbound_stream_failure(established_state, msg.reason, &msg.detail).await;
        } else {
            debug!(
                reason=?msg.reason,
                detail=%msg.detail,
                "Inbound RLPx stream failed while connection was not Established",
            );
        }
        // Always stop: inbound is unusable and `stopped()` runs remove_peer when Established.
        ctx.stop();
    }

    fn process_cast_error(
        state: &ConnectionState,
        result: Result<(), PeerConnectionError>,
        ctx: &Context<Self>,
    ) {
        if let Err(e) = result
            && let ConnectionState::Established(established_state) = state
        {
            match e {
                PeerConnectionError::Disconnected
                | PeerConnectionError::DisconnectReceived(_)
                | PeerConnectionError::DisconnectSent(_)
                | PeerConnectionError::HandshakeError(_)
                | PeerConnectionError::NoMatchingCapabilities
                | PeerConnectionError::InvalidPeerId
                | PeerConnectionError::InvalidMessageLength
                | PeerConnectionError::StateError(_)
                | PeerConnectionError::InvalidRecoveryId
                | PeerConnectionError::OutboundSendTimeout
                | PeerConnectionError::OutboundQueueFull => {
                    trace!(peer=%established_state.node, error=e.to_string(), "Peer connection error");
                    ctx.stop();
                }
                PeerConnectionError::IoError(ref io_e)
                    if io_e.kind() == std::io::ErrorKind::BrokenPipe =>
                {
                    // TODO: we need to check if this message is ocurring commonly due to a problem
                    // with our concurrency model
                    debug!(peer=%established_state.node, "Broken pipe with peer, disconnected");
                    ctx.stop();
                }
                PeerConnectionError::StoreError(StoreError::Trie(TrieError::InconsistentTree(
                    _,
                ))) => {
                    if established_state.blockchain.is_synced() {
                        // If we're responding with inconsistent trie while synced, our trie may be broken
                        // If this error is non sporadic we should investigate
                        error!(
                            peer=%established_state.node,
                            error=%e,
                            "Inconsistent trie while serving peer request; local state may be corrupted",
                        );
                    } else {
                        // If we're not synced, we expect to have inconsistent trie errors
                        trace!(
                            peer=%established_state.node,
                            error=%e,
                            "Error handling cast message",
                        );
                    }
                }
                _ => {
                    // We should check why we're failling to handle the cast message
                    debug!(
                        peer=%established_state.node,
                        capabilities=?established_state.capabilities,
                        error=%e,
                        "Error handling cast message",
                    );
                }
            }
        }
    }
}

async fn initialize_connection<S>(
    ctx: &Context<PeerConnectionServer>,
    state: &mut Established,
    mut stream: S,
    eth_version: Arc<RwLock<EthCapVersion>>,
) -> Result<(), PeerConnectionError>
where
    S: Unpin + Send + Stream<Item = Result<Message, PeerConnectionError>> + 'static,
{
    if state.peer_table.target_peers_reached().await? {
        debug!(peer=%state.node, "Reached target peer connections, discarding.");
        return Err(PeerConnectionError::TooManyPeers);
    }
    exchange_hello_messages(state, &mut stream).await?;

    // Update eth capability version to the negotiated version for further message decoding
    let version = match &state.negotiated_eth_capability {
        Some(cap) if cap == &Capability::eth(68) => EthCapVersion::V68,
        Some(cap) if cap == &Capability::eth(69) => EthCapVersion::V69,
        Some(cap) if cap == &Capability::eth(70) => EthCapVersion::V70,
        Some(cap) if cap == &Capability::eth(71) => EthCapVersion::V71,
        _ => EthCapVersion::default(),
    };
    *eth_version
        .write()
        .map_err(|err| PeerConnectionError::InternalError(err.to_string()))? = version;

    init_capabilities(state, &mut stream).await?;

    let mut connection = PeerConnection {
        handle: ctx.actor_ref(),
    };

    state.peer_table.new_connected_peer(
        state.node.clone(),
        connection.clone(),
        state.capabilities.clone(),
        state.is_inbound,
    )?;

    trace!(peer=%state.node, "Peer connection initialized.");

    // Send transactions transaction hashes from mempool at connection start
    send_all_pooled_tx_hashes(state, &mut connection).await?;

    // Periodic Pings repeated events.
    send_interval(
        PING_INTERVAL,
        ctx.clone(),
        peer_connection_server_protocol::SendPing,
    );

    // Periodic block range update.
    send_interval(
        BLOCK_RANGE_UPDATE_INTERVAL,
        ctx.clone(),
        peer_connection_server_protocol::BlockRangeUpdate,
    );

    // Periodic sweep of stale in-flight transaction requests.
    send_interval(
        INFLIGHT_TX_SWEEP_INTERVAL,
        ctx.clone(),
        peer_connection_server_protocol::SweepInflightTxs,
    );

    // Periodic flush of buffered transaction requests.
    send_interval(
        TX_REQUEST_BATCH_INTERVAL,
        ctx.clone(),
        peer_connection_server_protocol::FlushPendingTxRequests,
    );

    #[cfg(feature = "l2")]
    // Periodic L2 messages events.
    if state.l2_state.connection_state().is_ok() {
        send_interval(
            PERIODIC_BLOCK_BROADCAST_INTERVAL,
            ctx.clone(),
            L2Message {
                msg: L2Cast::BlockBroadcast,
            },
        );
        send_interval(
            PERIODIC_BATCH_BROADCAST_INTERVAL,
            ctx.clone(),
            L2Message {
                msg: L2Cast::BatchBroadcast,
            },
        );
    }

    // Decoder/`Framed` errors are terminal for the read half: the next poll is EOS, so
    // "skipping" is impossible. Notify the actor so it stops promptly instead of leaving
    // a zombie peer selectable until missed-pong timeout.
    // See https://github.com/lambdaclass/ethrex/issues/7035
    let inbound_ctx = ctx.clone();
    spawn_listener(
        ctx.clone(),
        stream.filter_map(move |result| match result {
            Ok(msg) => Some(peer_connection_server_protocol::IncomingMessage { message: msg }),
            Err(e) => {
                let reason = disconnect_reason_for_inbound_error(&e);
                debug!(
                    error=?e,
                    ?reason,
                    "Error receiving RLPx message, notifying connection to drop peer",
                );
                if let Err(send_err) =
                    inbound_ctx.send(peer_connection_server_protocol::InboundStreamFailed {
                        reason,
                        detail: e.to_string(),
                    })
                {
                    debug!(
                        error=?send_err,
                        ?reason,
                        "Failed to notify connection of inbound RLPx stream failure",
                    );
                }
                None
            }
        }),
    );

    if state.negotiated_eth_capability.is_some() {
        let stream: BroadcastStream<(Id, Arc<Message>)> =
            BroadcastStream::new(state.connection_broadcast_send.subscribe());
        let message_stream = stream.filter_map(|result| {
            result.ok().map(
                |(id, msg)| peer_connection_server_protocol::BroadcastMessage { task_id: id, msg },
            )
        });
        spawn_listener(ctx.clone(), message_stream);
    }

    Ok(())
}

async fn send_all_pooled_tx_hashes(
    state: &mut Established,
    connection: &mut PeerConnection,
) -> Result<(), PeerConnectionError> {
    let txs: Vec<MempoolTransaction> = state
        .blockchain
        .mempool
        .get_all_txs_by_sender()?
        .into_values()
        .flatten()
        .filter(|tx| !tx.is_privileged())
        .collect();
    if !txs.is_empty() {
        state
            .tx_broadcaster
            .add_txs(
                txs.iter().map(|tx| tx.hash(&NativeCrypto)).collect(),
                state.node.node_id(),
            )
            .map_err(|e| PeerConnectionError::BroadcastError(e.to_string()))?;
        send_tx_hashes(
            txs,
            state.capabilities.clone(),
            connection,
            state.node.node_id(),
            &state.blockchain,
        )
        .await
        .map_err(|e| PeerConnectionError::SendMessage(e.to_string()))?;
    }
    Ok(())
}

async fn send_block_range_update(state: &mut Established) -> Result<(), PeerConnectionError> {
    // BlockRangeUpdate was introduced in eth/69
    if state
        .negotiated_eth_capability
        .as_ref()
        .is_some_and(|eth| eth.version >= 69)
    {
        trace!(peer=%state.node, "Sending BlockRangeUpdate");
        let update = BlockRangeUpdate::new(&state.storage).await?;
        let lastet_block = update.latest_block;
        send(state, Message::BlockRangeUpdate(update)).await?;
        state.last_block_range_update_block = lastet_block - (lastet_block % 32);
    }
    Ok(())
}

async fn should_send_block_range_update(state: &Established) -> Result<bool, PeerConnectionError> {
    let latest_block = state.storage.get_latest_block_number().await?;
    if latest_block < state.last_block_range_update_block
        || latest_block - state.last_block_range_update_block >= 32
    {
        return Ok(true);
    }
    Ok(false)
}

async fn init_capabilities<S>(
    state: &mut Established,
    stream: &mut S,
) -> Result<(), PeerConnectionError>
where
    S: Unpin + Stream<Item = Result<Message, PeerConnectionError>>,
{
    // Sending eth Status if peer supports it
    if let Some(eth) = state.negotiated_eth_capability.clone() {
        let status = match eth.version {
            68 => Message::Status68(StatusMessage68::new(&state.storage).await?),
            69 => Message::Status69(StatusMessage69::new(&state.storage).await?),
            70 => Message::Status70(StatusMessage70::new(&state.storage).await?),
            71 => Message::Status71(StatusMessage71::new(&state.storage).await?),
            ver => {
                return Err(PeerConnectionError::HandshakeError(format!(
                    "Invalid eth version {ver}"
                )));
            }
        };
        trace!(peer=%state.node, "Sending status");
        send(state, status).await?;
        // The next immediate message in the ETH protocol is the
        // status, reference here:
        // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
        let msg = match receive(stream).await {
            Some(msg) => msg?,
            None => return Err(PeerConnectionError::Disconnected),
        };
        match msg {
            Message::Status68(msg_data) => {
                trace!(peer=%state.node, "Received Status(68)");
                backend::validate_status(msg_data, &state.storage, &eth).await?
            }
            Message::Status69(msg_data) => {
                trace!(peer=%state.node, "Received Status(69)");
                backend::validate_status(msg_data, &state.storage, &eth).await?
            }
            Message::Status70(msg_data) => {
                trace!(peer=%state.node, "Received Status(70)");
                backend::validate_status(msg_data, &state.storage, &eth).await?
            }
            Message::Status71(msg_data) => {
                trace!(peer=%state.node, "Received Status(71)");
                backend::validate_status(msg_data, &state.storage, &eth).await?
            }
            Message::Disconnect(disconnect) => {
                return Err(PeerConnectionError::HandshakeError(format!(
                    "Peer disconnected due to: {}",
                    disconnect.reason()
                )));
            }
            _ => {
                return Err(PeerConnectionError::HandshakeError(
                    "Expected a Status message".to_string(),
                ));
            }
        }
    }
    Ok(())
}

async fn send_disconnect_message(state: &mut Established, reason: Option<DisconnectReason>) {
    send(state, Message::Disconnect(DisconnectMessage { reason }))
        .await
        .unwrap_or_else(|_| {
            debug!(
                peer=%state.node,
                ?reason,
                "Could not send Disconnect message",
            );
        });
}

/// Record an inbound stream failure on an established connection and optionally notify the peer.
/// The caller is responsible for `ctx.stop()` so `stopped()` can run `remove_peer`.
///
/// Extracted so unit tests can cover reason bookkeeping without a full RLPx handshake.
async fn apply_inbound_stream_failure(
    state: &mut Established,
    incoming_reason: DisconnectReason,
    detail: &str,
) {
    debug!(
        peer=%state.node,
        reason=?incoming_reason,
        %detail,
        "Inbound RLPx stream failed, dropping peer",
    );
    // Preserve an earlier reason (e.g. peer-sent Disconnect) if one is already set.
    // Use the stored reason for both metrics and any wire Disconnect so they agree.
    let reason = *state.disconnect_reason.get_or_insert(incoming_reason);
    // Protocol breach: outbound may still be writable, so tell the peer why.
    // Network/IO failures usually mean the socket is already gone.
    if reason == DisconnectReason::ProtocolError {
        send_disconnect_message(state, Some(reason)).await;
    }
}

async fn connection_failed(state: &mut Established, error_text: &str, error: &PeerConnectionError) {
    debug!(
        peer=%state.node,
        %error_text,
        %error,
        "connection failure"
    );

    // Send disconnect message only if error is different than RLPxError::DisconnectRequested
    // because if it is a DisconnectRequested error it means that the peer requested the disconnection, not us.
    if !matches!(error, PeerConnectionError::DisconnectReceived(_)) {
        send_disconnect_message(state, match_disconnect_reason(error)).await;
    }

    // Discard peer from kademlia table in some cases
    match error {
        // already connected, don't discard it
        PeerConnectionError::DisconnectReceived(DisconnectReason::AlreadyConnected)
        | PeerConnectionError::DisconnectSent(DisconnectReason::AlreadyConnected) => {
            debug!(
                peer=%state.node,
                %error_text,
                %error,
                "Peer already connected, don't replace it"
            );
        }
        _ => {
            debug!(
                peer=%state.node,
                %error_text,
                %error,
                remote_public_key=%state.node.public_key,
                "discarding peer",
            );
        }
    }
}

fn match_disconnect_reason(error: &PeerConnectionError) -> Option<DisconnectReason> {
    match error {
        PeerConnectionError::DisconnectSent(reason) => Some(*reason),
        PeerConnectionError::DisconnectReceived(reason) => Some(*reason),
        PeerConnectionError::RLPDecodeError(_) => Some(DisconnectReason::NetworkError),
        PeerConnectionError::TooManyPeers => Some(DisconnectReason::TooManyPeers),
        // TODO build a proper matching between error types and disconnection reasons
        _ => None,
    }
}

/// Map an inbound `Framed`/codec error to the disconnect reason we attribute when
/// dropping the peer. Used for the established-connection read-listener path;
/// distinct from [`match_disconnect_reason`], which is for handshake/`connection_failed`.
///
/// Match is exhaustive so a new [`PeerConnectionError`] variant forces an explicit
/// classification instead of silently becoming the benign [`DisconnectReason::NetworkError`].
fn disconnect_reason_for_inbound_error(error: &PeerConnectionError) -> DisconnectReason {
    match error {
        // Wire/format failures from `RLPxCodec::decode` / message RLP decode.
        PeerConnectionError::RLPDecodeError(_)
        | PeerConnectionError::InvalidMessageFrame(_)
        | PeerConnectionError::CryptographyError(_)
        | PeerConnectionError::InvalidMessageLength => DisconnectReason::ProtocolError,

        // Transport errors, local/internal failures, and variants not expected from the
        // inbound Framed path. Listed explicitly (fail-closed) rather than `_`.
        PeerConnectionError::IoError(_)
        | PeerConnectionError::InternalError(_)
        | PeerConnectionError::Disconnected
        | PeerConnectionError::HandshakeError(_)
        | PeerConnectionError::StateError(_)
        | PeerConnectionError::NoMatchingCapabilities
        | PeerConnectionError::TooManyPeers
        | PeerConnectionError::OutboundSendTimeout
        | PeerConnectionError::OutboundQueueFull
        | PeerConnectionError::DisconnectReceived(_)
        | PeerConnectionError::DisconnectSent(_)
        | PeerConnectionError::NotFound(_)
        | PeerConnectionError::InvalidPeerId
        | PeerConnectionError::InvalidRecoveryId
        | PeerConnectionError::ExpectedRequestId(_)
        | PeerConnectionError::MessageNotHandled(_)
        | PeerConnectionError::BadRequest(_)
        | PeerConnectionError::RLPEncodeError(_)
        | PeerConnectionError::StoreError(_)
        | PeerConnectionError::BroadcastError(_)
        | PeerConnectionError::RecvError(_)
        | PeerConnectionError::SendMessage(_)
        | PeerConnectionError::MempoolError(_)
        | PeerConnectionError::BlockchainError(_)
        | PeerConnectionError::IncompatibleProtocol
        | PeerConnectionError::InvalidBlockRange
        | PeerConnectionError::L2CapabilityNotNegotiated
        | PeerConnectionError::InvalidBlockRangeUpdate
        | PeerConnectionError::ActorError(_)
        | PeerConnectionError::Timeout
        | PeerConnectionError::UnexpectedResponse(_, _) => DisconnectReason::NetworkError,
        #[cfg(feature = "l2")]
        PeerConnectionError::RollupStoreError(_) => DisconnectReason::NetworkError,
    }
}

async fn exchange_hello_messages<S>(
    state: &mut Established,
    stream: &mut S,
) -> Result<(), PeerConnectionError>
where
    S: Unpin + Stream<Item = Result<Message, PeerConnectionError>>,
{
    // This allow is because in l2 we mut the capabilities
    // to include the l2 cap
    #[allow(unused_mut)]
    let mut supported_capabilities: Vec<Capability> = [
        &SUPPORTED_ETH_CAPABILITIES[..],
        &SUPPORTED_SNAP_CAPABILITIES[..],
    ]
    .concat();
    #[cfg(feature = "l2")]
    if state.l2_state.is_supported() {
        supported_capabilities.push(crate::rlpx::l2::SUPPORTED_BASED_CAPABILITIES[0].clone());
    }
    let hello_msg = Message::Hello(p2p::HelloMessage::new(
        supported_capabilities,
        PublicKey::from_secret_key(secp256k1::SECP256K1, &state.signer),
        state.client_version.clone(),
    ));

    send(state, hello_msg).await?;

    // Receive Hello message
    let msg = match receive(stream).await {
        Some(msg) => msg?,
        None => return Err(PeerConnectionError::Disconnected),
    };

    match msg {
        Message::Hello(hello_message) => {
            let mut negotiated_eth_version = 0;
            let mut negotiated_snap_version = 0;

            trace!(
                peer=%state.node,
                capabilities=?hello_message.capabilities,
                "Hello message capabilities",
            );

            // Check if we have any capability in common and store the highest version
            for cap in &hello_message.capabilities {
                match cap.protocol() {
                    "eth" => {
                        if SUPPORTED_ETH_CAPABILITIES.contains(cap)
                            && cap.version > negotiated_eth_version
                        {
                            negotiated_eth_version = cap.version;
                        }
                    }
                    "snap" => {
                        if SUPPORTED_SNAP_CAPABILITIES.contains(cap)
                            && cap.version > negotiated_snap_version
                        {
                            negotiated_snap_version = cap.version;
                        }
                    }
                    #[cfg(feature = "l2")]
                    "based" if state.l2_state.is_supported() => {
                        state.l2_state.set_established()?;
                    }
                    _ => {}
                }
            }

            state.capabilities = hello_message.capabilities;

            if negotiated_eth_version == 0 {
                return Err(PeerConnectionError::NoMatchingCapabilities);
            }
            debug!("Negotiated eth version: eth/{}", negotiated_eth_version);
            state.negotiated_eth_capability = Some(Capability::eth(negotiated_eth_version));

            if negotiated_snap_version != 0 {
                debug!("Negotiated snap version: snap/{}", negotiated_snap_version);
                state.negotiated_snap_capability = Some(Capability::snap(negotiated_snap_version));
            }

            state.node.version = Some(hello_message.client_id);

            Ok(())
        }
        Message::Disconnect(disconnect) => {
            Err(PeerConnectionError::DisconnectReceived(disconnect.reason()))
        }
        _ => {
            // Fail if it is not a hello message
            Err(PeerConnectionError::BadRequest(
                "Expected Hello message".to_string(),
            ))
        }
    }
}

pub(crate) async fn send(
    state: &mut Established,
    message: Message,
) -> Result<(), PeerConnectionError> {
    #[cfg(feature = "metrics")]
    {
        use ethrex_metrics::p2p::METRICS_P2P;
        METRICS_P2P.inc_outgoing_message(message.metric_label());
    }
    // Hand off to the per-connection writer task via a BOUNDED queue (non-blocking). This
    // decouples the actor's serial drain from the network write, so a slow/wedged peer can
    // never back up the (unbounded) actor mailbox. If the bounded queue is full the peer
    // can't keep up: surface OutboundQueueFull so `process_cast_error` drops it.
    match state.outbound_tx.try_send(message) {
        Ok(()) => Ok(()),
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            // The peer can't drain our outbound fast enough. Attribute the drop as `UselessPeer`
            // so it is distinguishable from a generic `NetworkError` in `ethrex_p2p_disconnections`
            // (a spike here flags slow peers or our own over-sending, rather than hiding it).
            state
                .disconnect_reason
                .get_or_insert(DisconnectReason::UselessPeer);
            Err(PeerConnectionError::OutboundQueueFull)
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            // The writer task is gone. If it exited because a single send exceeded
            // `OUTBOUND_SEND_TIMEOUT`, the peer was wedged: surface `OutboundSendTimeout` and
            // attribute it likewise; otherwise the socket simply closed (`Disconnected`).
            if state
                .outbound_writer_timed_out
                .load(std::sync::atomic::Ordering::Acquire)
            {
                state
                    .disconnect_reason
                    .get_or_insert(DisconnectReason::UselessPeer);
                Err(PeerConnectionError::OutboundSendTimeout)
            } else {
                Err(PeerConnectionError::Disconnected)
            }
        }
    }
}

/// Spawns the per-connection writer task that owns the `sink` and drains a bounded outbound
/// queue into it. Keeping the network write off the actor thread means the actor's mailbox is
/// never gated on a slow peer; the only outbound buffer is this bounded channel (capacity
/// `OUTBOUND_QUEUE_CAP`). The task exits — closing the socket — when the connection is dropped
/// (all senders gone), when a send errors, or when a single send exceeds `OUTBOUND_SEND_TIMEOUT`
/// (a wedged peer). Once it exits, further `send()`s observe a closed queue and the peer is dropped.
pub(crate) fn spawn_outbound_writer(
    mut sink: SplitSink<Framed<TcpStream, RLPxCodec>, Message>,
) -> (
    tokio::sync::mpsc::Sender<Message>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(OUTBOUND_QUEUE_CAP);
    // Set when the writer exits because a single send exceeded `OUTBOUND_SEND_TIMEOUT` (a wedged
    // peer), so `send` can surface `OutboundSendTimeout` instead of a generic `Disconnected` once
    // the bounded queue closes.
    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer_timed_out = timed_out.clone();
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            match tokio::time::timeout(OUTBOUND_SEND_TIMEOUT, sink.send(message)).await {
                Ok(Ok(())) => {}
                // Send error: the socket is gone; let the closed queue surface as `Disconnected`.
                Ok(Err(_)) => break,
                // The send itself timed out: the peer is wedged. Flag it so the drop is attributed.
                Err(_) => {
                    writer_timed_out.store(true, std::sync::atomic::Ordering::Release);
                    break;
                }
            }
        }
        let _ = sink.close().await;
    });
    (tx, timed_out)
}

/// Reads from the frame until a frame is available.
///
/// Returns `None` when the stream buffer is 0. This could indicate that the client has disconnected,
/// but we cannot safely assume an EOF, as per the Tokio documentation.
///
/// If the handshake has not been established, it is reasonable to terminate the connection.
///
/// For an established connection, [`check_periodic_task`] will detect actual disconnections
/// while sending pings and you should not assume a disconnection.
///
/// See [`Framed::new`] for more details.
async fn receive<S>(stream: &mut S) -> Option<Result<Message, PeerConnectionError>>
where
    S: Unpin + Stream<Item = Result<Message, PeerConnectionError>>,
{
    stream.next().await
}

/// Returns true if the peer is within its rate limit for data-serving requests, false if exceeded.
/// Resets the window counter when the window duration has elapsed.
fn check_serve_request_rate(state: &mut Established) -> bool {
    let now = Instant::now();
    if now.duration_since(state.serve_request_window_start) >= SERVE_REQUEST_WINDOW {
        state.serve_request_window_start = now;
        state.serve_requests_in_window = 0;
    }
    state.serve_requests_in_window += 1;
    state.serve_requests_in_window <= MAX_SERVE_REQUESTS_PER_WINDOW
}

async fn handle_incoming_message(
    state: &mut Established,
    message: Message,
) -> Result<(), PeerConnectionError> {
    #[cfg(feature = "metrics")]
    {
        use ethrex_metrics::p2p::METRICS_P2P;
        METRICS_P2P.inc_incoming_message(message.metric_label());
    }

    // Rate-limit incoming data-serving requests to prevent resource exhaustion.
    let is_data_request = matches!(
        message,
        Message::GetBlockHeaders(_)
            | Message::GetBlockBodies(_)
            | Message::GetReceipts68(_)
            | Message::GetReceipts69(_)
            | Message::GetReceipts70(_)
            | Message::GetPooledTransactions(_)
            | Message::GetAccountRange(_)
            | Message::GetStorageRanges(_)
            | Message::GetByteCodes(_)
            | Message::GetTrieNodes(_)
    );
    if is_data_request && !check_serve_request_rate(state) {
        debug!(
            peer = %state.node,
            window_requests = state.serve_requests_in_window,
            "Disconnecting peer: exceeded incoming request rate limit",
        );
        send_disconnect_message(state, Some(DisconnectReason::UselessPeer)).await;
        return Err(PeerConnectionError::DisconnectSent(
            DisconnectReason::UselessPeer,
        ));
    }

    let peer_supports_eth = state.negotiated_eth_capability.is_some();
    #[cfg(feature = "l2")]
    let peer_supports_l2 = state.l2_state.connection_state().is_ok();
    match message {
        Message::Disconnect(msg_data) => {
            let reason = msg_data.reason();
            trace!(
                peer=%state.node,
                ?reason,
                "Received Disconnect"
            );
            state.disconnect_reason = Some(reason);

            // TODO handle the disconnection request

            return Err(PeerConnectionError::DisconnectReceived(reason));
        }
        Message::Ping(_) => {
            trace!(peer=%state.node, "Sending pong message");
            send(state, Message::Pong(PongMessage {})).await?;
        }
        Message::Pong(_) => {
            // Liveness: the peer answered our ping; reset the missed-pong counter.
            state.missed_pongs = 0;
        }
        Message::Status68(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                backend::validate_status(msg_data, &state.storage, eth).await?
            };
        }
        Message::Status69(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                backend::validate_status(msg_data, &state.storage, eth).await?
            };
        }
        Message::Status70(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                backend::validate_status(msg_data, &state.storage, eth).await?
            };
        }
        Message::Status71(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                backend::validate_status(msg_data, &state.storage, eth).await?
            };
        }
        Message::GetAccountRange(req) => {
            let response = process_account_range_request(req, state.storage.clone()).await?;
            send(state, Message::AccountRange(response)).await?
        }
        Message::Transactions(txs) if peer_supports_eth => {
            // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#transactions-0x02
            if !txs.transactions.is_empty() {
                state.received_txs_from_peer = true;
            }
            if state.blockchain.is_synced() {
                let tx_hashes: Vec<_> = txs
                    .transactions
                    .iter()
                    .map(|tx| tx.hash(&NativeCrypto))
                    .collect();

                // Offload pool insertion to a background task so we don't block
                // the ConnectionServer (validation + signature recovery are expensive).
                let blockchain = state.blockchain.clone();
                let peer = state.node.to_string();
                #[cfg(feature = "l2")]
                let is_l2_mode = state.l2_state.is_supported();
                tokio::spawn(async move {
                    for tx in txs.transactions {
                        #[cfg(feature = "l2")]
                        if (is_l2_mode && matches!(tx, Transaction::EIP4844Transaction(_)))
                            || tx.is_privileged()
                        {
                            let tx_type = tx.tx_type();
                            debug!(peer=%peer, "Rejecting transaction in L2 mode - {tx_type} transactions are not broadcasted in L2");
                            continue;
                        }

                        if let Err(e) = blockchain.add_transaction_to_pool(tx).await {
                            debug!(
                                peer=%peer,
                                error=%e,
                                "Error adding transaction"
                            );
                        }
                    }
                });

                // Notify the broadcaster immediately — it only tracks hashes
                // to avoid re-broadcasting to the sender. The actual broadcast
                // happens on a periodic timer that queries the mempool directly.
                state
                    .tx_broadcaster
                    .add_txs(tx_hashes, state.node.node_id())
                    .map_err(|e| PeerConnectionError::BroadcastError(e.to_string()))?;
            }
        }
        Message::GetBlockHeaders(msg_data) if peer_supports_eth => {
            let response = BlockHeaders {
                id: msg_data.id,
                block_headers: msg_data.fetch_headers(&state.storage).await,
            };
            send(state, Message::BlockHeaders(response)).await?;
        }
        Message::GetBlockBodies(msg_data) if peer_supports_eth => {
            let response = BlockBodies {
                id: msg_data.id,
                block_bodies: msg_data.fetch_blocks(&state.storage).await,
            };
            send(state, Message::BlockBodies(response)).await?;
        }
        Message::GetBlockAccessLists(GetBlockAccessLists { id, block_hashes })
            if peer_supports_eth =>
        {
            use crate::rlpx::eth::block_access_lists::BLOCK_ACCESS_LIST_LIMIT;
            let mut block_access_lists =
                Vec::with_capacity(block_hashes.len().min(BLOCK_ACCESS_LIST_LIMIT));
            for hash in &block_hashes {
                // EIP-8159: only serve a BAL that matches the block header's
                // commitment. A stored BAL that doesn't hash to the header's
                // `block_access_list_hash` (e.g. a stale/empty entry from a prior
                // regeneration) must be reported as unavailable (`0x80`) rather
                // than served as a wrong BAL, which receiving peers would reject.
                let bal = match state.storage.get_block_access_list(*hash) {
                    Ok(Some(bal)) => {
                        let commitment = match state.storage.get_block_header_by_hash(*hash) {
                            Ok(Some(header)) => header.block_access_list_hash,
                            Ok(None) => None,
                            Err(err) => {
                                // Don't serve an unverified BAL: degrade to 0x80
                                // (unavailable), but log so an operator can tell a
                                // committed BAL was refused due to a DB error.
                                warn!(
                                    "Failed to read header for BAL commitment check (hash {hash:#x}): {err}; reporting BAL unavailable"
                                );
                                None
                            }
                        };
                        bal.matches_commitment(commitment, &NativeCrypto)
                            .then_some(bal)
                    }
                    Ok(None) => None,
                    Err(err) => {
                        error!("Error accessing DB while building BAL response for peer: {err}");
                        None
                    }
                };
                block_access_lists.push(bal);
                if block_access_lists.len() >= BLOCK_ACCESS_LIST_LIMIT {
                    break;
                }
            }
            let response = BlockAccessLists::new(id, block_access_lists);
            send(state, Message::BlockAccessLists(response)).await?;
        }
        Message::GetReceipts68(GetReceipts68 { id, block_hashes }) if peer_supports_eth => {
            let mut receipts = Vec::new();
            for hash in block_hashes.iter() {
                receipts.push(state.storage.get_receipts_for_block(hash).await?);
            }
            send(state, Message::Receipts68(Receipts68::new(id, receipts))).await?;
        }
        Message::GetReceipts69(GetReceipts68 { id, block_hashes }) if peer_supports_eth => {
            let mut receipts = Vec::new();
            for hash in block_hashes.iter() {
                receipts.push(state.storage.get_receipts_for_block(hash).await?);
            }
            send(state, Message::Receipts69(Receipts69::new(id, receipts))).await?;
        }
        // EIP-7975: eth/70 partial receipt requests
        Message::GetReceipts70(GetReceipts70 {
            id,
            first_block_receipt_index,
            block_hashes,
        }) if peer_supports_eth => {
            let block_hashes = &block_hashes[..block_hashes.len().min(256)];
            let mut all_receipts: Vec<Vec<Receipt>> = Vec::new();
            let mut total_size: usize = 0;
            let mut last_block_incomplete = false;

            for (i, hash) in block_hashes.iter().enumerate() {
                let start_index = if i == 0 { first_block_receipt_index } else { 0 };
                let block_receipts = state
                    .storage
                    .get_receipts_for_block_from_index(hash, start_index, None)
                    .await?;

                let mut block_receipt_list = Vec::new();
                let mut hit_limit = false;
                for receipt in block_receipts {
                    let receipt_size = receipt.length();
                    if total_size + receipt_size > SOFT_RESPONSE_LIMIT
                        && (!block_receipt_list.is_empty() || !all_receipts.is_empty())
                    {
                        hit_limit = true;
                        // Only mark incomplete when the current block actually
                        // has a partial receipt list. When the limit is hit
                        // before any receipt from this block fits, the previous
                        // block is complete — setting the flag would cause the
                        // peer to re-request an already-complete block.
                        if !block_receipt_list.is_empty() {
                            last_block_incomplete = true;
                        }
                        break;
                    }
                    total_size += receipt_size;
                    block_receipt_list.push(receipt);
                }

                // Don't push an empty list when the limit was hit before any
                // receipt from this block could be included — an empty trailing
                // list would mislead the peer into thinking the block has no
                // transactions.
                if !block_receipt_list.is_empty() || !hit_limit {
                    all_receipts.push(block_receipt_list);
                }

                if hit_limit {
                    break;
                }
            }

            let response =
                Message::Receipts70(Receipts70::new(id, last_block_incomplete, all_receipts));
            send(state, response).await?;
        }
        Message::BlockRangeUpdate(update) => {
            trace!(
                peer=%state.node,
                range_from=update.earliest_block,
                range_to=update.latest_block,
                "Block range update",
            );
            // We will only validate the incoming update, we may decide to store and use this information in the future
            if let Err(err) = update.validate() {
                debug!(
                    peer=%state.node,
                    reason=%err,
                    "Disconnecting peer: invalid block range update",
                );
                send_disconnect_message(state, Some(DisconnectReason::SubprotocolError)).await;
                return Err(PeerConnectionError::DisconnectSent(
                    DisconnectReason::SubprotocolError,
                ));
            }
        }
        Message::NewPooledTransactionHashes(new_pooled_transaction_hashes) if peer_supports_eth => {
            // Don't request transactions if we're not synced — we won't be building blocks soon.
            if state.blockchain.is_synced() {
                let hashes = new_pooled_transaction_hashes
                    .get_transactions_to_request(&state.blockchain, state.node.node_id())?;
                if !hashes.is_empty() {
                    // Buffer hashes for batched requesting instead of sending immediately.
                    // The periodic flush_pending_tx_requests handler will send them.
                    state
                        .pending_tx_requests
                        .push((new_pooled_transaction_hashes, hashes));
                }
            }
        }
        Message::GetPooledTransactions(msg) => {
            let response = msg.handle(&state.blockchain)?;
            let batch_size = response.pooled_transactions.len() as u64;
            // Leech detection: disconnect peers that drain transactions but never contribute any.
            if state.txs_sent_to_peer + batch_size > LEECH_TX_SENT_THRESHOLD
                && !state.received_txs_from_peer
            {
                debug!(
                    peer = %state.node,
                    txs_sent = state.txs_sent_to_peer,
                    "Disconnecting peer: leech detected (sent many txs but received none)",
                );
                send_disconnect_message(state, Some(DisconnectReason::UselessPeer)).await;
                return Err(PeerConnectionError::DisconnectSent(
                    DisconnectReason::UselessPeer,
                ));
            }
            send(state, Message::PooledTransactions(response)).await?;
            state.txs_sent_to_peer += batch_size;
        }
        Message::PooledTransactions(msg) if peer_supports_eth => {
            if !msg.pooled_transactions.is_empty() {
                state.received_txs_from_peer = true;
            }
            // Always clear in-flight tracking for this response, regardless of sync status,
            // so other connections can re-request these hashes if needed.
            let removed_request = state.requested_pooled_txs.remove(&msg.id);
            if let Some((_, ref requested_hashes, _)) = removed_request {
                state
                    .blockchain
                    .mempool
                    .clear_in_flight_txs(requested_hashes)?;
            }
            // If we receive a blob transaction without blobs or with blobs that don't match the versioned hashes we must disconnect from the peer
            for tx in &msg.pooled_transactions {
                if let P2PTransaction::EIP4844TransactionWithBlobs(itx) = tx
                    && (itx.blobs_bundle.is_empty()
                        || itx
                            .blobs_bundle
                            .validate_blob_commitment_hashes(&itx.tx.blob_versioned_hashes)
                            .is_err())
                {
                    debug!(
                        peer=%state.node,
                        "Disconnecting peer: invalid or missing blobs",
                    );
                    if let Some((_announced, requested_hashes, _)) = &removed_request {
                        retry_on_alternates(&state.blockchain, &state.peer_table, requested_hashes)
                            .await;
                    }
                    send_disconnect_message(state, Some(DisconnectReason::SubprotocolError)).await;
                    return Err(PeerConnectionError::DisconnectSent(
                        DisconnectReason::SubprotocolError,
                    ));
                }
            }
            if state.blockchain.is_synced() {
                if let Some((announced, requested_hashes, _)) = &removed_request {
                    let fork = state.blockchain.current_fork().await?;
                    if let Err(error) = msg.validate_requested(announced, fork) {
                        debug!(
                            peer=%state.node,
                            reason=%error,
                            "Disconnecting peer: invalid pooled transactions response",
                        );
                        retry_on_alternates(&state.blockchain, &state.peer_table, requested_hashes)
                            .await;
                        send_disconnect_message(state, Some(DisconnectReason::SubprotocolError))
                            .await;
                        return Err(PeerConnectionError::DisconnectSent(
                            DisconnectReason::SubprotocolError,
                        ));
                    }
                }
                #[cfg(feature = "l2")]
                let is_l2_mode = state.l2_state.is_supported();

                #[cfg(not(feature = "l2"))]
                let is_l2_mode = false;
                if let Err(error) = msg.handle(&state.node, &state.blockchain, is_l2_mode).await {
                    if matches!(
                        error,
                        ethrex_blockchain::error::MempoolError::BlobsBundleError(_)
                    ) {
                        debug!(
                            peer=%state.node,
                            reason=%error,
                            "Disconnecting peer: invalid pooled transactions response",
                        );
                        if let Some((_announced, requested_hashes, _)) = &removed_request {
                            retry_on_alternates(
                                &state.blockchain,
                                &state.peer_table,
                                requested_hashes,
                            )
                            .await;
                        }
                        send_disconnect_message(state, Some(DisconnectReason::SubprotocolError))
                            .await;
                        return Err(PeerConnectionError::DisconnectSent(
                            DisconnectReason::SubprotocolError,
                        ));
                    }
                    return Err(error.into());
                }
            }
        }
        Message::GetStorageRanges(req) => {
            let response = process_storage_ranges_request(req, state.storage.clone()).await?;
            send(state, Message::StorageRanges(response)).await?
        }
        Message::GetByteCodes(req) => {
            let storage_clone = state.storage.clone();
            let response = process_byte_codes_request(req, storage_clone)
                .await
                .map_err(|_| {
                    PeerConnectionError::InternalError(
                        "Failed to execute bytecode retrieval task".to_string(),
                    )
                })?;
            send(state, Message::ByteCodes(response)).await?
        }
        Message::GetTrieNodes(req) => {
            let id = req.id;
            match process_trie_nodes_request(req, state.storage.clone()).await {
                Ok(response) => send(state, Message::TrieNodes(response)).await?,
                Err(_) => send(state, Message::TrieNodes(TrieNodes { id, nodes: vec![] })).await?,
            }
        }
        #[cfg(feature = "l2")]
        Message::L2(req) if peer_supports_l2 => {
            handle_based_capability_message(state, req).await?;
        }
        // Send response messages to the backend
        message @ Message::AccountRange(_)
        | message @ Message::StorageRanges(_)
        | message @ Message::ByteCodes(_)
        | message @ Message::TrieNodes(_)
        | message @ Message::BlockBodies(_)
        | message @ Message::BlockHeaders(_)
        | message @ Message::Receipts68(_)
        | message @ Message::Receipts69(_)
        | message @ Message::Receipts70(_)
        | message @ Message::BlockAccessLists(_) => {
            if let Some((_, tx)) = message
                .request_id()
                .and_then(|id| state.current_requests.remove(&id))
            {
                tx.send(message)
                    .map_err(|e| PeerConnectionError::SendMessage(e.to_string()))?
            } else {
                return Err(PeerConnectionError::ExpectedRequestId(format!("{message}")));
            }
        }
        // TODO: Add new message types and handlers as they are implemented
        message => return Err(PeerConnectionError::MessageNotHandled(format!("{message}"))),
    };
    Ok(())
}

async fn handle_outgoing_message(
    state: &mut Established,
    message: Message,
) -> Result<(), PeerConnectionError> {
    trace!(
        peer=%state.node,
        %message,
        "Sending message"
    );
    send(state, message).await?;
    Ok(())
}

async fn handle_outgoing_request(
    state: &mut Established,
    message: Message,
    sender: oneshot::Sender<Message>,
) -> Result<(), PeerConnectionError> {
    // Insert the request in the request map if it supports a request id.
    message.request_id().and_then(|id| {
        state
            .current_requests
            .insert(id, (format!("{message}"), sender))
    });
    trace!(
        peer=%state.node,
        %message,
        "Sending request"
    );
    send(state, message).await?;
    Ok(())
}

async fn handle_broadcast(
    state: &mut Established,
    (id, broadcasted_msg): (task::Id, Arc<Message>),
) -> Result<(), PeerConnectionError> {
    if id != tokio::task::id() {
        match broadcasted_msg.as_ref() {
            #[cfg(feature = "l2")]
            l2_msg @ Message::L2(_) => {
                handle_l2_broadcast(state, l2_msg).await?;
            }
            msg => {
                error!(
                    peer=%state.node,
                    message=%msg,
                    "Non-supported message broadcasted"
                );
                let error_message = format!("Non-supported message broadcasted: {msg}");
                return Err(PeerConnectionError::BroadcastError(error_message));
            }
        }
    }
    Ok(())
}

async fn handle_block_range_update(state: &mut Established) -> Result<(), PeerConnectionError> {
    if should_send_block_range_update(state).await? {
        send_block_range_update(state).await
    } else {
        Ok(())
    }
}

/// Drains the pending transaction request buffer and sends batched
/// GetPooledTransactions requests, respecting the 256-hash-per-request
/// limit from the devp2p ETH spec.
async fn flush_pending_tx_requests(state: &mut Established) -> Result<(), PeerConnectionError> {
    if state.pending_tx_requests.is_empty() {
        return Ok(());
    }

    let pending = std::mem::take(&mut state.pending_tx_requests);

    // Build a trimmed announcement containing only the hashes we're actually requesting,
    // with their original types and sizes for response validation.
    let mut all_hashes: Vec<H256> = Vec::new();
    let mut all_types: Vec<u8> = Vec::new();
    let mut all_sizes: Vec<usize> = Vec::new();

    for (announcement, hashes) in &pending {
        let trimmed = announcement.filter_to(hashes);
        all_hashes.extend_from_slice(&trimmed.transaction_hashes);
        all_types.extend_from_slice(&trimmed.transaction_types);
        all_sizes.extend(trimmed.transaction_sizes);
    }

    // Send in chunks of MAX_HASHES_PER_REQUEST per the devp2p spec.
    const MAX_HASHES_PER_REQUEST: usize = 256;
    for (i, chunk) in all_hashes.chunks(MAX_HASHES_PER_REQUEST).enumerate() {
        let offset = i * MAX_HASHES_PER_REQUEST;
        let chunk_types = &all_types[offset..offset + chunk.len()];
        let chunk_sizes = &all_sizes[offset..offset + chunk.len()];

        let announcement = NewPooledTransactionHashes::from_raw(
            chunk_types.to_vec().into(),
            chunk_sizes.to_vec(),
            chunk.to_vec(),
        );
        let request = GetPooledTransactions::new(random(), chunk.to_vec());
        let request_id = request.id;
        // Send first, only register in requested_pooled_txs on success.
        // This ensures we never track hashes for messages that were not transmitted.
        if let Err(e) = send(state, Message::GetPooledTransactions(request)).await {
            // Clear in-flight for the current chunk (failed to send) and all remaining chunks,
            // then try alternate announcers. Order matters: clear first so the alternate's
            // reserve_unknown_hashes sees the hashes as free.
            // Build an announcement covering every unsent hash (later chunks too) so the
            // alternate can validate its response against the original type/size metadata.
            let unsent = &all_hashes[offset..];
            if !unsent.is_empty() {
                if let Err(clear_err) = state.blockchain.mempool.clear_in_flight_txs(unsent) {
                    warn!(error = %clear_err, "Failed to clear in-flight transaction tracking after send error");
                }
                retry_on_alternates(&state.blockchain, &state.peer_table, unsent).await;
            }
            return Err(e);
        }
        state
            .requested_pooled_txs
            .insert(request_id, (announcement, chunk.to_vec(), Instant::now()));
    }

    Ok(())
}

/// For each hash that has a remaining alternate announcer, look up that
/// peer's connection and enqueue the request there. Each alternate carries
/// the (type, size) metadata it originally announced, so the retry request
/// is built from the alternate's own announcement rather than the failing
/// peer's; otherwise validation against the failing peer's sizes would
/// reject the alternate's response when the two announcements differ (e.g.
/// bare blob tx vs full sidecar).
///
/// If a popped alternate is no longer reachable, keep popping until a live
/// peer is found or alternates for that hash are exhausted, so a disconnected
/// alternate doesn't burn the only fallback slot.
async fn retry_on_alternates(
    blockchain: &Arc<Blockchain>,
    peer_table: &PeerTable,
    hashes: &[H256],
) {
    if hashes.is_empty() {
        return;
    }
    // Group hashes by chosen live alternate, carrying their own type/size.
    // We walk per-hash so a dead alternate for hash X doesn't consume the
    // slot that hash Y could use. The `PeerConnection` handle from the
    // liveness probe is stashed in `by_peer` and reused at enqueue time,
    // so there's no second lookup (and no race where the connection drops
    // between probe and use).
    type AltGroup = (PeerConnection, Vec<(H256, u8, usize)>);
    let mut by_peer: FxHashMap<H256, AltGroup> = FxHashMap::default();
    for hash in hashes {
        loop {
            let alt = match blockchain.mempool.pop_alternate(*hash) {
                Ok(Some(a)) => a,
                Ok(None) => break,
                Err(e) => {
                    warn!(error = %e, "pop_alternate failed");
                    break;
                }
            };
            // Reuse the connection we already grabbed for this peer.
            if let Some((_, list)) = by_peer.get_mut(&alt.peer_id) {
                list.push((*hash, alt.tx_type, alt.tx_size));
                break;
            }
            match peer_table.get_peer_connection(alt.peer_id).await {
                Ok(Some(conn)) => {
                    by_peer.insert(alt.peer_id, (conn, vec![(*hash, alt.tx_type, alt.tx_size)]));
                    break;
                }
                Ok(None) => continue, // dead peer, try next alternate
                Err(e) => {
                    warn!(error = %e, "get_peer_connection failed");
                    break;
                }
            }
        }
    }

    for (_, (conn, entries)) in by_peer {
        let mut types = Vec::with_capacity(entries.len());
        let mut sizes = Vec::with_capacity(entries.len());
        let mut hash_list = Vec::with_capacity(entries.len());
        for (h, t, s) in &entries {
            hash_list.push(*h);
            types.push(*t);
            sizes.push(*s);
        }
        let announcement =
            NewPooledTransactionHashes::from_raw(types.into(), sizes, hash_list.clone());
        if let Err(e) = conn.enqueue_tx_requests(announcement, hash_list) {
            debug!(error = %e, "Failed to enqueue tx requests on alternate peer");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        network::P2PContext, peer_table::PeerTableServer, rlpx::utils::decompress_pubkey,
        tx_broadcaster::TxBroadcaster, types::NetworkConfig,
    };
    use ethrex_common::types::Genesis;
    use ethrex_rlp::error::RLPDecodeError;
    use ethrex_storage::EngineType;
    use secp256k1::rand::rngs::OsRng;
    use std::{
        io,
        net::{IpAddr, Ipv4Addr},
        sync::atomic::AtomicBool,
    };
    use tokio::net::TcpListener;
    use tokio_util::task::TaskTracker;

    #[test]
    fn inbound_rlp_decode_error_is_protocol_error() {
        let err = PeerConnectionError::RLPDecodeError(RLPDecodeError::MalformedData);
        assert_eq!(
            disconnect_reason_for_inbound_error(&err),
            DisconnectReason::ProtocolError
        );
    }

    #[test]
    fn inbound_invalid_frame_is_protocol_error() {
        let err = PeerConnectionError::InvalidMessageFrame("bad mac".to_string());
        assert_eq!(
            disconnect_reason_for_inbound_error(&err),
            DisconnectReason::ProtocolError
        );
    }

    #[test]
    fn inbound_cryptography_error_is_protocol_error() {
        let err = PeerConnectionError::CryptographyError("bad keystream".to_string());
        assert_eq!(
            disconnect_reason_for_inbound_error(&err),
            DisconnectReason::ProtocolError
        );
    }

    #[test]
    fn inbound_invalid_message_length_is_protocol_error() {
        assert_eq!(
            disconnect_reason_for_inbound_error(&PeerConnectionError::InvalidMessageLength),
            DisconnectReason::ProtocolError
        );
    }

    #[test]
    fn inbound_io_error_is_network_error() {
        let err = PeerConnectionError::IoError(io::Error::new(io::ErrorKind::BrokenPipe, "pipe"));
        assert_eq!(
            disconnect_reason_for_inbound_error(&err),
            DisconnectReason::NetworkError
        );
    }

    #[test]
    fn inbound_internal_error_is_network_error() {
        let err = PeerConnectionError::InternalError("poisoned eth_version lock".to_string());
        assert_eq!(
            disconnect_reason_for_inbound_error(&err),
            DisconnectReason::NetworkError
        );
    }

    #[test]
    fn inbound_timeout_is_network_error() {
        assert_eq!(
            disconnect_reason_for_inbound_error(&PeerConnectionError::Timeout),
            DisconnectReason::NetworkError
        );
    }

    fn test_established(outbound_tx: tokio::sync::mpsc::Sender<Message>) -> Established {
        let store = Store::new("", EngineType::InMemory).expect("in-memory store");
        let blockchain = Arc::new(Blockchain::default_with_store(store.clone()));
        let peer_table = PeerTableServer::spawn(H256::from_low_u64_be(1), 10, store.clone());
        let tx_broadcaster =
            TxBroadcaster::spawn(peer_table.clone(), blockchain.clone(), 1_000).expect("tx bc");
        let signer = SecretKey::new(&mut OsRng);
        let node = Node::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            30303,
            30303,
            decompress_pubkey(&PublicKey::from_secret_key(secp256k1::SECP256K1, &signer)),
        );
        let (broadcast_tx, _broadcast_rx) = broadcast::channel(1);
        Established {
            signer,
            outbound_tx,
            outbound_writer_timed_out: Arc::new(AtomicBool::new(false)),
            node,
            is_inbound: false,
            storage: store,
            blockchain,
            capabilities: vec![Capability::eth(68)],
            negotiated_eth_capability: Some(Capability::eth(68)),
            negotiated_snap_capability: None,
            last_block_range_update_block: 0,
            requested_pooled_txs: HashMap::new(),
            pending_tx_requests: Vec::new(),
            client_version: "ethrex-test".to_string(),
            connection_broadcast_send: broadcast_tx,
            peer_table,
            #[cfg(feature = "l2")]
            l2_state: L2ConnState::Unsupported,
            tx_broadcaster,
            current_requests: HashMap::new(),
            disconnect_reason: None,
            is_validated: true,
            serve_request_window_start: Instant::now(),
            serve_requests_in_window: 0,
            txs_sent_to_peer: 0,
            received_txs_from_peer: false,
            missed_pongs: 0,
        }
    }

    #[tokio::test]
    async fn apply_inbound_stream_failure_records_network_error_reason() {
        let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(16);
        let mut state = test_established(outbound_tx);

        apply_inbound_stream_failure(
            &mut state,
            DisconnectReason::NetworkError,
            "simulated io failure",
        )
        .await;

        assert_eq!(
            state.disconnect_reason,
            Some(DisconnectReason::NetworkError)
        );
        // NetworkError must not enqueue a wire Disconnect.
        assert!(outbound_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn apply_inbound_stream_failure_preserves_earlier_reason() {
        let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(16);
        let mut state = test_established(outbound_tx);
        state.disconnect_reason = Some(DisconnectReason::PingTimeout);

        apply_inbound_stream_failure(
            &mut state,
            DisconnectReason::ProtocolError,
            "decode failed after peer disconnect reason was set",
        )
        .await;

        assert_eq!(state.disconnect_reason, Some(DisconnectReason::PingTimeout));
        // Stored reason is not ProtocolError, so no wire Disconnect.
        assert!(outbound_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn apply_inbound_stream_failure_sends_disconnect_on_protocol_error() {
        let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(16);
        let mut state = test_established(outbound_tx);

        apply_inbound_stream_failure(
            &mut state,
            DisconnectReason::ProtocolError,
            "simulated decode failure",
        )
        .await;

        assert_eq!(
            state.disconnect_reason,
            Some(DisconnectReason::ProtocolError)
        );
        match outbound_rx.try_recv() {
            Ok(Message::Disconnect(DisconnectMessage {
                reason: Some(DisconnectReason::ProtocolError),
            })) => {}
            other => panic!("expected ProtocolError Disconnect, got {other:?}"),
        }
    }

    /// Behavioral regression for https://github.com/lambdaclass/ethrex/issues/7035.
    ///
    /// Completes a real TCP RLPx handshake (no test-only `started()` seam), then
    /// delivers `InboundStreamFailed` — the same message the inbound `Framed`
    /// listener sends on codec/decode `Err`. Asserts the peer leaves selection
    /// well before missed-pong timeout (~30s).
    ///
    /// Score is not asserted: this path `remove_peer`s the row rather than
    /// applying `record_failure`.
    #[tokio::test]
    async fn inbound_stream_failed_removes_peer_from_selection_after_handshake() {
        let mut store = Store::new("", EngineType::InMemory).expect("in-memory store");
        store
            .add_initial_state(Genesis::default())
            .await
            .expect("genesis");
        let blockchain = Arc::new(Blockchain::default_with_store(store.clone()));

        let sut_signer = SecretKey::new(&mut OsRng);
        let dialer_signer = SecretKey::new(&mut OsRng);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let bound = listener.local_addr().expect("local_addr");

        let sut_node = Node::new(
            bound.ip(),
            bound.port(),
            bound.port(),
            decompress_pubkey(&PublicKey::from_secret_key(
                secp256k1::SECP256K1,
                &sut_signer,
            )),
        );
        let dialer_node = Node::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            0,
            0,
            decompress_pubkey(&PublicKey::from_secret_key(
                secp256k1::SECP256K1,
                &dialer_signer,
            )),
        );
        let dialer_id = dialer_node.node_id();

        let sut_table = PeerTableServer::spawn(sut_node.node_id(), 10, store.clone());
        let dialer_table = PeerTableServer::spawn(dialer_node.node_id(), 10, store.clone());

        let sut_ctx = P2PContext::new(
            sut_node.clone(),
            NetworkConfig::from_node(&sut_node),
            TaskTracker::new(),
            sut_signer,
            sut_table.clone(),
            store.clone(),
            blockchain.clone(),
            "ethrex-test-sut".to_string(),
            None,
            1_000,
            1.0,
        )
        .expect("sut P2PContext");
        let dialer_ctx = P2PContext::new(
            dialer_node.clone(),
            NetworkConfig::from_node(&dialer_node),
            TaskTracker::new(),
            dialer_signer,
            dialer_table,
            store,
            blockchain,
            "ethrex-test-dialer".to_string(),
            None,
            1_000,
            1.0,
        )
        .expect("dialer P2PContext");

        let sut_ctx_accept = sut_ctx.clone();
        let accept = tokio::spawn(async move {
            let (stream, peer_addr) = listener.accept().await.expect("accept");
            let permit = sut_ctx_accept
                .inbound_admission
                .clone()
                .acquire_owned()
                .await
                .expect("inbound permit");
            PeerConnection::spawn_as_receiver(sut_ctx_accept, peer_addr, stream, permit)
        });

        // Keep the dialer connection alive for the duration of the test so a
        // remote close does not race our simulated inbound failure.
        let _dialer_conn = PeerConnection::spawn_as_initiator(dialer_ctx, &sut_node);

        let peer_conn = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(Some(conn)) = sut_table.get_peer_connection(dialer_id).await {
                    break conn;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("peer should become selectable after a real RLPx handshake");

        let _sut_conn = accept.await.expect("accept task");

        assert!(
            sut_table
                .has_eligible_peer(SUPPORTED_ETH_CAPABILITIES.to_vec())
                .await
                .expect("has_eligible_peer"),
            "peer must be selectable before inbound failure"
        );

        peer_conn
            .handle
            .inbound_stream_failed(
                DisconnectReason::ProtocolError,
                "simulated framed decode error".to_string(),
            )
            .expect("send InboundStreamFailed");
        peer_conn.handle.join().await;

        assert!(
            !sut_table
                .has_eligible_peer(SUPPORTED_ETH_CAPABILITIES.to_vec())
                .await
                .expect("has_eligible_peer"),
            "peer must not remain selectable after inbound stream failure"
        );
        assert!(
            sut_table
                .get_peer_connection(dialer_id)
                .await
                .expect("get_peer_connection")
                .is_none(),
            "peer must be removed from the table after inbound stream failure"
        );
        assert!(
            sut_table
                .get_best_peer(SUPPORTED_ETH_CAPABILITIES.to_vec())
                .await
                .expect("get_best_peer")
                .is_none(),
            "get_best_peer must not return the dropped peer"
        );
    }
}
