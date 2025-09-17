use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
    time::Duration,
};

use ethrex_blockchain::Blockchain;
use ethrex_common::types::{MempoolTransaction, Transaction};
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
use spawned_rt::tasks::{BroadcastStream, mpsc};
use tokio::{
    net::TcpStream,
    sync::{Mutex, broadcast},
    task::{self, Id},
};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::{debug, error};

use crate::{
    kademlia::{Kademlia, PeerChannels},
    metrics::METRICS,
    network::P2PContext,
    rlpx::{
        Message,
        connection::{codec::RLPxCodec, handshake},
        error::RLPxError,
        eth::{
            backend,
            blocks::{BlockBodies, BlockHeaders},
            receipts::{GetReceipts, Receipts68, Receipts69},
            status::{StatusMessage68, StatusMessage69},
            transactions::{GetPooledTransactions, NewPooledTransactionHashes},
            update::BlockRangeUpdate,
        },
        l2::{
            self, PERIODIC_BATCH_BROADCAST_INTERVAL, PERIODIC_BLOCK_BROADCAST_INTERVAL,
            l2_connection::{
                self, L2Cast, L2ConnState, broadcast_l2_message, handle_based_capability_message,
                handle_l2_broadcast,
            },
        },
        message::EthCapVersion,
        p2p::{
            self, Capability, DisconnectMessage, DisconnectReason, PingMessage, PongMessage,
            SUPPORTED_ETH_CAPABILITIES, SUPPORTED_SNAP_CAPABILITIES,
        },
        utils::{log_peer_debug, log_peer_error, log_peer_warn},
    },
    snap::{
        process_account_range_request, process_byte_codes_request, process_storage_ranges_request,
        process_trie_nodes_request,
    },
    tx_broadcaster::{InMessage, TxBroadcaster, send_tx_hashes},
    types::Node,
};

const PING_INTERVAL: Duration = Duration::from_secs(10);
const BLOCK_RANGE_UPDATE_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) type RLPxConnBroadcastSender = broadcast::Sender<(tokio::task::Id, Arc<Message>)>;

type MsgResult = Result<OutMessage, RLPxError>;
type RLPxConnectionHandle = GenServerHandle<RLPxConnection>;

#[derive(Clone, Debug)]
pub struct Initiator {
    pub(crate) context: P2PContext,
    pub(crate) node: Node,
}

#[derive(Clone, Debug)]
pub struct Receiver {
    pub(crate) context: P2PContext,
    pub(crate) peer_addr: SocketAddr,
    pub(crate) stream: Arc<TcpStream>,
}

#[derive(Clone, Debug)]
pub struct Established {
    pub(crate) signer: SecretKey,
    // Sending part of the TcpStream to connect with the remote peer
    // The receiving part is owned by the stream listen loop task
    pub(crate) sink: Arc<Mutex<SplitSink<Framed<TcpStream, RLPxCodec>, Message>>>,
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
    pub(crate) connection_broadcast_send: RLPxConnBroadcastSender,
    pub(crate) table: Kademlia,
    pub(crate) backend_channel: Option<mpsc::Sender<Message>>,
    pub(crate) _inbound: bool,
    pub(crate) l2_state: L2ConnState,
    pub(crate) tx_broadcaster: GenServerHandle<TxBroadcaster>,
}

impl Established {
    async fn teardown(&self) {
        // Closing the sink. It may fail if it is already closed (eg. the other side already closed it)
        // Just logging a debug line if that's the case.
        let _ = self
            .sink
            .lock()
            .await
            .close()
            .await
            .inspect_err(|err| debug!("Could not close the socket: {err}"));
    }
}

#[derive(Clone, Debug)]
pub enum InnerState {
    HandshakeFailed,
    Initiator(Initiator),
    Receiver(Receiver),
    Established(Established),
}

#[derive(Clone, Debug)]
#[allow(private_interfaces)]
pub enum CastMessage {
    PeerMessage(Message),
    BackendMessage(Message),
    SendPing,
    BlockRangeUpdate,
    BroadcastMessage(task::Id, Arc<Message>),
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
pub struct RLPxConnection {
    inner_state: InnerState,
}

impl RLPxConnection {
    pub async fn spawn_as_receiver(
        context: P2PContext,
        peer_addr: SocketAddr,
        stream: TcpStream,
    ) -> RLPxConnectionHandle {
        let inner_state = InnerState::Receiver(Receiver {
            context,
            peer_addr,
            stream: Arc::new(stream),
        });
        let connection = RLPxConnection { inner_state };
        connection.start()
    }

    pub async fn spawn_as_initiator(context: P2PContext, node: &Node) -> RLPxConnectionHandle {
        let inner_state = InnerState::Initiator(Initiator {
            context,
            node: node.clone(),
        });
        let connection = RLPxConnection { inner_state };
        connection.start()
    }
}

impl GenServer for RLPxConnection {
    type CallMsg = Unused;
    type CastMsg = CastMessage;
    type OutMsg = MsgResult;
    type Error = RLPxError;

    async fn init(
        mut self,
        handle: &GenServerHandle<Self>,
    ) -> Result<InitResult<Self>, Self::Error> {
        // Set a default eth version that we can update after we negotiate peer capabilities
        // This eth version will only be used to encode & decode the initial `Hello` messages.
        let eth_version = Arc::new(RwLock::new(EthCapVersion::default()));
        match handshake::perform(self.inner_state, eth_version.clone()).await {
            Ok((mut established_state, stream)) => {
                log_peer_debug(&established_state.node, "Starting RLPx connection");

                if let Err(reason) =
                    initialize_connection(handle, &mut established_state, stream, eth_version).await
                {
                    if let Some(contact) = established_state
                        .table
                        .table
                        .lock()
                        .await
                        .get_mut(&established_state.node.node_id())
                    {
                        match &reason {
                            RLPxError::NoMatchingCapabilities() | RLPxError::HandshakeError(_) => {
                                contact.unwanted = true
                            }
                            _ => {}
                        }
                    }
                    connection_failed(
                        &mut established_state,
                        "Failed to initialize RLPx connection",
                        &reason,
                    )
                    .await;

                    METRICS.record_new_rlpx_conn_failure(reason).await;

                    self.inner_state = InnerState::Established(established_state);
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
                    // New state
                    self.inner_state = InnerState::Established(established_state);
                    Ok(Success(self))
                }
            }
            Err(err) => {
                // Handshake failed, just log a debug message.
                // No connection was established so no need to perform any other action
                debug!("Failed Handshake on RLPx connection {err}");
                self.inner_state = InnerState::HandshakeFailed;
                Ok(NoSuccess(self))
            }
        }
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &RLPxConnectionHandle,
    ) -> CastResponse {
        if let InnerState::Established(ref mut established_state) = self.inner_state {
            let peer_supports_l2 = established_state.l2_state.connection_state().is_ok();
            let result = match message {
                Self::CastMsg::PeerMessage(message) => {
                    log_peer_debug(
                        &established_state.node,
                        &format!("Received peer message: {message}"),
                    );
                    handle_peer_message(established_state, message).await
                }
                Self::CastMsg::BackendMessage(message) => {
                    log_peer_debug(
                        &established_state.node,
                        &format!("Received backend message: {message}"),
                    );
                    handle_backend_message(established_state, message).await
                }
                Self::CastMsg::SendPing => {
                    send(established_state, Message::Ping(PingMessage {})).await
                }
                Self::CastMsg::BroadcastMessage(id, msg) => {
                    log_peer_debug(
                        &established_state.node,
                        &format!("Received broadcasted message: {msg}"),
                    );
                    handle_broadcast(established_state, (id, msg)).await
                }
                Self::CastMsg::BlockRangeUpdate => {
                    log_peer_debug(&established_state.node, "Block Range Update");
                    handle_block_range_update(established_state).await
                }
                Self::CastMsg::L2(msg) if peer_supports_l2 => {
                    log_peer_debug(&established_state.node, "Handling cast for L2 msg: {msg:?}");
                    match msg {
                        L2Cast::BatchBroadcast => {
                            l2_connection::send_sealed_batch(established_state).await
                        }
                        L2Cast::BlockBroadcast => {
                            l2::l2_connection::send_new_block(established_state).await
                        }
                    }
                }
                _ => Err(RLPxError::MessageNotHandled(
                    "Unknown message or capability not handled".to_string(),
                )),
            };

            if let Err(e) = result {
                match e {
                    RLPxError::Disconnected()
                    | RLPxError::DisconnectReceived(_)
                    | RLPxError::DisconnectSent(_)
                    | RLPxError::HandshakeError(_)
                    | RLPxError::NoMatchingCapabilities()
                    | RLPxError::InvalidPeerId()
                    | RLPxError::InvalidMessageLength()
                    | RLPxError::StateError(_)
                    | RLPxError::InvalidRecoveryId() => {
                        log_peer_debug(&established_state.node, &e.to_string());
                        return CastResponse::Stop;
                    }
                    RLPxError::IoError(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                        log_peer_error(
                            &established_state.node,
                            "Broken pipe with peer, disconnected",
                        );
                        return CastResponse::Stop;
                    }
                    RLPxError::StoreError(StoreError::Trie(TrieError::InconsistentTree)) => {
                        if established_state.blockchain.is_synced() {
                            log_peer_error(
                                &established_state.node,
                                &format!("Error handling cast message: {e}"),
                            );
                        } else {
                            log_peer_debug(
                                &established_state.node,
                                &format!("Error handling cast message: {e}"),
                            );
                        }
                    }
                    _ => {
                        log_peer_warn(
                            &established_state.node,
                            &format!("Error handling cast message: {e}"),
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
        match self.inner_state {
            InnerState::Established(established_state) => {
                log_peer_debug(
                    &established_state.node,
                    "Closing connection with established peer",
                );
                established_state
                    .table
                    .peers
                    .lock()
                    .await
                    .remove(&established_state.node.node_id());
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
    handle: &RLPxConnectionHandle,
    state: &mut Established,
    mut stream: S,
    eth_version: Arc<RwLock<EthCapVersion>>,
) -> Result<(), RLPxError>
where
    S: Unpin + Send + Stream<Item = Result<Message, RLPxError>> + 'static,
{
    exchange_hello_messages(state, &mut stream).await?;

    // Update eth capability version to the negotiated version for further message decoding
    let version = match &state.negotiated_eth_capability {
        Some(cap) if cap == &Capability::eth(68) => EthCapVersion::V68,
        Some(cap) if cap == &Capability::eth(69) => EthCapVersion::V69,
        _ => EthCapVersion::default(),
    };
    *eth_version
        .write()
        .map_err(|err| RLPxError::InternalError(err.to_string()))? = version;

    // Handshake OK: handle connection
    // Create channels to communicate directly to the peer
    let (mut peer_channels, sender) = PeerChannels::create(handle.clone());

    // Updating the state to establish the backend channel
    state.backend_channel = Some(sender);

    init_capabilities(state, &mut stream).await?;

    state
        .table
        .set_connected_peer(
            state.node.clone(),
            peer_channels.clone(),
            state.capabilities.clone(),
        )
        .await;

    log_peer_debug(&state.node, "Peer connection initialized.");

    // Send transactions transaction hashes from mempool at connection start
    send_all_pooled_tx_hashes(state, &mut peer_channels).await?;

    // Periodic Pings repeated events.
    send_interval(PING_INTERVAL, handle.clone(), CastMessage::SendPing);

    // Periodic block range update.
    send_interval(
        BLOCK_RANGE_UPDATE_INTERVAL,
        handle.clone(),
        CastMessage::BlockRangeUpdate,
    );

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
            Ok(msg) => Some(CastMessage::PeerMessage(msg)),
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
    peer_channels: &mut PeerChannels,
) -> Result<(), RLPxError> {
    let txs: Vec<MempoolTransaction> = state
        .blockchain
        .mempool
        .get_all_txs_by_sender()?
        .into_values()
        .flatten()
        .collect();
    if !txs.is_empty() {
        state
            .tx_broadcaster
            .cast(InMessage::AddTxs(
                txs.iter().map(|tx| tx.hash()).collect(),
                state.node.node_id(),
            ))
            .await
            .map_err(|e| RLPxError::BroadcastError(e.to_string()))?;
        send_tx_hashes(
            txs,
            state.capabilities.clone(),
            peer_channels,
            state.node.node_id(),
            &state.blockchain,
        )
        .await
        .map_err(|e| RLPxError::SendMessage(e.to_string()))?;
    }
    Ok(())
}

async fn send_block_range_update(state: &mut Established) -> Result<(), RLPxError> {
    // BlockRangeUpdate was introduced in eth/69
    if let Some(eth) = &state.negotiated_eth_capability {
        if eth.version >= 69 {
            log_peer_debug(&state.node, "Sending BlockRangeUpdate");
            let update = BlockRangeUpdate::new(&state.storage).await?;
            let lastet_block = update.latest_block;
            send(state, Message::BlockRangeUpdate(update)).await?;
            state.last_block_range_update_block = lastet_block - (lastet_block % 32);
        }
    }
    Ok(())
}

async fn should_send_block_range_update(state: &mut Established) -> Result<bool, RLPxError> {
    let latest_block = state.storage.get_latest_block_number().await?;
    if latest_block < state.last_block_range_update_block
        || latest_block - state.last_block_range_update_block >= 32
    {
        return Ok(true);
    }
    Ok(false)
}

async fn init_capabilities<S>(state: &mut Established, stream: &mut S) -> Result<(), RLPxError>
where
    S: Unpin + Stream<Item = Result<Message, RLPxError>>,
{
    // Sending eth Status if peer supports it
    if let Some(eth) = state.negotiated_eth_capability.clone() {
        let status = match eth.version {
            68 => Message::Status68(StatusMessage68::new(&state.storage).await?),
            69 => Message::Status69(StatusMessage69::new(&state.storage).await?),
            ver => {
                return Err(RLPxError::HandshakeError(format!(
                    "Invalid eth version {ver}"
                )));
            }
        };
        log_peer_debug(&state.node, "Sending status");
        send(state, status).await?;
        // The next immediate message in the ETH protocol is the
        // status, reference here:
        // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
        let msg = match receive(stream).await {
            Some(msg) => msg?,
            None => return Err(RLPxError::Disconnected()),
        };
        match msg {
            Message::Status68(msg_data) => {
                log_peer_debug(&state.node, "Received Status(68)");
                backend::validate_status(msg_data, &state.storage, &eth).await?
            }
            Message::Status69(msg_data) => {
                log_peer_debug(&state.node, "Received Status(69)");
                backend::validate_status(msg_data, &state.storage, &eth).await?
            }
            Message::Disconnect(disconnect) => {
                return Err(RLPxError::HandshakeError(format!(
                    "Peer disconnected due to: {}",
                    disconnect.reason()
                )));
            }
            _ => {
                return Err(RLPxError::HandshakeError(
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
            log_peer_debug(
                &state.node,
                &format!("Could not send Disconnect message: ({reason:?})."),
            );
        });
}

async fn connection_failed(state: &mut Established, error_text: &str, error: &RLPxError) {
    log_peer_debug(&state.node, &format!("{error_text}: ({error})"));

    // Send disconnect message only if error is different than RLPxError::DisconnectRequested
    // because if it is a DisconnectRequested error it means that the peer requested the disconnection, not us.
    if !matches!(error, RLPxError::DisconnectReceived(_)) {
        send_disconnect_message(state, match_disconnect_reason(error)).await;
    }

    // Discard peer from kademlia table in some cases
    match error {
        // already connected, don't discard it
        RLPxError::DisconnectReceived(DisconnectReason::AlreadyConnected)
        | RLPxError::DisconnectSent(DisconnectReason::AlreadyConnected) => {
            log_peer_debug(&state.node, &format!("{error_text}: ({error})"));
            log_peer_debug(&state.node, "Peer already connected, don't replace it");
        }
        _ => {
            let remote_public_key = state.node.public_key;
            log_peer_debug(
                &state.node,
                &format!("{error_text}: ({error}), discarding peer {remote_public_key}"),
            );
        }
    }

    state.teardown().await;
}

fn match_disconnect_reason(error: &RLPxError) -> Option<DisconnectReason> {
    match error {
        RLPxError::DisconnectSent(reason) => Some(*reason),
        RLPxError::DisconnectReceived(reason) => Some(*reason),
        RLPxError::RLPDecodeError(_) => Some(DisconnectReason::NetworkError),
        // TODO build a proper matching between error types and disconnection reasons
        _ => None,
    }
}

async fn exchange_hello_messages<S>(
    state: &mut Established,
    stream: &mut S,
) -> Result<(), RLPxError>
where
    S: Unpin + Stream<Item = Result<Message, RLPxError>>,
{
    let mut supported_capabilities: Vec<Capability> = [
        &SUPPORTED_ETH_CAPABILITIES[..],
        &SUPPORTED_SNAP_CAPABILITIES[..],
    ]
    .concat();
    if state.l2_state.is_supported() {
        supported_capabilities.push(l2::SUPPORTED_BASED_CAPABILITIES[0].clone());
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
        None => return Err(RLPxError::Disconnected()),
    };

    match msg {
        Message::Hello(hello_message) => {
            let mut negotiated_eth_version = 0;
            let mut negotiated_snap_version = 0;

            log_peer_debug(
                &state.node,
                &format!(
                    "Hello message capabilities {:?}",
                    hello_message.capabilities
                ),
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
                    "based" if state.l2_state.is_supported() => {
                        state.l2_state.set_established()?;
                    }
                    _ => {}
                }
            }

            state.capabilities = hello_message.capabilities;

            if negotiated_eth_version == 0 {
                return Err(RLPxError::NoMatchingCapabilities());
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
        Message::Disconnect(disconnect) => Err(RLPxError::DisconnectReceived(disconnect.reason())),
        _ => {
            // Fail if it is not a hello message
            Err(RLPxError::BadRequest("Expected Hello message".to_string()))
        }
    }
}

pub(crate) async fn send(state: &mut Established, message: Message) -> Result<(), RLPxError> {
    state.sink.lock().await.send(message).await
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
async fn receive<S>(stream: &mut S) -> Option<Result<Message, RLPxError>>
where
    S: Unpin + Stream<Item = Result<Message, RLPxError>>,
{
    stream.next().await
}

async fn handle_peer_message(state: &mut Established, message: Message) -> Result<(), RLPxError> {
    let peer_supports_eth = state.negotiated_eth_capability.is_some();
    let peer_supports_l2 = state.l2_state.connection_state().is_ok();
    match message {
        Message::Disconnect(msg_data) => {
            let reason = msg_data.reason();

            log_peer_debug(&state.node, &format!("Received Disconnect: {reason}"));

            METRICS
                .record_new_rlpx_conn_disconnection(
                    &state.node.version.clone().unwrap_or("Unknown".to_string()),
                    reason,
                )
                .await;

            state.table.peers.lock().await.remove(&state.node.node_id());

            // TODO handle the disconnection request

            return Err(RLPxError::DisconnectReceived(reason));
        }
        Message::Ping(_) => {
            log_peer_debug(&state.node, "Sending pong message");
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
                let is_l2_mode = state.l2_state.is_supported();
                for tx in &txs.transactions {
                    // Reject blob transactions in L2 mode
                    if is_l2_mode && matches!(tx, Transaction::EIP4844Transaction(_)) {
                        log_peer_debug(
                            &state.node,
                            "Rejecting blob transaction in L2 mode - blob transactions are not supported in L2",
                        );
                        continue;
                    }

                    if let Err(e) = state.blockchain.add_transaction_to_pool(tx.clone()).await {
                        log_peer_warn(&state.node, &format!("Error adding transaction: {e}"));
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
                    .map_err(|e| RLPxError::BroadcastError(e.to_string()))?;
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
                    receipts.push(state.storage.get_receipts_for_block(hash)?);
                }
                let response = match eth.version {
                    68 => Message::Receipts68(Receipts68::new(id, receipts)),
                    69 => Message::Receipts69(Receipts69::new(id, receipts)),
                    ver => {
                        return Err(RLPxError::InternalError(format!(
                            "Invalid eth version {ver}"
                        )));
                    }
                };
                send(state, response).await?;
            }
        }
        Message::BlockRangeUpdate(update) => {
            log_peer_debug(
                &state.node,
                &format!(
                    "Block range update: {} to {}",
                    update.earliest_block, update.latest_block
                ),
            );
            // We will only validate the incoming update, we may decide to store and use this information in the future
            if let Err(err) = update.validate() {
                log_peer_warn(
                    &state.node,
                    &format!("disconnected from peer. Reason: {err}"),
                );
                send_disconnect_message(state, Some(DisconnectReason::SubprotocolError)).await;
                return Err(RLPxError::DisconnectSent(
                    DisconnectReason::SubprotocolError,
                ));
            }
        }
        Message::NewPooledTransactionHashes(new_pooled_transaction_hashes) if peer_supports_eth => {
            let hashes =
                new_pooled_transaction_hashes.get_transactions_to_request(&state.blockchain)?;

            let request = GetPooledTransactions::new(random(), hashes);
            send(state, Message::GetPooledTransactions(request)).await?;
        }
        Message::GetPooledTransactions(msg) => {
            let response = msg.handle(&state.blockchain)?;
            send(state, Message::PooledTransactions(response)).await?;
        }
        Message::PooledTransactions(msg) if peer_supports_eth => {
            if state.blockchain.is_synced() {
                // TODO(#3745): disconnect from peers that send invalid blob sidecars
                if let Some(requested) = state.requested_pooled_txs.get(&msg.id) {
                    let fork = state.blockchain.current_fork().await?;
                    if let Err(error) = msg.validate_requested(requested, fork).await {
                        log_peer_warn(
                            &state.node,
                            &format!("disconnected from peer. Reason: {error}"),
                        );
                        send_disconnect_message(state, Some(DisconnectReason::SubprotocolError))
                            .await;
                        return Err(RLPxError::DisconnectSent(
                            DisconnectReason::SubprotocolError,
                        ));
                    } else {
                        state.requested_pooled_txs.remove(&msg.id);
                    }
                }
                let is_l2_mode = state.l2_state.is_supported();
                msg.handle(&state.node, &state.blockchain, is_l2_mode)
                    .await?;
            }
        }
        Message::GetStorageRanges(req) => {
            let response = process_storage_ranges_request(req, state.storage.clone()).await?;
            send(state, Message::StorageRanges(response)).await?
        }
        Message::GetByteCodes(req) => {
            let response = process_byte_codes_request(req, state.storage.clone())?;
            send(state, Message::ByteCodes(response)).await?
        }
        Message::GetTrieNodes(req) => {
            let response = process_trie_nodes_request(req, state.storage.clone()).await?;
            send(state, Message::TrieNodes(response)).await?
        }
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
            state
                .backend_channel
                .as_mut()
                // TODO: this unwrap() is temporary, until we fix the backend process to use spawned
                .expect("Backend channel is not available")
                .send(message)?
        }
        // TODO: Add new message types and handlers as they are implemented
        message => return Err(RLPxError::MessageNotHandled(format!("{message}"))),
    };
    Ok(())
}

async fn handle_backend_message(
    state: &mut Established,
    message: Message,
) -> Result<(), RLPxError> {
    log_peer_debug(&state.node, &format!("Sending message {message}"));
    send(state, message).await?;
    Ok(())
}

async fn handle_broadcast(
    state: &mut Established,
    (id, broadcasted_msg): (task::Id, Arc<Message>),
) -> Result<(), RLPxError> {
    if id != tokio::task::id() {
        match broadcasted_msg.as_ref() {
            l2_msg @ Message::L2(_) => {
                handle_l2_broadcast(state, l2_msg).await?;
            }
            msg => {
                let error_message = format!("Non-supported message broadcasted: {msg}");
                log_peer_error(&state.node, &error_message);
                return Err(RLPxError::BroadcastError(error_message));
            }
        }
    }
    Ok(())
}

async fn handle_block_range_update(state: &mut Established) -> Result<(), RLPxError> {
    if should_send_block_range_update(state).await? {
        send_block_range_update(state).await
    } else {
        Ok(())
    }
}

pub(crate) fn broadcast_message(state: &Established, msg: Message) -> Result<(), RLPxError> {
    match msg {
        l2_msg @ Message::L2(_) => broadcast_l2_message(state, l2_msg),
        msg => {
            let error_message = format!("Broadcasting for msg: {msg} is not supported");
            log_peer_error(&state.node, &error_message);
            Err(RLPxError::BroadcastError(error_message))
        }
    }
}
