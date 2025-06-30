use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use ethrex_blockchain::Blockchain;
use ethrex_common::{
    H256,
    types::{MempoolTransaction, Transaction},
};
use ethrex_storage::Store;
use futures::{SinkExt as _, Stream, stream::SplitSink};
use k256::{PublicKey, ecdsa::SigningKey};
use rand::random;
use spawned_concurrency::tasks::{
    CallResponse, CastResponse, GenServer, GenServerHandle, send_interval,
};
use tokio::{
    net::TcpStream,
    sync::{Mutex, broadcast, mpsc::Sender},
    task,
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
        message::Message,
        p2p::{
            self, Capability, DisconnectMessage, DisconnectReason, PingMessage, PongMessage,
            SUPPORTED_ETH_CAPABILITIES, SUPPORTED_P2P_CAPABILITIES, SUPPORTED_SNAP_CAPABILITIES,
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

pub(crate) type RLPxConnBroadcastSender = broadcast::Sender<(tokio::task::Id, Arc<Message>)>;

type MsgResult = Result<OutMessage, RLPxError>;
type RLPxConnectionHandle = GenServerHandle<RLPxConnection>;

#[derive(Clone)]
pub struct RLPxConnectionState(pub InnerState);

#[derive(Clone)]
pub struct Initiator {
    pub(crate) context: P2PContext,
    pub(crate) node: Node,
}

#[derive(Clone)]
pub struct Receiver {
    pub(crate) context: P2PContext,
    pub(crate) peer_addr: SocketAddr,
    pub(crate) stream: Arc<TcpStream>,
}

#[derive(Clone)]
pub struct Established {
    pub(crate) signer: SigningKey,
    pub(crate) sink: Arc<Mutex<SplitSink<Framed<TcpStream, RLPxCodec>, Message>>>,
    pub(crate) node: Node,
    pub(crate) storage: Store,
    pub(crate) blockchain: Arc<Blockchain>,
    pub(crate) capabilities: Vec<Capability>,
    pub(crate) negotiated_eth_capability: Option<Capability>,
    pub(crate) negotiated_snap_capability: Option<Capability>,
    pub(crate) last_block_range_update_block: u64,
    pub(crate) broadcasted_txs: HashSet<H256>,
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
}

#[derive(Clone)]
pub enum InnerState {
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

#[derive(Clone)]
pub enum CallMessage {}

#[derive(Clone)]
#[allow(private_interfaces)]
pub enum CastMessage {
    PeerMessage(Message),
    BackendMessage(Message),
    SendPing,
    SendNewPooledTxHashes,
    BlockRangeUpdate,
    BroadcastMessage(task::Id, Arc<Message>),
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
pub struct RLPxConnection {}

impl RLPxConnection {
    pub async fn spawn_as_receiver(
        context: P2PContext,
        peer_addr: SocketAddr,
        stream: TcpStream,
    ) -> Result<RLPxConnectionHandle, std::io::Error> {
        let state = RLPxConnectionState::new_as_receiver(context, peer_addr, stream);
        Ok(RLPxConnection::start(state))
    }

    pub async fn spawn_as_initiator(
        context: P2PContext,
        node: &Node,
    ) -> Result<RLPxConnectionHandle, std::io::Error> {
        let state = RLPxConnectionState::new_as_initiator(context, node);
        Ok(RLPxConnection::start(state.clone()))
    }
}

impl GenServer for RLPxConnection {
    type CallMsg = CallMessage;
    type CastMsg = CastMessage;
    type OutMsg = MsgResult;
    type State = RLPxConnectionState;
    type Error = RLPxError;

    fn new() -> Self {
        Self {}
    }

    async fn init(
        &mut self,
        handle: &GenServerHandle<Self>,
        mut state: Self::State,
    ) -> Result<Self::State, Self::Error> {
        let (mut established_state, stream) = handshake::perform(state.0).await?;
        log_peer_debug(&established_state.node, "Starting RLPx connection");

        if let Err(reason) = initialize_connection(handle, &mut established_state, stream).await {
            connection_failed(
                &mut established_state,
                "Failed to initialize RLPx connection",
                reason,
            )
            .await;
            Err(RLPxError::Disconnected())
        } else {
            // New state
            state.0 = InnerState::Established(established_state);
            Ok(state)
        }
    }

    async fn handle_call(
        &mut self,
        _message: Self::CallMsg,
        _handle: &RLPxConnectionHandle,
        state: Self::State,
    ) -> CallResponse<Self> {
        CallResponse::Reply(state, Ok(OutMessage::Done))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &RLPxConnectionHandle,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        if let InnerState::Established(mut established_state) = state.0.clone() {
            match message {
                // TODO: handle all these "let _"
                // See https://github.com/lambdaclass/ethrex/issues/3375
                Self::CastMsg::PeerMessage(message) => {
                    log_peer_debug(
                        &established_state.node,
                        &format!("Received peer message: {message}"),
                    );
                    let _ = handle_peer_message(&mut established_state, message).await;
                }
                Self::CastMsg::BackendMessage(message) => {
                    log_peer_debug(
                        &established_state.node,
                        &format!("Received backend message: {message}"),
                    );
                    let _ = handle_backend_message(&mut established_state, message).await;
                }
                Self::CastMsg::SendPing => {
                    let _ = send(&mut established_state, Message::Ping(PingMessage {})).await;
                    log_peer_debug(&established_state.node, "Ping sent");
                }
                Self::CastMsg::SendNewPooledTxHashes => {
                    let _ = send_new_pooled_tx_hashes(&mut established_state).await;
                }
                Self::CastMsg::BroadcastMessage(id, msg) => {
                    log_peer_debug(
                        &established_state.node,
                        &format!("Received broadcasted message: {msg}"),
                    );
                    let _ = handle_broadcast(&mut established_state, (id, msg)).await;
                }
                Self::CastMsg::BlockRangeUpdate => {
                    log_peer_debug(&established_state.node, "Block Range Update");
                    let _ = handle_block_range_update(&mut established_state).await;
                }
            }
            // Update the state
            state.0 = InnerState::Established(established_state);
            CastResponse::NoReply(state)
        } else {
            // Received a Cast message but connection is not ready. Log an error but keep the connection alive.
            error!("Connection not yet established");
            CastResponse::NoReply(state)
        }
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

    spawn_listener(handle.clone(), &state.node, stream);

    spawn_broadcast_listener(handle.clone(), state);

    Ok(())
}

async fn send_new_pooled_tx_hashes(state: &mut Established) -> Result<(), RLPxError> {
    if SUPPORTED_ETH_CAPABILITIES
        .iter()
        .any(|cap| state.capabilities.contains(cap))
    {
        let filter =
            |tx: &Transaction| -> bool { !state.broadcasted_txs.contains(&tx.compute_hash()) };
        let txs: Vec<MempoolTransaction> = state
            .blockchain
            .mempool
            .filter_transactions_with_filter_fn(&filter)?
            .into_values()
            .flatten()
            .collect();
        if !txs.is_empty() {
            let tx_count = txs.len();
            for tx in txs {
                send(
                    state,
                    Message::NewPooledTransactionHashes(NewPooledTransactionHashes::new(
                        vec![(*tx).clone()],
                        &state.blockchain,
                    )?),
                )
                .await?;
                // Possible improvement: the mempool already knows the hash but the filter function does not return it
                state.broadcasted_txs.insert((*tx).compute_hash());
            }
            log_peer_debug(
                &state.node,
                &format!("Sent {tx_count} transactions to peer"),
            );
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
    log_peer_debug(&state.node, &format!("{error_text}: ({error})"));

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

    let _ = state.sink.lock().await.close().await;
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
    let supported_capabilities: Vec<Capability> = [
        &SUPPORTED_ETH_CAPABILITIES[..],
        &SUPPORTED_SNAP_CAPABILITIES[..],
        &SUPPORTED_P2P_CAPABILITIES[..],
    ]
    .concat();
    let hello_msg = Message::Hello(p2p::HelloMessage::new(
        supported_capabilities,
        PublicKey::from(state.signer.verifying_key()),
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
                match cap.protocol {
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

async fn send(state: &mut Established, message: Message) -> Result<(), RLPxError> {
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

// TODO replace this spawn, once it's implemented in spawned
// See https://github.com/lambdaclass/ethrex/issues/3387 and
// https://github.com/lambdaclass/spawned/issues/17
fn spawn_listener<S>(mut conn: RLPxConnectionHandle, node: &Node, mut stream: S)
where
    S: Unpin + Send + Stream<Item = Result<Message, RLPxError>> + 'static,
{
    let node = node.clone();
    spawned_rt::tasks::spawn(async move {
        loop {
            match stream.next().await {
                Some(Ok(message)) => {
                    let _ = conn.cast(CastMessage::PeerMessage(message)).await;
                }
                Some(Err(e)) => {
                    log_peer_debug(&node, &format!("Received RLPX Error in msg {e}"));
                    break;
                }
                // `None` does not neccessary means EOF, so we will keep the loop running
                // (See Framed::new)
                None => (),
            }
        }
    });
}

// TODO Maybe provide a similar mechanism for this listener, or remove it when
// Broadcast is handled in a spawned GenServer
// See https://github.com/lambdaclass/ethrex/issues/3387 and
// https://github.com/lambdaclass/spawned/issues/17 and
// https://github.com/lambdaclass/ethrex/issues/3388
fn spawn_broadcast_listener(mut handle: RLPxConnectionHandle, state: &mut Established) {
    // Subscribe this connection to the broadcasting channel.
    // TODO currently spawning a listener task that will suscribe to a broadcast channel and
    // create RLPxConnection Broadcast messages to send the Genserver
    // We have to improve this mechanism to avoid manual creation of channels and subscriptions
    // (That is, we should have a spawned-based broadcaster or maybe the backend should handle the
    // transactions propagation)
    if state.negotiated_eth_capability.is_some() {
        let mut receiver = state.connection_broadcast_send.subscribe();
        spawned_rt::tasks::spawn(async move {
            loop {
                if let Ok((id, msg)) = receiver.recv().await {
                    let _ = handle.cast(CastMessage::BroadcastMessage(id, msg)).await;
                };
            }
        });
    };
}

async fn handle_peer_message(state: &mut Established, message: Message) -> Result<(), RLPxError> {
    let peer_supports_eth = state.negotiated_eth_capability.is_some();
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
            if state.blockchain.is_synced() {
                let mut valid_txs = vec![];
                for tx in &txs.transactions {
                    if let Err(e) = state.blockchain.add_transaction_to_pool(tx.clone()).await {
                        log_peer_warn(&state.node, &format!("Error adding transaction: {e}"));
                        continue;
                    }
                    valid_txs.push(tx.clone());
                }
                if !valid_txs.is_empty() {
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
                    if let Err(error) = msg.validate_requested(requested).await {
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
                // TODO(#1131): Avoid cloning this vector.
                let cloned = txs.transactions.clone();
                let new_msg = Message::Transactions(Transactions {
                    transactions: cloned,
                });
                send(state, new_msg).await?;
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

fn broadcast_message(state: &Established, msg: Message) -> Result<(), RLPxError> {
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
        msg => {
            let error_message = format!("Broadcasting for msg: {msg} is not supported");
            log_peer_error(&state.node, &error_message);
            Err(RLPxError::BroadcastError(error_message))
        }
    }
}
