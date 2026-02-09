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
    peer_table::PeerTable,
    rlpx::{
        Message,
        connection::{codec::RLPxCodec, handshake},
        error::PeerConnectionError,
        eth::{
            blocks::{BlockBodies, BlockHeaders},
            receipts::{GetReceipts, Receipts68, Receipts69},
            status::{StatusMessage68, StatusMessage69},
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
    tx_broadcaster::{InMessage, TxBroadcaster, send_tx_hashes},
    types::Node,
};
use ethrex_blockchain::Blockchain;
#[cfg(feature = "l2")]
use ethrex_common::types::Transaction;
use ethrex_common::types::{MempoolTransaction, P2PTransaction};
use ethrex_storage::{Store, error::StoreError};
use ethrex_trie::TrieError;
use futures::{SinkExt as _, Stream, stream::SplitSink};
use rand::random;
use secp256k1::{PublicKey, SecretKey};
use spawned_concurrency::{
    messages::Unused,
    tasks::{
        CastResponse, GenServer, GenServerHandle,
        InitResult::{self, NoSuccess, Success},
        send_interval, spawn_listener,
    },
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
    sync::{Mutex, broadcast, oneshot},
    task::{self, Id},
};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::{debug, error, trace, warn};

const PING_INTERVAL: Duration = Duration::from_secs(10);
const BLOCK_RANGE_UPDATE_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) type PeerConnBroadcastSender = broadcast::Sender<(tokio::task::Id, Arc<Message>)>;

#[derive(Clone, Debug)]
pub struct PeerConnection {
    handle: GenServerHandle<PeerConnectionServer>,
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
            .cast(CastMessage::OutgoingMessage(message))
            .await
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
            .cast(CastMessage::OutgoingRequest(message, Arc::new(oneshot_tx)))
            .await
            .map_err(|err| PeerConnectionError::InternalError(err.to_string()))?;

        // Wait for the response or timeout. This blocks the calling task (and not the ConnectionServer task)
        match tokio::time::timeout(timeout, oneshot_rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(PeerConnectionError::RecvError(error.to_string())),
            Err(_timeout) => {
                // Notify timeout on request id
                self.handle
                    .cast(CastMessage::RequestTimeout { id })
                    .await
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
    pub(crate) tx_broadcaster: GenServerHandle<TxBroadcaster>,
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

#[derive(Clone, Debug)]
#[allow(private_interfaces)]
pub enum CastMessage {
    /// Received a message from the remote peer
    IncomingMessage(Message),
    /// We send information to the remote peer
    OutgoingMessage(Message),
    /// We request information from the remote peer
    OutgoingRequest(Message, Arc<oneshot::Sender<Message>>),
    /// Received a notification of a request that timeouted.
    RequestTimeout { id: u64 },
    /// Periodic message to send ping to remote peer
    SendPing,
    /// Periodic message to send block range update to remote peer
    BlockRangeUpdate,
    /// Received a message to broadcast. Used only for L2, we have to move this logic to tx_broadcaster.
    BroadcastMessage(task::Id, Arc<Message>),
    /// L2 message
    #[cfg(feature = "l2")]
    L2(L2Cast),
}

pub enum OutMessage {
    InitResponse {
        node: Node,
        framed: Arc<Mutex<Framed<TcpStream, RLPxCodec>>>,
    },
    Done,
    Error,
}

#[derive(Debug)]
pub struct PeerConnectionServer {
    state: ConnectionState,
}

impl GenServer for PeerConnectionServer {
    type CallMsg = Unused;
    type CastMsg = CastMessage;
    type OutMsg = Unused;
    type Error = PeerConnectionError;

    async fn init(
        mut self,
        handle: &GenServerHandle<Self>,
    ) -> Result<InitResult<Self>, Self::Error> {
        // Set a default eth version that we can update after we negotiate peer capabilities
        // This eth version will only be used to encode & decode the initial `Hello` messages.
        let eth_version = Arc::new(RwLock::new(EthCapVersion::default()));
        match handshake::perform(self.state, eth_version.clone()).await {
            Ok((mut established_state, stream)) => {
                trace!(peer=%established_state.node, "Starting RLPx connection");
                if let Err(reason) =
                    initialize_connection(handle, &mut established_state, stream, eth_version).await
                {
                    match &reason {
                        PeerConnectionError::NoMatchingCapabilities
                        | PeerConnectionError::HandshakeError(_) => {
                            established_state
                                .peer_table
                                .set_unwanted(&established_state.node.node_id())
                                .await?;
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
                    Ok(NoSuccess(self))
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
                    Ok(Success(self))
                }
            }
            Err(err) => {
                // Handshake failed, just log a debug message.
                // No connection was established so no need to perform any other action
                debug!("Failed Handshake on RLPx connection {err}");
                self.state = ConnectionState::HandshakeFailed;
                Ok(NoSuccess(self))
            }
        }
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        if let ConnectionState::Established(ref mut established_state) = self.state {
            #[cfg(feature = "l2")]
            let peer_supports_l2 = established_state.l2_state.connection_state().is_ok();
            let result = match message {
                Self::CastMsg::IncomingMessage(message) => {
                    trace!(
                        peer=%established_state.node,
                        %message,
                        "Received incoming message",
                    );
                    handle_incoming_message(established_state, message).await
                }
                Self::CastMsg::OutgoingMessage(message) => {
                    trace!(
                        peer=%established_state.node,
                        %message,
                        "Received outgoing request",
                    );
                    handle_outgoing_message(established_state, message).await
                }
                Self::CastMsg::OutgoingRequest(message, sender) => {
                    trace!(
                        peer=%established_state.node,
                        %message,
                        "Received outgoing request",
                    );
                    handle_outgoing_request(
                        established_state,
                        message,
                        Arc::<oneshot::Sender<Message>>::into_inner(sender)
                            .expect("Could not obtain sender channel"),
                    )
                    .await
                }
                Self::CastMsg::RequestTimeout { id } => {
                    // Discard the request from current requests
                    if let Some((msg_type, _)) = established_state.current_requests.remove(&id) {
                        // This log should be debug, because we should see if the timeout is something
                        // we don't expect to constantly happen, so in dev mode we should see it
                        debug!(
                            peer=%established_state.node,
                            %msg_type,
                            %id,
                            "Request timedout",
                        );
                    }
                    Ok(())
                }
                Self::CastMsg::SendPing => {
                    send(established_state, Message::Ping(PingMessage {})).await
                }
                Self::CastMsg::BroadcastMessage(id, msg) => {
                    trace!(
                        peer=%established_state.node,
                        message=%msg,
                        "Received broadcasted message",
                    );
                    handle_broadcast(established_state, (id, msg)).await
                }
                Self::CastMsg::BlockRangeUpdate => {
                    trace!(
                        peer=%established_state.node,
                        "Block Range Update"
                    );
                    handle_block_range_update(established_state).await
                }
                #[cfg(feature = "l2")]
                Self::CastMsg::L2(msg) if peer_supports_l2 => {
                    trace!(
                        peer=%established_state.node,
                        message=?msg,
                        "Handling cast for L2 msg"
                    );
                    match msg {
                        L2Cast::BatchBroadcast => {
                            let res = l2_connection::send_sealed_batch(established_state).await;
                            res.and(
                                l2_connection::process_batches_on_queue(established_state).await,
                            )
                        }
                        L2Cast::BlockBroadcast => {
                            let res = l2_connection::send_new_block(established_state).await;
                            res.and(l2_connection::process_blocks_on_queue(established_state).await)
                        }
                    }
                }
                #[cfg(feature = "l2")]
                _ => Err(PeerConnectionError::MessageNotHandled(
                    "Unknown message or capability not handled".to_string(),
                )),
            };

            if let Err(e) = result {
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
                        return CastResponse::Stop;
                    }
                    PeerConnectionError::IoError(e)
                        if e.kind() == std::io::ErrorKind::BrokenPipe =>
                    {
                        // TODO: we need to check if this message is ocurring commonly due to a problem
                        // with our concurrency model
                        debug!(peer=%established_state.node, "Broken pipe with peer, disconnected");
                        return CastResponse::Stop;
                    }
                    PeerConnectionError::StoreError(StoreError::Trie(
                        TrieError::InconsistentTree(_),
                    )) => {
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
        } else {
            // Received a Cast message but connection is not ready. Log an error but keep the connection alive.
            error!("Connection not yet established");
        }
        CastResponse::NoReply
    }

    async fn teardown(self, _handle: &GenServerHandle<Self>) -> Result<(), Self::Error> {
        match self.state {
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
                established_state
                    .peer_table
                    .remove_peer(established_state.node.node_id())
                    .await?;
                established_state.teardown().await;
            }
            _ => {
                // Nothing to do if the connection was not established
            }
        };
        Ok(())
    }
}

async fn initialize_connection<S>(
    handle: &GenServerHandle<PeerConnectionServer>,
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
        _ => EthCapVersion::default(),
    };
    *eth_version
        .write()
        .map_err(|err| PeerConnectionError::InternalError(err.to_string()))? = version;

    init_capabilities(state, &mut stream).await?;

    let mut connection = PeerConnection {
        handle: handle.clone(),
    };

    state
        .peer_table
        .new_connected_peer(
            state.node.clone(),
            connection.clone(),
            state.capabilities.clone(),
        )
        .await?;

    trace!(peer=%state.node, "Peer connection initialized.");

    // Send transactions transaction hashes from mempool at connection start
    send_all_pooled_tx_hashes(state, &mut connection).await?;

    // Periodic Pings repeated events.
    send_interval(PING_INTERVAL, handle.clone(), CastMessage::SendPing);

    // Periodic block range update.
    send_interval(
        BLOCK_RANGE_UPDATE_INTERVAL,
        handle.clone(),
        CastMessage::BlockRangeUpdate,
    );

    #[cfg(feature = "l2")]
    // Periodic L2 messages events.
    if state.l2_state.connection_state().is_ok() {
        send_interval(
            PERIODIC_BLOCK_BROADCAST_INTERVAL,
            handle.clone(),
            CastMessage::L2(L2Cast::BlockBroadcast),
        );
        send_interval(
            PERIODIC_BATCH_BROADCAST_INTERVAL,
            handle.clone(),
            CastMessage::L2(L2Cast::BatchBroadcast),
        );
    }

    spawn_listener(
        handle.clone(),
        stream.filter_map(|result| match result {
            Ok(msg) => Some(CastMessage::IncomingMessage(msg)),
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
            result
                .ok()
                .map(|(id, msg)| CastMessage::BroadcastMessage(id, msg))
        });
        spawn_listener(handle.clone(), message_stream);
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
            .cast(InMessage::AddTxs(
                txs.iter().map(|tx| tx.hash()).collect(),
                state.node.node_id(),
            ))
            .await
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
                backend::validate_status(msg_data, &state.storage, eth).await?
            };
        }
        Message::Status69(msg_data) => {
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
            if state.blockchain.is_synced() {
                #[cfg(feature = "l2")]
                let is_l2_mode = state.l2_state.is_supported();
                for tx in &txs.transactions {
                    // Reject blob transactions in L2 mode
                    #[cfg(feature = "l2")]
                    if (is_l2_mode && matches!(tx, Transaction::EIP4844Transaction(_)))
                        || tx.is_privileged()
                    {
                        let tx_type = tx.tx_type();
                        debug!(peer=%state.node, "Rejecting transaction in L2 mode - {tx_type} transactions are not broadcasted in L2");
                        continue;
                    }

                    if let Err(e) = state.blockchain.add_transaction_to_pool(tx.clone()).await {
                        debug!(
                            peer=%state.node,
                            error=%e,
                            "Error adding transaction"
                        );
                        continue;
                    }
                }
                state
                    .tx_broadcaster
                    .cast(InMessage::AddTxs(
                        txs.transactions.iter().map(|tx| tx.hash()).collect(),
                        state.node.node_id(),
                    ))
                    .await
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
        Message::GetReceipts(GetReceipts { id, block_hashes }) if peer_supports_eth => {
            if let Some(eth) = &state.negotiated_eth_capability {
                let mut receipts = Vec::new();
                for hash in block_hashes.iter() {
                    receipts.push(state.storage.get_receipts_for_block(hash).await?);
                }
                let response = match eth.version {
                    68 => Message::Receipts68(Receipts68::new(id, receipts)),
                    69 => Message::Receipts69(Receipts69::new(id, receipts)),
                    ver => {
                        return Err(PeerConnectionError::InternalError(format!(
                            "Invalid eth version {ver}"
                        )));
                    }
                };
                send(state, response).await?;
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
                    if matches!(error, ethrex_blockchain::error::MempoolError::BlobsBundleError(_)) {
                        warn!(
                            peer=%state.node,
                            reason=%error,
                            "disconnected from peer",
                        );
                        send_disconnect_message(state, Some(DisconnectReason::SubprotocolError)).await;
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
        | message @ Message::Receipts69(_) => {
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
