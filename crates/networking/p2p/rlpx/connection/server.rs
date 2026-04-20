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
    time::Duration,
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
    pub(crate) last_block_range_update_block: u64,
    pub(crate) requested_pooled_txs: HashMap<u64, NewPooledTransactionHashes>,
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
}

impl Established {
    async fn teardown(&mut self) {
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
                debug!(
                    peer=%established_state.node,
                    is_validated=established_state.is_validated,
                    disconnect_reason=?established_state.disconnect_reason,
                    "Tearing down peer connection"
                );
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

    // Update eth capability version to the negotiated version for further message decoding
    let version = match &state.negotiated_eth_capability {
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
            68 => {
                let msg = StatusMessage68::new(&state.storage).await?;
                debug!(
                    peer=%state.node,
                    eth_version = msg.eth_version,
                    network_id = msg.network_id,
                    total_difficulty = %msg.total_difficulty,
                    block_hash = ?msg.block_hash,
                    genesis = ?msg.genesis,
                    fork_id_hash = ?msg.fork_id.fork_hash,
                    fork_id_next = msg.fork_id.fork_next,
                    "Sending Status(68)"
                );
                Message::Status68(msg)
            }
            69 => {
                let msg = StatusMessage69::new(&state.storage).await?;
                debug!(
                    peer=%state.node,
                    eth_version = msg.eth_version,
                    network_id = msg.network_id,
                    genesis = ?msg.genesis,
                    fork_id_hash = ?msg.fork_id.fork_hash,
                    fork_id_next = msg.fork_id.fork_next,
                    earliest_block = msg.earliest_block,
                    latest_block = msg.latest_block,
                    latest_hash = ?msg.latest_block_hash,
                    "Sending Status(69)"
                );
                Message::Status69(msg)
            }
            70 => Message::Status70(StatusMessage70::new(&state.storage).await?),
            ver => {
                return Err(PeerConnectionError::HandshakeError(format!(
                    "Invalid eth version {ver}"
                )));
            }
        };
        send(state, status).await?;
        // The next immediate message in the ETH protocol is the
        // status, reference here:
        // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
        let msg = match receive(stream).await {
            Some(msg) => msg?,
            None => return Err(PeerConnectionError::Disconnected),
        };
        let remote_head = match msg {
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
        };

        // On Polygon chains, signal the sync manager with the remote peer's head.
        let chain_id = state.storage.get_chain_config().chain_id;
        if ethrex_polygon::genesis::is_polygon_chain(chain_id)
            && !remote_head.is_zero()
            && state.blockchain.secs_since_last_block() > 4
        {
            debug!(
                peer=%state.node,
                head=?remote_head,
                "Setting Polygon sync target from peer status"
            );
            state.blockchain.set_polygon_sync_head(remote_head);
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
    // Bor (Polygon) uses a non-standard eth/69 Status format that includes TD.
    // If we negotiate eth/69, our standard Status (no TD) will be misinterpreted
    // by Bor (genesis hash decoded as TD → BitLen > 100 → disconnect).
    // Restrict to eth/68 for Polygon so both sides use the compatible eth/68 Status.
    let chain_id = state.storage.get_chain_config().chain_id;
    let is_polygon = ethrex_polygon::genesis::is_polygon_chain(chain_id);
    let supported_eth: &[Capability] = if is_polygon {
        &[Capability::eth(68)]
    } else {
        &SUPPORTED_ETH_CAPABILITIES
    };
    // This allow is because in l2 we mut the capabilities
    // to include the l2 cap
    #[allow(unused_mut)]
    let mut supported_capabilities: Vec<Capability> =
        [supported_eth, &SUPPORTED_SNAP_CAPABILITIES[..]].concat();
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
                        if supported_eth.contains(cap) && cap.version > negotiated_eth_version {
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
            debug!("Negotatied eth version: eth/{}", negotiated_eth_version);
            state.negotiated_eth_capability = Some(Capability::eth(negotiated_eth_version));

            if negotiated_snap_version != 0 {
                debug!("Negotatied snap version: snap/{}", negotiated_snap_version);
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

async fn handle_incoming_message(
    state: &mut Established,
    message: Message,
) -> Result<(), PeerConnectionError> {
    #[cfg(feature = "metrics")]
    {
        use ethrex_metrics::p2p::METRICS_P2P;
        METRICS_P2P.inc_incoming_message(message.metric_label());
    }
    let peer_supports_eth = state.negotiated_eth_capability.is_some();
    #[cfg(feature = "l2")]
    let peer_supports_l2 = state.l2_state.connection_state().is_ok();
    match message {
        Message::Disconnect(msg_data) => {
            let reason = msg_data.reason();
            warn!(
                peer=%state.node,
                ?reason,
                "Received Disconnect from peer"
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
            }
        }
        Message::Status69(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                let _ = backend::validate_status(msg_data, &state.storage, eth).await?;
            }
        }
        Message::Status70(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                let _ = backend::validate_status(msg_data, &state.storage, eth).await?;
            }
        }
        Message::GetAccountRange(req) => {
            let response = process_account_range_request(req, state.storage.clone()).await?;
            send(state, Message::AccountRange(response)).await?
        }
        Message::Transactions(txs) if peer_supports_eth => {
            // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#transactions-0x02
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
            let block_headers = msg_data.fetch_headers(&state.storage).await;
            trace!(
                peer=%state.node,
                id=msg_data.id,
                response_count=block_headers.len(),
                "Serving GetBlockHeaders request"
            );
            let response = BlockHeaders {
                id: msg_data.id,
                block_headers,
            };
            send(state, Message::BlockHeaders(response)).await?;
        }
        Message::GetBlockBodies(msg_data) if peer_supports_eth => {
            let block_bodies = msg_data.fetch_blocks(&state.storage).await;
            trace!(
                peer=%state.node,
                id=msg_data.id,
                response_count=block_bodies.len(),
                "Serving GetBlockBodies request"
            );
            let response = BlockBodies {
                id: msg_data.id,
                block_bodies,
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
        }
        Message::NewPooledTransactionHashes(new_pooled_transaction_hashes) if peer_supports_eth => {
            let hashes =
                new_pooled_transaction_hashes.get_transactions_to_request(&state.blockchain)?;
            let request = GetPooledTransactions::new(random(), hashes);
            state
                .requested_pooled_txs
                .insert(request.id, new_pooled_transaction_hashes);
            send(state, Message::GetPooledTransactions(request)).await?;
        }
        Message::GetPooledTransactions(msg) => {
            let response = msg.handle(&state.blockchain)?;
            send(state, Message::PooledTransactions(response)).await?;
        }
        Message::PooledTransactions(msg) if peer_supports_eth => {
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
                if let Some(requested) = state.requested_pooled_txs.get(&msg.id) {
                    let fork = state.blockchain.current_fork().await?;
                    if let Err(error) = msg.validate_requested(requested, fork) {
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
                    } else {
                        state.requested_pooled_txs.remove(&msg.id);
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
        // Polygon PoS: process new blocks received via P2P.
        // These ETH messages are deprecated post-merge for Ethereum but are the
        // primary block propagation mechanism for Polygon.
        Message::EthNewBlock(new_block) if peer_supports_eth => {
            let chain_id = state.storage.get_chain_config().chain_id;
            let is_polygon = ethrex_polygon::genesis::is_polygon_chain(chain_id);
            if is_polygon {
                let new_block = *new_block;
                let block_hash = new_block.block.hash();
                let block_number = new_block.block.header.number;

                // Skip blocks we already have or that are already being processed.
                let latest = state.storage.get_latest_block_number().await.unwrap_or(0);
                if block_number <= latest {
                    // Already processed — skip silently.
                } else if !state.blockchain.mark_polygon_in_flight(block_hash) {
                    debug!(peer=%state.node, block_number, "Block already in-flight, skipping");
                } else {
                    debug!(
                        peer=%state.node,
                        block_number,
                        hash=?block_hash,
                        td=%new_block.total_difficulty,
                        "Received new Polygon block via P2P"
                    );

                    // Check if parent exists before executing
                    let parent_hash = new_block.block.header.parent_hash;
                    if state
                        .storage
                        .get_block_header_by_hash(parent_hash)?
                        .is_none()
                    {
                        // Parent not in DB — buffer for chain-following when parent arrives
                        state
                            .blockchain
                            .buffer_polygon_pending_block(new_block.block);
                        let latest = state.storage.get_latest_block_number().await.unwrap_or(0);
                        if block_number > latest + 64 {
                            warn!(
                                peer=%state.node,
                                block_number,
                                latest,
                                gap = block_number.saturating_sub(latest),
                                "Polygon block parent not found, triggering gap-fill sync"
                            );
                            state.blockchain.set_polygon_sync_head(block_hash);
                        } else {
                            debug!(
                                peer=%state.node,
                                block_number,
                                latest,
                                "Polygon block parent not found, buffered for chain-follow"
                            );
                        }
                    } else {
                        // Parent exists — offload pipeline to background task
                        // so we don't block the peer connection for ~2s.
                        let blockchain = state.blockchain.clone();
                        let storage = state.storage.clone();
                        let block = new_block.block;
                        let announced_td = new_block.total_difficulty;
                        tokio::spawn(async move {
                            // Serialize with the sync cycle: both paths call
                            // forkchoice_update, which destructively rewinds
                            // canonical entries above the given head. Without
                            // this lock, racing updates produce a canonical
                            // "hole" that breaks BLOCKHASH lookups later.
                            let canonical_lock = blockchain.polygon_canonical_lock();
                            let _canonical_guard = canonical_lock.lock().await;
                            // Re-check: the sync cycle may have advanced past
                            // this block while we were waiting for the lock.
                            let latest = storage.get_latest_block_number().await.unwrap_or(0);
                            let blk_number = block.header.number;
                            let blk_hash = block.hash();
                            if blk_number <= latest {
                                blockchain.clear_polygon_in_flight(&blk_hash);
                                return;
                            }
                            // Run the blocking pipeline on the tokio blocking pool
                            let bc = blockchain.clone();
                            let result = tokio::task::spawn_blocking(move || {
                                bc.add_block_pipeline(block, None)
                            })
                            .await;
                            let result = match result {
                                Ok(r) => r,
                                Err(e) => {
                                    // Clear in-flight on panic so the block can be retried.
                                    blockchain.clear_polygon_in_flight(&blk_hash);
                                    warn!(block_number = blk_number, error = %e, "Polygon block task panicked");
                                    return;
                                }
                            };
                            match result {
                                Ok(()) => {
                                    let latest =
                                        storage.get_latest_block_number().await.unwrap_or(0);
                                    let local_td_lower_bound = ethrex_common::U256::from(latest);
                                    if announced_td > local_td_lower_bound || blk_number > latest {
                                        if let Err(e) = storage
                                            .forkchoice_update(
                                                vec![],
                                                blk_number,
                                                blk_hash,
                                                None,
                                                None,
                                            )
                                            .await
                                        {
                                            warn!(block_number = blk_number, error = %e, "forkchoice_update failed");
                                        } else {
                                            debug!(block_number = blk_number, %announced_td, "Updated canonical head (Polygon)");
                                        }
                                    }

                                    // Chain-follow: process buffered blocks whose parent we just stored
                                    let mut next_parent = blk_hash;
                                    let mut chain_depth = 0u32;
                                    loop {
                                        if chain_depth >= 64 {
                                            debug!("Chain-follow depth limit reached (64)");
                                            break;
                                        }
                                        let Some(pending) =
                                            blockchain.take_polygon_pending_block(next_parent)
                                        else {
                                            break;
                                        };
                                        chain_depth += 1;
                                        let pending_hash = pending.hash();
                                        let pending_number = pending.header.number;
                                        let bc_inner = blockchain.clone();
                                        let inner_result = tokio::task::spawn_blocking(move || {
                                            bc_inner.add_block_pipeline(pending, None)
                                        })
                                        .await;
                                        match inner_result {
                                            Ok(Ok(())) => {
                                                let latest = storage
                                                    .get_latest_block_number()
                                                    .await
                                                    .unwrap_or(0);
                                                if pending_number > latest {
                                                    let _ = storage
                                                        .forkchoice_update(
                                                            vec![],
                                                            pending_number,
                                                            pending_hash,
                                                            None,
                                                            None,
                                                        )
                                                        .await;
                                                }
                                                debug!(
                                                    block_number = pending_number,
                                                    "Processed buffered Polygon block"
                                                );
                                                next_parent = pending_hash;
                                            }
                                            Ok(Err(e)) => {
                                                warn!(block_number = pending_number, error = %e, "Failed to process buffered Polygon block");
                                                break;
                                            }
                                            Err(e) => {
                                                warn!(block_number = pending_number, error = %e, "Buffered block task panicked");
                                                break;
                                            }
                                        }
                                    }
                                    blockchain.clear_polygon_in_flight(&blk_hash);
                                }
                                Err(e) => {
                                    // Clear in-flight on error so the block can be retried.
                                    blockchain.clear_polygon_in_flight(&blk_hash);
                                    warn!(block_number = blk_number, error = %e, "Failed to process Polygon block");
                                }
                            }
                        });
                    }
                }
            }
            // Non-Polygon chains: silently ignore (post-merge, blocks come via Engine API).
        }
        Message::NewBlockHashes(new_block_hashes) if peer_supports_eth => {
            let chain_id = state.storage.get_chain_config().chain_id;
            let is_polygon = ethrex_polygon::genesis::is_polygon_chain(chain_id);
            if is_polygon {
                // Request full block headers+bodies for unknown, non-in-flight hashes.
                let latest_nbh = state.storage.get_latest_block_number().await.unwrap_or(0);
                let unknown: Vec<(H256, u64)> = new_block_hashes
                    .block_hashes
                    .iter()
                    .filter(|(_, num)| *num > latest_nbh)
                    .filter(|(hash, _)| {
                        state
                            .storage
                            .get_block_header_by_hash(*hash)
                            .ok()
                            .flatten()
                            .is_none()
                    })
                    .copied()
                    .collect();

                for &(hash, number) in &unknown {
                    debug!(
                        peer=%state.node,
                        block_number = number,
                        hash = ?hash,
                        "Fetching block from NewBlockHashes announcement"
                    );

                    // Send GetBlockHeaders and GetBlockBodies for this hash
                    let header_id: u64 = rand::random();
                    let body_id: u64 = rand::random();

                    let (header_tx, header_rx) = tokio::sync::oneshot::channel::<Message>();
                    let (body_tx, body_rx) = tokio::sync::oneshot::channel::<Message>();

                    handle_outgoing_request(
                        state,
                        Message::GetBlockHeaders(GetBlockHeaders {
                            id: header_id,
                            startblock: HashOrNumber::Hash(hash),
                            limit: 1,
                            skip: 0,
                            reverse: false,
                        }),
                        header_tx,
                    )
                    .await?;

                    handle_outgoing_request(
                        state,
                        Message::GetBlockBodies(GetBlockBodies {
                            id: body_id,
                            block_hashes: vec![hash],
                        }),
                        body_tx,
                    )
                    .await?;

                    // Spawn a task to await both responses and process the block
                    let blockchain = state.blockchain.clone();
                    let storage = state.storage.clone();
                    tokio::spawn(async move {
                        let timeout = std::time::Duration::from_secs(5);
                        let (header_resp, body_resp) = tokio::join!(
                            tokio::time::timeout(timeout, header_rx),
                            tokio::time::timeout(timeout, body_rx),
                        );

                        // Extract header
                        let header = match header_resp {
                            Ok(Ok(Message::BlockHeaders(mut bh)))
                                if !bh.block_headers.is_empty() =>
                            {
                                bh.block_headers.swap_remove(0)
                            }
                            _ => {
                                debug!(
                                    block_number = number,
                                    "Failed to fetch header for NewBlockHashes"
                                );
                                return;
                            }
                        };

                        // Extract body
                        let body = match body_resp {
                            Ok(Ok(Message::BlockBodies(mut bb))) if !bb.block_bodies.is_empty() => {
                                bb.block_bodies.swap_remove(0)
                            }
                            _ => {
                                debug!(
                                    block_number = number,
                                    "Failed to fetch body for NewBlockHashes"
                                );
                                return;
                            }
                        };

                        let block = ethrex_common::types::Block::new(header, body);
                        let blk_number = block.header.number;
                        let blk_hash = block.hash();

                        // Serialize with the sync cycle and NewBlock handler.
                        let canonical_lock = blockchain.polygon_canonical_lock();
                        let _canonical_guard = canonical_lock.lock().await;

                        // Skip if already processed or in-flight
                        let latest = storage.get_latest_block_number().await.unwrap_or(0);
                        if blk_number <= latest || !blockchain.mark_polygon_in_flight(blk_hash) {
                            return;
                        }

                        // Check parent exists
                        let parent_hash = block.header.parent_hash;
                        let parent_exists = storage
                            .get_block_header_by_hash(parent_hash)
                            .ok()
                            .flatten()
                            .is_some();

                        if !parent_exists {
                            blockchain.buffer_polygon_pending_block(block);
                            debug!(
                                block_number = blk_number,
                                "Fetched block buffered (parent missing)"
                            );
                            return;
                        }

                        // Process via pipeline
                        let bc = blockchain.clone();
                        let result =
                            tokio::task::spawn_blocking(move || bc.add_block_pipeline(block, None))
                                .await;
                        match result {
                            Ok(Ok(())) => {
                                let latest = storage.get_latest_block_number().await.unwrap_or(0);
                                if blk_number > latest {
                                    let _ = storage
                                        .forkchoice_update(vec![], blk_number, blk_hash, None, None)
                                        .await;
                                }
                                debug!(
                                    block_number = blk_number,
                                    "Processed block from NewBlockHashes"
                                );

                                // Chain-follow buffered blocks
                                let mut next_parent = blk_hash;
                                let mut chain_depth = 0u32;
                                loop {
                                    if chain_depth >= 64 {
                                        debug!("Chain-follow depth limit reached (64)");
                                        break;
                                    }
                                    let Some(pending) =
                                        blockchain.take_polygon_pending_block(next_parent)
                                    else {
                                        break;
                                    };
                                    chain_depth += 1;
                                    let pending_hash = pending.hash();
                                    let pending_number = pending.header.number;
                                    let bc_inner = blockchain.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        bc_inner.add_block_pipeline(pending, None)
                                    })
                                    .await
                                    {
                                        Ok(Ok(())) => {
                                            let latest = storage
                                                .get_latest_block_number()
                                                .await
                                                .unwrap_or(0);
                                            if pending_number > latest {
                                                let _ = storage
                                                    .forkchoice_update(
                                                        vec![],
                                                        pending_number,
                                                        pending_hash,
                                                        None,
                                                        None,
                                                    )
                                                    .await;
                                            }
                                            debug!(
                                                block_number = pending_number,
                                                "Processed buffered block (chain-follow)"
                                            );
                                            next_parent = pending_hash;
                                        }
                                        _ => break,
                                    }
                                }
                                blockchain.clear_polygon_in_flight(&blk_hash);
                            }
                            Ok(Err(e)) => {
                                blockchain.clear_polygon_in_flight(&blk_hash);
                                warn!(block_number = blk_number, error = %e, "Failed to process fetched block");
                            }
                            Err(e) => {
                                blockchain.clear_polygon_in_flight(&blk_hash);
                                warn!(block_number = blk_number, error = %e, "Fetched block task panicked");
                            }
                        }
                    });
                }
            }
            // Non-Polygon chains: silently ignore.
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
        // TODO: Add new message types and handlers as they are implemented
        message => {
            warn!(
                peer=%state.node,
                message=%message,
                "Unhandled incoming message"
            );
            return Err(PeerConnectionError::MessageNotHandled(format!("{message}")));
        }
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
