use std::{
    collections::{HashMap, HashSet},
    mem,
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use ethrex_blockchain::{Blockchain, mempool::Mempool};
use ethrex_common::{H256, utils::EventHandle};
use ethrex_storage::Store;
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
use tokio::{
    net::TcpStream,
    sync::{Mutex, broadcast, mpsc::Sender},
    task::{self, Id},
};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::{debug, error};

use crate::{
    discv4::server::MAX_PEERS_TCP_CONNECTIONS,
    kademlia::{KademliaTable, PeerChannels},
    network::P2PContext,
    rlpx::{
        connection::{codec::RLPxCodec, handshake},
        error::RLPxError,
        eth::{
            backend,
            blocks::{BlockBodies, BlockHeaders},
            receipts::{GetReceipts, Receipts},
            status::StatusMessage,
            transactions::{GetPooledTransactions, NewPooledTransactionHashes, Transactions},
            update::BlockRangeUpdate,
        },
        l2::{
            self, PERIODIC_BATCH_BROADCAST_INTERVAL, PERIODIC_BLOCK_BROADCAST_INTERVAL,
            l2_connection::{
                self, L2Cast, L2ConnState, broadcast_l2_message, handle_based_capability_message,
                handle_l2_broadcast,
            },
        },
        message::Message,
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
    types::Node,
};

const PING_INTERVAL: Duration = Duration::from_secs(10);
const TX_BROADCAST_INTERVAL: Duration = Duration::from_millis(500);
const BLOCK_RANGE_UPDATE_INTERVAL: Duration = Duration::from_secs(60);
// Soft limit for the number of transaction hashes sent in a single NewPooledTransactionHashes message as per [the spec](https://github.com/ethereum/devp2p/blob/master/caps/eth.md#newpooledtransactionhashes-0x080)
const NEW_POOLED_TRANSACTION_HASHES_SOFT_LIMIT: usize = 4096;

pub(crate) type RLPxConnBroadcastSender = broadcast::Sender<(tokio::task::Id, Arc<Message>)>;

type MsgResult = Result<OutMessage, RLPxError>;
type RLPxConnectionHandle = GenServerHandle<RLPxConnection>;

pub struct RLPxConnectionState(pub InnerState);

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

#[derive(Debug)]
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
    pub(crate) tx_bcast_state: TxBcastState,
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
    pub(crate) table: Arc<Mutex<KademliaTable>>,
    pub(crate) backend_channel: Option<Sender<Message>>,
    pub(crate) inbound: bool,
    pub(crate) l2_state: L2ConnState,
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

#[derive(Debug)]
pub struct TxBcastState {
    queue_and_filter: Arc<std::sync::Mutex<(HashSet<H256>, HashSet<H256>)>>,
    _listener: EventHandle<H256>,
}

impl TxBcastState {
    pub fn new(mempool: &Mempool) -> Self {
        let state = Arc::new(std::sync::Mutex::new((HashSet::new(), HashSet::new())));
        Self {
            queue_and_filter: Arc::clone(&state),
            _listener: mempool.add_listener(move |&tx_hash| {
                let mut state = state.lock().expect("poisoned mutex");
                if !state.1.contains(&tx_hash) {
                    state.0.insert(tx_hash);
                }
            }),
        }
    }
}

#[derive(Debug)]
pub enum InnerState {
    HandshakeFailed,
    Initiator(Initiator),
    Receiver(Receiver),
    Established(Established),
}

impl RLPxConnectionState {
    pub fn new_as_receiver(context: P2PContext, peer_addr: SocketAddr, stream: TcpStream) -> Self {
        Self(InnerState::Receiver(Receiver {
            context,
            peer_addr,
            stream: Arc::new(stream),
        }))
    }

    pub fn new_as_initiator(context: P2PContext, node: &Node) -> Self {
        Self(InnerState::Initiator(Initiator {
            context,
            node: node.clone(),
        }))
    }
}

#[derive(Clone, Debug)]
#[allow(private_interfaces)]
pub enum CastMessage {
    PeerMessage(Message),
    BackendMessage(Message),
    SendPing,
    SendNewPooledTxHashes,
    BlockRangeUpdate,
    BroadcastMessage(task::Id, Arc<Message>),
    L2(L2Cast),
}

#[derive(Clone)]
#[allow(private_interfaces)]
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
        match handshake::perform(self.inner_state).await {
            Ok((mut established_state, stream)) => {
                log_peer_debug(&established_state.node, "Starting RLPx connection");

                if let Err(reason) =
                    initialize_connection(handle, &mut established_state, stream).await
                {
                    connection_failed(
                        &mut established_state,
                        "Failed to initialize RLPx connection",
                        reason,
                    )
                    .await;
                    self.inner_state = InnerState::Established(established_state);
                    Ok(NoSuccess(self))
                } else {
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
                Self::CastMsg::SendNewPooledTxHashes => {
                    send_new_pooled_tx_hashes(established_state).await
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
                    .lock()
                    .await
                    .replace_peer(established_state.node.node_id());
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
) -> Result<(), RLPxError>
where
    S: Unpin + Send + Stream<Item = Result<Message, RLPxError>> + 'static,
{
    post_handshake_checks(state.table.clone()).await?;

    exchange_hello_messages(state, &mut stream).await?;

    // Handshake OK: handle connection
    // Create channels to communicate directly to the peer
    let (peer_channels, sender) = PeerChannels::create(handle.clone());

    // Updating the state to establish the backend channel
    state.backend_channel = Some(sender);

    // NOTE: if the peer came from the discovery server it will already be inserted in the table
    // but that might not always be the case, so we try to add it to the table
    // Note: we don't ping the node we let the validation service do its job
    {
        let mut table_lock = state.table.lock().await;
        table_lock.insert_node_forced(state.node.clone());
        table_lock.init_backend_communication(
            state.node.node_id(),
            peer_channels,
            state.capabilities.clone(),
            state.inbound,
        );
    }
    init_capabilities(state, &mut stream).await?;
    log_peer_debug(&state.node, "Peer connection initialized.");

    // Send transactions transaction hashes from mempool at connection start
    send_new_pooled_tx_hashes(state).await?;

    // Periodic broadcast check repeated events.
    send_interval(
        TX_BROADCAST_INTERVAL,
        handle.clone(),
        CastMessage::SendNewPooledTxHashes,
    );

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
        |msg: Message| CastMessage::PeerMessage(msg),
        stream,
    );

    if state.negotiated_eth_capability.is_some() {
        let stream = BroadcastStream::new(state.connection_broadcast_send.subscribe());
        let message_builder =
            |(id, msg): (Id, Arc<Message>)| CastMessage::BroadcastMessage(id, msg);
        spawn_listener(handle.clone(), message_builder, stream);
    }

    Ok(())
}

async fn send_new_pooled_tx_hashes(state: &mut Established) -> Result<(), RLPxError> {
    if SUPPORTED_ETH_CAPABILITIES
        .iter()
        .any(|cap| state.capabilities.contains(cap))
    {
        let mut tx_hashes = mem::take(
            &mut state
                .tx_bcast_state
                .queue_and_filter
                .lock()
                .expect("poisoned mutex")
                .0,
        );
        if !tx_hashes.is_empty() {
            let mut transactions = Vec::new();
            let mut iter = tx_hashes.drain().peekable();
            while iter.peek().is_some() {
                let tx_chunk = (&mut iter).take(NEW_POOLED_TRANSACTION_HASHES_SOFT_LIMIT);
                let tx_chunk_len = tx_chunk.len();

                transactions.reserve(tx_chunk_len);
                for tx in tx_chunk {
                    transactions.push(
                        match state.blockchain.mempool.get_transaction_by_hash(tx)? {
                            Some(tx) => tx,
                            None => continue,
                        },
                    );
                }

                send(
                    state,
                    Message::NewPooledTransactionHashes(NewPooledTransactionHashes::new(
                        &transactions,
                        &state.blockchain,
                    )?),
                )
                .await?;
                log_peer_debug(
                    &state.node,
                    &format!("Sent {tx_chunk_len} transaction hashes to peer"),
                );

                transactions.clear();
            }
        }
    }
    Ok(())
}

async fn send_block_range_update(state: &mut Established) -> Result<(), RLPxError> {
    // BlockRangeUpdate was introduced in eth/69
    if let Some(eth) = &state.negotiated_eth_capability {
        if eth.version >= 69 {
            log_peer_debug(&state.node, "Sending BlockRangeUpdate");
            let update = BlockRangeUpdate::new(&state.storage).await?;
            let lastet_block = update.lastest_block;
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
        let status = StatusMessage::new(&state.storage, &eth).await?;
        log_peer_debug(&state.node, "Sending status");
        send(state, Message::Status(status)).await?;
        // The next immediate message in the ETH protocol is the
        // status, reference here:
        // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
        let msg = match receive(stream).await {
            Some(msg) => msg?,
            None => return Err(RLPxError::Disconnected()),
        };
        match msg {
            Message::Status(msg_data) => {
                log_peer_debug(&state.node, "Received Status");
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

async fn post_handshake_checks(
    table: Arc<Mutex<crate::kademlia::KademliaTable>>,
) -> Result<(), RLPxError> {
    // Check if connected peers exceed the limit
    let peer_count = {
        let table_lock = table.lock().await;
        table_lock.count_connected_peers()
    };

    if peer_count >= MAX_PEERS_TCP_CONNECTIONS {
        return Err(RLPxError::DisconnectSent(DisconnectReason::TooManyPeers));
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

async fn connection_failed(state: &mut Established, error_text: &str, error: RLPxError) {
    // Send disconnect message only if error is different than RLPxError::DisconnectRequested
    // because if it is a DisconnectRequested error it means that the peer requested the disconnection, not us.
    if !matches!(error, RLPxError::DisconnectReceived(_)) {
        send_disconnect_message(state, match_disconnect_reason(&error)).await;
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
            state.table.lock().await.replace_peer(state.node.node_id());
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
            log_peer_debug(
                &state.node,
                &format!("Received Disconnect: {}", msg_data.reason()),
            );
            // TODO handle the disconnection request
            return Err(RLPxError::DisconnectReceived(msg_data.reason()));
        }
        Message::Ping(_) => {
            log_peer_debug(&state.node, "Sending pong message");
            send(state, Message::Pong(PongMessage {})).await?;
        }
        Message::Pong(_) => {
            // We ignore received Pong messages
        }
        Message::Status(msg_data) => {
            if let Some(eth) = &state.negotiated_eth_capability {
                backend::validate_status(msg_data, &state.storage, eth).await?
            };
        }
        Message::GetAccountRange(req) => {
            let response = process_account_range_request(req, state.storage.clone())?;
            send(state, Message::AccountRange(response)).await?
        }
        Message::Transactions(txs) if peer_supports_eth => {
            // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#transactions-0x02
            if state.blockchain.is_synced() {
                let mut valid_txs = vec![];
                for tx in txs.transactions {
                    // Mark as broadcasted for the sender so we don't include it in the next
                    // `SendNewPooledTxHashes` message to this peer. Doing so violates spec.
                    // For broadcast itself, `handle_broadcast` filters by task id already.
                    {
                        let mut bcast_state = state
                            .tx_bcast_state
                            .queue_and_filter
                            .lock()
                            .expect("poisoned mutex");
                        bcast_state.0.remove(&tx.hash());
                        bcast_state.1.insert(tx.hash());
                    }

                    if let Err(e) = state.blockchain.add_transaction_to_pool(tx.clone()).await {
                        log_peer_warn(&state.node, &format!("Error adding transaction: {e}"));
                        continue;
                    }
                    valid_txs.push(tx);
                }
                // FIXME(#1131): we're supposed to send `Transaction` message only to a random
                // subset and send only the hashes to everyone else.
                // Consider sending to the sqrt of the number of peers like geth does.
                if !valid_txs.is_empty() {
                    log_peer_debug(
                        &state.node,
                        &format!("Broadcasted {} transactions to peers", valid_txs.len()),
                    );
                    broadcast_message(state, Message::Transactions(Transactions::new(valid_txs)))?;
                }
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
                let response = Receipts::new(id, receipts, eth)?;
                send(state, Message::Receipts(response)).await?;
            }
        }
        Message::BlockRangeUpdate(update) => {
            if update.earliest_block > update.lastest_block {
                return Err(RLPxError::InvalidBlockRange);
            }
            //TODO implement the logic
            log_peer_debug(
                &state.node,
                &format!(
                    "Range block update: {} to {}",
                    update.earliest_block, update.lastest_block
                ),
            );
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
                msg.handle(&state.node, &state.blockchain).await?;
            }
        }
        Message::GetStorageRanges(req) => {
            let response = process_storage_ranges_request(req, state.storage.clone())?;
            send(state, Message::StorageRanges(response)).await?
        }
        Message::GetByteCodes(req) => {
            let response = process_byte_codes_request(req, state.storage.clone())?;
            send(state, Message::ByteCodes(response)).await?
        }
        Message::GetTrieNodes(req) => {
            let response = process_trie_nodes_request(req, state.storage.clone())?;
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
        | message @ Message::Receipts(_) => {
            state
                .backend_channel
                .as_mut()
                // TODO: this unwrap() is temporary, until we fix the backend process to use spawned
                .expect("Backend channel is not available")
                .send(message)
                .await?
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
            Message::Transactions(txs) => {
                let filtered = {
                    let mut bcast_state = state
                        .tx_bcast_state
                        .queue_and_filter
                        .lock()
                        .expect("poisoned mutex");
                    txs.transactions
                        .iter()
                        .filter(|&tx| {
                            bcast_state.0.remove(&tx.hash());
                            bcast_state.1.insert(tx.hash())
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                };
                if !filtered.is_empty() {
                    log_peer_debug(
                        &state.node,
                        &format!(
                            "Sending {} transactions to peer from broadcast",
                            filtered.len()
                        ),
                    );
                    let new_msg = Message::Transactions(Transactions {
                        transactions: filtered,
                    });
                    send(state, new_msg).await?;
                }
            }
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
        txs_msg @ Message::Transactions(_) => {
            let txs = Arc::new(txs_msg);
            let task_id = tokio::task::id();
            let Ok(_) = state.connection_broadcast_send.send((task_id, txs)) else {
                let error_message = "Could not broadcast received transactions";
                log_peer_error(&state.node, error_message);
                return Err(RLPxError::BroadcastError(error_message.to_owned()));
            };
            Ok(())
        }
        l2_msg @ Message::L2(_) => broadcast_l2_message(state, l2_msg),
        msg => {
            let error_message = format!("Broadcasting for msg: {msg} is not supported");
            log_peer_error(&state.node, &error_message);
            Err(RLPxError::BroadcastError(error_message))
        }
    }
}
