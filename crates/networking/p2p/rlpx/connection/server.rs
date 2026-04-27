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
            blocks::{BlockBodies, BlockHeaders, GetBlockBodies, GetBlockHeaders, HashOrNumber},
            bsc::UpgradeStatusMsg,
            receipts::{
                GetReceipts68, GetReceipts70, Receipts68, Receipts69, Receipts70,
                SOFT_RESPONSE_LIMIT,
            },
            status::{StatusMessage68, StatusMessage69, StatusMessage70},
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
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::TrieError;
use futures::{SinkExt as _, Stream, stream::SplitSink};
use rand::random;
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
use tracing::{debug, error, info, trace, warn};

const PING_INTERVAL: Duration = Duration::from_secs(10);
const BLOCK_RANGE_UPDATE_INTERVAL: Duration = Duration::from_secs(60);
const INFLIGHT_TX_SWEEP_INTERVAL: Duration = Duration::from_secs(15);
const INFLIGHT_TX_TIMEOUT: Duration = Duration::from_secs(30);
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
    ) -> PeerConnection {
        let state = ConnectionState::Receiver(Receiver {
            context,
            peer_addr,
            stream: Arc::new(stream),
        });
        let connection = PeerConnectionServer { state };
        Self {
            handle: connection.start(),
        }
    }

    pub fn spawn_as_initiator(context: P2PContext, node: &Node) -> PeerConnection {
        let state = ConnectionState::Initiator(Initiator {
            context,
            node: node.clone(),
        });
        let connection = PeerConnectionServer { state };
        Self {
            handle: connection.start(),
        }
    }

    pub async fn outgoing_message(&mut self, message: Message) -> Result<(), PeerConnectionError> {
        self.handle
            .outgoing_message(message)
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
    // Sending part of the TcpStream to connect with the remote peer
    // The receiving part is owned by the stream listen loop task
    pub(crate) sink: SplitSink<Framed<TcpStream, RLPxCodec>, Message>,
    pub(crate) node: Node,
    pub(crate) storage: Store,
    pub(crate) blockchain: Arc<Blockchain>,
    pub(crate) capabilities: Vec<Capability>,
    pub(crate) negotiated_eth_capability: Option<Capability>,
    pub(crate) negotiated_snap_capability: Option<Capability>,
    pub(crate) negotiated_bsc_capability: Option<Capability>,
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
}

impl Established {
    async fn teardown(&mut self) {
        // Clear any in-flight transaction hashes so other connections can re-request them.
        for (_, (_announced, requested_hashes, _)) in self.requested_pooled_txs.drain() {
            let _ = self
                .blockchain
                .mempool
                .clear_in_flight_txs(&requested_hashes);
        }
        // Also clear hashes that were buffered but not yet sent.
        for (_announced, pending_hashes) in self.pending_tx_requests.drain(..) {
            let _ = self.blockchain.mempool.clear_in_flight_txs(&pending_hashes);
        }
        // Closing the sink. It may fail if it is already closed (eg. the other side already closed it)
        // Just logging a debug line if that's the case.
        let _ = self
            .sink
            .close()
            .await
            .inspect_err(|err| debug!("Could not close the socket: {err}"));
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
        match handshake::perform(state, eth_version.clone()).await {
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
            let self_handle = ctx.actor_ref();
            let result =
                handle_incoming_message(established_state, msg.message, self_handle).await;
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
                warn!("Could not obtain sender channel: Arc has multiple references");
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
                if let Some((_, hashes, _)) = state.requested_pooled_txs.remove(&id) {
                    let _ = state.blockchain.mempool.clear_in_flight_txs(&hashes);
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
                | PeerConnectionError::InvalidRecoveryId => {
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
                            "Error handling cast message",
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

    // Update eth capability version to the negotiated version for further message decoding.
    // BSC advertises a `bsc` capability for peer discovery, but does NOT use it
    // for message offset calculations. BSC messages (BscCapMsg, VotesMsg, UpgradeStatusMsg)
    // are sent within the eth message code range. Use V68Bsc which has the SAME
    // offsets as V68 but with a BSC-specific catch-all for unknown eth-range codes.
    let version = match &state.negotiated_eth_capability {
        Some(cap) if cap == &Capability::eth(68) && state.negotiated_bsc_capability.is_some() => {
            EthCapVersion::V68Bsc
        }
        Some(cap) if cap == &Capability::eth(68) => EthCapVersion::V68,
        Some(cap) if cap == &Capability::eth(69) => EthCapVersion::V69,
        Some(cap) if cap == &Capability::eth(70) => EthCapVersion::V70,
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

    spawn_listener(
        ctx.clone(),
        stream.filter_map(|result| match result {
            Ok(msg) => Some(peer_connection_server_protocol::IncomingMessage { message: msg }),
            Err(e) => {
                debug!(error=?e, "Error receiving RLPx message");
                // Skipping invalid data
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
                txs.iter().map(|tx| tx.hash()).collect(),
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
            ver => {
                return Err(PeerConnectionError::HandshakeError(format!(
                    "Invalid eth version {ver}"
                )));
            }
        };
        trace!(peer=%state.node, "Sending status");
        send(state, status).await?;

        // BSC peers (chain ID 56 = mainnet, 97 = Chapel testnet) expect an
        // UpgradeStatusMsg (0x0b) immediately after the eth status message.
        // Reference: https://github.com/bnb-chain/bsc/blob/master/eth/protocols/eth/handshake.go
        let chain_id = state.storage.get_chain_config().chain_id;
        if chain_id == 56 || chain_id == 97 {
            trace!(peer=%state.node, "Sending BSC UpgradeStatus");
            send(
                state,
                Message::UpgradeStatus(UpgradeStatusMsg {
                    disable_peer_tx_broadcast: false,
                }),
            )
            .await?;
        }
        // The next immediate message in the ETH protocol is the status.
        // BSC peers may send an UpgradeStatusMsg before or after their Status —
        // consume it if it arrives first, then read the actual Status.
        // Reference: https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
        let mut received_upgrade_status = false;
        let mut status_received = false;
        for _ in 0..5 {
            let msg = match receive(stream).await {
                Some(msg) => msg?,
                None => return Err(PeerConnectionError::Disconnected),
            };
            match msg {
                Message::Status68(msg_data) => {
                    trace!(peer=%state.node, "Received Status(68)");
                    let remote_head =
                        backend::validate_status(msg_data, &state.storage, &eth).await?;
                    let chain_id = state.storage.get_chain_config().chain_id;
                    debug!(
                        peer=%state.node,
                        %chain_id,
                        ?remote_head,
                        "Status(68) validated, checking BSC sync head"
                    );
                    // BSC mainnet (56) and Chapel testnet (97) have no Engine API;
                    // notify the sync bridge so it can trigger sync toward this peer's head.
                    if (chain_id == 56 || chain_id == 97) && !remote_head.is_zero() {
                        state.blockchain.set_bsc_sync_head(remote_head);
                    }
                    status_received = true;
                    break;
                }
                Message::Status69(msg_data) => {
                    trace!(peer=%state.node, "Received Status(69)");
                    let remote_head =
                        backend::validate_status(msg_data, &state.storage, &eth).await?;
                    let chain_id = state.storage.get_chain_config().chain_id;
                    if (chain_id == 56 || chain_id == 97) && !remote_head.is_zero() {
                        state.blockchain.set_bsc_sync_head(remote_head);
                    }
                    status_received = true;
                    break;
                }
                Message::Status70(msg_data) => {
                    trace!(peer=%state.node, "Received Status(70)");
                    let remote_head =
                        backend::validate_status(msg_data, &state.storage, &eth).await?;
                    let chain_id = state.storage.get_chain_config().chain_id;
                    if (chain_id == 56 || chain_id == 97) && !remote_head.is_zero() {
                        state.blockchain.set_bsc_sync_head(remote_head);
                    }
                    status_received = true;
                    break;
                }
                Message::UpgradeStatus(upgrade) => {
                    trace!(
                        peer=%state.node,
                        disable_tx_broadcast=%upgrade.disable_peer_tx_broadcast,
                        "Received BSC UpgradeStatus"
                    );
                    received_upgrade_status = true;
                    // Continue loop to read the actual Status message
                }
                Message::BscIgnored => {
                    trace!(peer=%state.node, "Ignoring bsc sub-protocol message during handshake");
                    // Continue loop — BSC peers send BscCapMsg before Status
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
        if !status_received {
            return Err(PeerConnectionError::HandshakeError(
                "Did not receive Status message after 5 attempts".to_string(),
            ));
        }
        let _ = received_upgrade_status;

        // BSC pivot header fetching is done by the snap sync module
        // after the peer connection is fully established, using PeerHandler.
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

async fn exchange_hello_messages<S>(
    state: &mut Established,
    stream: &mut S,
) -> Result<(), PeerConnectionError>
where
    S: Unpin + Stream<Item = Result<Message, PeerConnectionError>>,
{
    let mut supported_capabilities: Vec<Capability> = [
        &SUPPORTED_ETH_CAPABILITIES[..],
        &SUPPORTED_SNAP_CAPABILITIES[..],
    ]
    .concat();
    #[cfg(feature = "l2")]
    if state.l2_state.is_supported() {
        supported_capabilities.push(crate::rlpx::l2::SUPPORTED_BASED_CAPABILITIES[0].clone());
    }
    // BSC chains (mainnet 56 / Chapel 97) require the bsc capability in the Hello message.
    // Without it, BSC peers classify us as a non-BSC peer and disconnect to free peer slots.
    let chain_id_for_hello = state.storage.get_chain_config().chain_id;
    if chain_id_for_hello == 56 || chain_id_for_hello == 97 {
        supported_capabilities.push(Capability::bsc(1));
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
            let mut negotiated_bsc_version = 0u8;

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
                    // We only advertise bsc/1. Accept only bsc/1 from the peer
                    // to ensure both sides agree on the same message count (2).
                    // bsc/2 and bsc/3 have 4 messages which would shift offsets.
                    "bsc"
                        if (chain_id_for_hello == 56 || chain_id_for_hello == 97)
                            && cap.version == 1
                            && negotiated_bsc_version == 0 =>
                    {
                        negotiated_bsc_version = 1;
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
            debug!("Negotatied eth version: eth/{}", negotiated_eth_version);
            state.negotiated_eth_capability = Some(Capability::eth(negotiated_eth_version));

            if negotiated_snap_version != 0 {
                debug!("Negotatied snap version: snap/{}", negotiated_snap_version);
                state.negotiated_snap_capability = Some(Capability::snap(negotiated_snap_version));
            }

            if negotiated_bsc_version != 0 {
                debug!("Negotiated bsc version: bsc/{}", negotiated_bsc_version);
                state.negotiated_bsc_capability = Some(Capability::bsc(negotiated_bsc_version));
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
    state.sink.send(message).await
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
    self_handle: ActorRef<PeerConnectionServer>,
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
        warn!(
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
            // We ignore received Pong messages
        }
        Message::Status68(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                let _ = backend::validate_status(msg_data, &state.storage, eth).await?;
            };
        }
        Message::Status69(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                let _ = backend::validate_status(msg_data, &state.storage, eth).await?;
            };
        }
        Message::Status70(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                let _ = backend::validate_status(msg_data, &state.storage, eth).await?;
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
                let tx_hashes: Vec<_> = txs.transactions.iter().map(|tx| tx.hash()).collect();

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
                    .get_receipts_for_block_from_index(hash, start_index)
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
        Message::NewBlockHashes(announce) => {
            // BSC peers broadcast NewBlockHashes on every new tip. Forward the
            // highest-numbered hash to the BSC sync bridge (fallback) AND
            // directly fetch each announced block from this peer in parallel.
            // The direct fetch cuts the batch-wait incurred by forward_sync
            // (~0.5-2s) and gets blocks executed near-immediately.
            let chain_id = state.storage.get_chain_config().chain_id;
            if chain_id == 56 || chain_id == 97 {
                if let Some((hash, _)) = announce.hashes_and_numbers.iter().max_by_key(|(_, n)| *n)
                    && !hash.is_zero()
                {
                    state.blockchain.set_bsc_sync_head(*hash);
                }
                // Direct-fetch only when close to tip. When catching up,
                // forward sync stores headers ahead of state.
                const DIRECT_FETCH_AHEAD_LIMIT: u64 = 8;
                let latest = state.storage.get_latest_block_number().await.unwrap_or(0);
                for (hash, number) in &announce.hashes_and_numbers {
                    if hash.is_zero() {
                        continue;
                    }
                    if *number > latest + DIRECT_FETCH_AHEAD_LIMIT {
                        continue;
                    }
                    // Dedup: already have this block?
                    let already_stored = state
                        .storage
                        .get_block_header_by_hash(*hash)
                        .ok()
                        .flatten()
                        .is_some();
                    if already_stored {
                        continue;
                    }
                    // Cross-peer dedup: another peer already fetching this hash?
                    let inserted = state
                        .blockchain
                        .fetching_blocks
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(*hash);
                    if !inserted {
                        continue;
                    }
                    // Spawn a task that fetches header + body in parallel from
                    // THIS peer, then hands the assembled block to the import
                    // pipeline. On any failure, we just release the in-flight
                    // mark — the sync bridge (primed via set_bsc_sync_head
                    // above) will pick up the block.
                    let hash = *hash;
                    let blockchain = state.blockchain.clone();
                    let peer_handle = self_handle.clone();
                    let peer_table = state.peer_table.clone();
                    tokio::spawn(async move {
                        let primary = PeerConnection {
                            handle: peer_handle,
                        };
                        let outcome = fetch_and_import_bsc_block(
                            primary,
                            peer_table,
                            hash,
                            blockchain.clone(),
                        )
                        .await;
                        if let Err(e) = outcome {
                            debug!(%hash, "BSC direct fetch/import failed: {e}");
                        }
                        blockchain
                            .fetching_blocks
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .remove(&hash);
                    });
                }
            }
        }
        Message::NewBlockAnnouncement(announce) => {
            // BSC peers broadcast full blocks inline. Importing directly
            // removes the header+body round-trips a sync cycle would need,
            // getting us to tip with near-zero lag.
            //
            // Peer-claimed fields (`number`, `parent_hash`, etc.) are NOT
            // trusted — real validation happens inside `add_block_pipeline`.
            // The `number == latest + 1` gate is a performance heuristic only:
            // it filters the two common "wasted CPU" cases cheaply:
            //   - old blocks re-broadcast by slow peers
            //   - far-future speculative claims
            // A peer that lies about `number` just ends up at `add_block_pipeline`
            // where full validation rejects the block (same as without the gate).
            //
            // Future blocks (number > latest + 1) are already handled by
            // NewBlockHashes and BlockRangeUpdate triggering the sync cycle;
            // no need to propagate an unverified hash from NewBlock here.
            let chain_id = state.storage.get_chain_config().chain_id;
            if chain_id == 56 || chain_id == 97 {
                let block = announce.block;
                let block_hash = block.hash();
                // Skip if we already have this block stored (likely: multiple
                // peers announced the same block). `add_block_pipeline` has
                // no early duplicate check and would re-execute.
                let already_have = state
                    .storage
                    .get_block_header_by_hash(block_hash)
                    .ok()
                    .flatten()
                    .is_some();
                if !already_have {
                    // Always hand to add_block_pipeline — it validates,
                    // handles reorgs (side-chain blocks at `number == latest`
                    // can still become canonical), and auto-queues to the
                    // pending store when the parent is missing. Out-of-order
                    // arrivals are handled transparently. `add_block_pipeline`
                    // internally serializes BSC callers on `bsc_import_lock`.
                    let blockchain = state.blockchain.clone();
                    let block_number = block.header.number;
                    tokio::task::spawn_blocking(move || {
                        if blockchain.add_block_pipeline(block, None).is_ok() {
                            // Match the direct-fetch path: advance the
                            // canonical head so RPC reflects the new tip
                            // and forward_sync skips this block on its
                            // next pass.
                            tokio::runtime::Handle::current().block_on(async {
                                let _ = blockchain
                                    .advance_canonical_head(block_number, block_hash)
                                    .await;
                            });
                        }
                    });
                }
                // Wake the sync bridge so any newly-pending blocks get
                // drained promptly. BSC forward sync fetches by number,
                // so peer-claimed hash can't poison the import path.
                state.blockchain.set_bsc_sync_head(block_hash);
                // Else: ignore. Other triggers (NewBlockHashes,
                // BlockRangeUpdate) will drive the sync bridge if we're
                // actually behind.
            }
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
                warn!(
                    peer=%state.node,
                    reason=%err,
                    "disconnected from peer",
                );
                send_disconnect_message(state, Some(DisconnectReason::SubprotocolError)).await;
                return Err(PeerConnectionError::DisconnectSent(
                    DisconnectReason::SubprotocolError,
                ));
            }
            // On BSC chains, BlockRangeUpdate carries the peer's current tip hash.
            // Feed it into the sync-head candidate set for fresher pivot selection.
            let chain_id = state.storage.get_chain_config().chain_id;
            if (chain_id == 56 || chain_id == 97) && !update.latest_block_hash.is_zero() {
                state.blockchain.set_bsc_sync_head(update.latest_block_hash);
            }
        }
        Message::NewPooledTransactionHashes(new_pooled_transaction_hashes) if peer_supports_eth => {
            // Don't request transactions if we're not synced — we won't be building blocks soon.
            if state.blockchain.is_synced() {
                let hashes =
                    new_pooled_transaction_hashes.get_transactions_to_request(&state.blockchain)?;
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
                warn!(
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
                    warn!(
                        peer=%state.node,
                        "disconnected from peer. Reason: Invalid/Missing Blobs",
                    );
                    send_disconnect_message(state, Some(DisconnectReason::SubprotocolError)).await;
                    return Err(PeerConnectionError::DisconnectSent(
                        DisconnectReason::SubprotocolError,
                    ));
                }
            }
            if state.blockchain.is_synced() {
                if let Some((announced, _requested_hashes, _)) = removed_request {
                    let fork = state.blockchain.current_fork().await?;
                    if let Err(error) = msg.validate_requested(&announced, fork) {
                        warn!(
                            peer=%state.node,
                            reason=%error,
                            "disconnected from peer",
                        );
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
                        warn!(
                            peer=%state.node,
                            reason=%error,
                            "disconnected from peer",
                        );
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
        | message @ Message::Receipts70(_) => {
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
        // BSC-specific: UpgradeStatusMsg (0x0b) is sent by BSC peers immediately
        // after the eth status exchange. We consume it without disconnecting.
        // The disable_peer_tx_broadcast flag is ignored for now.
        // Reference: https://github.com/bnb-chain/bsc/blob/master/eth/protocols/eth/handshake.go
        Message::UpgradeStatus(msg) => {
            trace!(
                peer=%state.node,
                disable_peer_tx_broadcast=%msg.disable_peer_tx_broadcast,
                "Received BSC UpgradeStatus",
            );
        }
        // bsc sub-protocol messages (BscCapMsg / VotesMsg) — silently consumed.
        // We advertise bsc/1 to keep BSC peers connected but do not implement the
        // bsc sub-protocol beyond accepting the connection.
        Message::BscIgnored => {
            trace!(peer=%state.node, "Received bsc sub-protocol message (ignored)");
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
            // Clear in-flight for the current chunk (failed to send) and all remaining chunks.
            let unsent = &all_hashes[offset..];
            if !unsent.is_empty() {
                let _ = state.blockchain.mempool.clear_in_flight_txs(unsent);
            }
            return Err(e);
        }
        state
            .requested_pooled_txs
            .insert(request_id, (announcement, chunk.to_vec(), Instant::now()));
    }

    Ok(())
}

/// Fetch a single block by hash for the BSC `NewBlockHashes` fast-path and
/// submit it to `add_block_pipeline`.
///
/// `primary` is the announcing peer; we also pull the top-N highest-scored
/// eth-capable peers from the peer table and race the fetches across all of
/// them. The first peer to deliver a valid (header, body) pair wins; the
/// rest are dropped (their orphan responses are tolerated by the existing
/// `current_requests` plumbing). Errors are non-fatal — the sync bridge
/// fallback covers any block that drops on the floor.
async fn fetch_and_import_bsc_block(
    primary: PeerConnection,
    peer_table: PeerTable,
    hash: H256,
    blockchain: Arc<Blockchain>,
) -> Result<(), String> {
    use ethrex_common::types::Block;
    const FETCH_TIMEOUT: Duration = Duration::from_secs(3);
    /// Hedge across top-N highest-scored peers (plus the announcer).
    /// 5 brings P(all-slow) below ~0.5% under the observed RTT
    /// distribution while keeping bandwidth bounded.
    const FANOUT: usize = 5;

    // Build the candidate list: announcer (slot 0, no peer_id known here)
    // + LRU "proven-fast" peers + top-scored peers, deduped, up to FANOUT.
    // peer_ids[i] is None for the announcer and Some(_) for everyone else,
    // so we can record only non-announcer winners back into the LRU.
    let mut peer_ids: Vec<Option<H256>> = vec![None];
    let mut peers: Vec<PeerConnection> = vec![primary];
    let mut excluded: Vec<H256> = Vec::new();

    // LRU pass: insert any LRU peer that's still connected and eth-capable.
    let lru_ids = blockchain.bsc_block_winners_snapshot();
    if !lru_ids.is_empty() {
        let connected = peer_table
            .get_peer_connections(SUPPORTED_ETH_CAPABILITIES.to_vec())
            .await
            .unwrap_or_default();
        let connected_map: std::collections::HashMap<H256, PeerConnection> =
            connected.into_iter().collect();
        for id in lru_ids {
            if peers.len() >= FANOUT {
                break;
            }
            if let Some(conn) = connected_map.get(&id) {
                peers.push(conn.clone());
                peer_ids.push(Some(id));
                excluded.push(id);
            }
        }
    }

    // Fill remaining slots with top-scored peers (excluding LRU peers we
    // already added).
    while peers.len() < FANOUT {
        match peer_table
            .get_best_peer_excluding(SUPPORTED_ETH_CAPABILITIES.to_vec(), excluded.clone())
            .await
        {
            Ok(Some((id, conn))) => {
                excluded.push(id);
                peers.push(conn);
                peer_ids.push(Some(id));
            }
            _ => break,
        }
    }

    let fetch_start = std::time::Instant::now();
    let mut set = tokio::task::JoinSet::new();
    for (i, conn) in peers.into_iter().enumerate() {
        set.spawn(async move {
            let header_req = Message::GetBlockHeaders(GetBlockHeaders {
                id: rand::random(),
                startblock: HashOrNumber::Hash(hash),
                limit: 1,
                skip: 0,
                reverse: false,
            });
            let body_req = Message::GetBlockBodies(GetBlockBodies {
                id: rand::random(),
                block_hashes: vec![hash],
            });
            let mut hdr_conn = conn.clone();
            let mut body_conn = conn;
            let (h, b) = tokio::join!(
                hdr_conn.outgoing_request(header_req, FETCH_TIMEOUT),
                body_conn.outgoing_request(body_req, FETCH_TIMEOUT),
            );
            let header = match h.map_err(|e| format!("header: {e}"))? {
                Message::BlockHeaders(BlockHeaders { block_headers, .. }) => block_headers
                    .into_iter()
                    .find(|h| h.hash() == hash)
                    .ok_or_else(|| "header hash mismatch".to_string())?,
                other => return Err(format!("unexpected header response: {other}")),
            };
            let body = match b.map_err(|e| format!("body: {e}"))? {
                Message::BlockBodies(BlockBodies { block_bodies, .. }) => block_bodies
                    .into_iter()
                    .next()
                    .ok_or_else(|| "empty body response".to_string())?,
                other => return Err(format!("unexpected body response: {other}")),
            };
            Ok::<_, String>((i, header, body))
        });
    }

    // Take the first peer to deliver a valid pair; abort the rest.
    let mut last_err = String::from("no peers");
    let (winner_i, header, body) = loop {
        match set.join_next().await {
            Some(Ok(Ok(v))) => break v,
            Some(Ok(Err(e))) => last_err = e,
            Some(Err(e)) => last_err = format!("join: {e}"),
            None => return Err(format!("all peers failed: {last_err}")),
        }
    };
    set.abort_all();
    let fetch_ms = fetch_start.elapsed().as_millis() as u64;

    // Record non-announcer winners to the LRU. The announcer (winner_i == 0)
    // wins most races by default (propagation head start), so recording it
    // adds no signal — the LRU's purpose is to surface peers that can
    // actually outrace the announcer for the slow-announcer cases.
    if let Some(Some(winner_id)) = peer_ids.get(winner_i).copied() {
        blockchain.bsc_record_block_winner(winner_id);
    }

    let block = Block { header, body };
    let number = block.header.number;
    let block_hash = block.hash();
    let blockchain_ref = blockchain.clone();
    let pipeline_start = std::time::Instant::now();
    tokio::task::spawn_blocking(move || blockchain_ref.add_block_pipeline(block, None))
        .await
        .map_err(|e| format!("join: {e}"))?
        .map_err(|e| format!("pipeline: {e}"))?;
    let pipeline_done = std::time::Instant::now();
    blockchain
        .advance_canonical_head(number, block_hash)
        .await
        .map_err(|e| format!("forkchoice: {e}"))?;
    let head_done = std::time::Instant::now();
    info!(
        block = number,
        winner = winner_i,
        fetch_ms,
        pipeline_ms = pipeline_done.duration_since(pipeline_start).as_millis() as u64,
        forkchoice_ms = head_done.duration_since(pipeline_done).as_millis() as u64,
        "BSC direct-fetch import"
    );
    Ok(())
}
