use std::{collections::HashSet, net::SocketAddr, sync::Arc};

use ethrex_blockchain::Blockchain;
use ethrex_common::{
    types::{MempoolTransaction, Transaction},
    H256,
};
use ethrex_storage::Store;
use futures::SinkExt;
use k256::{ecdsa::SigningKey, PublicKey};
use rand::random;
use spawned_concurrency::tasks::{
    send_after, CallResponse, CastResponse, GenServer, GenServerHandle,
};
use tokio::{
    net::{TcpSocket, TcpStream},
    sync::{broadcast, mpsc::Sender, Mutex},
};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::{debug, error, info};

use crate::{
    discv4::server::MAX_PEERS_TCP_CONNECTIONS,
    kademlia::{KademliaTable, PeerChannels},
    network::P2PContext,
    rlpx::{
        error::RLPxError,
        eth::{
            backend,
            blocks::{BlockBodies, BlockHeaders},
            receipts::{GetReceipts, Receipts},
            transactions::{GetPooledTransactions, NewPooledTransactionHashes, Transactions},
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

use super::{codec::RLPxCodec, handshake};

pub(crate) const PERIODIC_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
pub(crate) const PERIODIC_TX_BROADCAST_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(500);

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
}

#[derive(Clone)]
pub struct Established {
    pub(crate) signer: SigningKey,
    pub(crate) framed: Arc<Mutex<Framed<TcpStream, RLPxCodec>>>,
    pub(crate) node: Node,
    pub(crate) storage: Store,
    pub(crate) blockchain: Arc<Blockchain>,
    pub(crate) capabilities: Vec<Capability>,
    pub(crate) negotiated_eth_capability: Option<Capability>,
    pub(crate) negotiated_snap_capability: Option<Capability>,
    pub(crate) broadcasted_txs: HashSet<H256>,
    pub(crate) client_version: String,
    //// Send end of the channel used to broadcast messages
    //// to other connected peers, is ok to have it here,
    //// since internally it's an Arc.
    //// The ID is to ignore the message sent from the same task.
    //// This is used both to send messages and to received broadcasted
    //// messages from other connections (sent from other peers).
    //// The receive end is instantiated after the handshake is completed
    //// under `handle_peer`.
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
    pub fn new_as_receiver(context: P2PContext, peer_addr: SocketAddr) -> Self {
        Self(InnerState::Receiver(Receiver { context, peer_addr }))
    }

    pub fn new_as_initiator(context: P2PContext, node: &Node) -> Self {
        Self(InnerState::Initiator(Initiator {
            context,
            node: node.clone(),
        }))
    }
}

pub enum CallMessage {
    Init(TcpStream),
}

pub enum CastMessage {
    PeerMessage(Message),
    BroadcastMessage,
    BackendMessage(Message),
    SendPing,
    SendNewPooledTxHashes,
}

#[derive(Clone)]
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
    pub async fn spawn_as_receiver(context: P2PContext, peer_addr: SocketAddr, stream: TcpStream) {
        info!("spawn_as_receiver");
        let state = RLPxConnectionState::new_as_receiver(context, peer_addr);
        info!("r new state");
        let mut conn = RLPxConnection::start(state);
        info!("r connected");
        match conn.call(CallMessage::Init(stream)).await {
            Ok(Ok(OutMessage::InitResponse { node, framed })) => {
                info!("r listener");
                spawn_listener(conn, node, framed);
                info!("r done");
            }
            Ok(Ok(_)) => error!("Unexpected response from connection"),
            Ok(Err(error)) => error!("Error starting RLPxConnection: {:?}", error),
            Err(error) => error!("Unhandled error starting RLPxConnection: {:?}", error),
        }
    }

    pub async fn spawn_as_initiator(context: P2PContext, node: &Node) {
        info!("spawn_as_initiator");
        let addr = SocketAddr::new(node.ip, node.tcp_port);
        let stream = match tcp_stream(addr).await {
            Ok(result) => result,
            Err(error) => {
                log_peer_debug(node, &format!("Error creating tcp connection {error}"));
                context.table.lock().await.replace_peer(node.node_id());
                return;
            }
        };
        info!("i stream");
        let state = RLPxConnectionState::new_as_initiator(context, node);
        info!("i new state");
        let mut conn = RLPxConnection::start(state.clone());
        info!("i connected");
        match conn.call(CallMessage::Init(stream)).await {
            Ok(Ok(OutMessage::InitResponse { node, framed })) => {
                info!("i listener");
                spawn_listener(conn, node, framed);
                info!("i done");
            }
            Ok(Ok(_)) => error!("Unexpected response from connection"),
            Ok(Err(error)) => error!("Error starting RLPxConnection: {:?}", error),
            Err(error) => error!("Unhandled error starting RLPxConnection: {:?}", error),
        }
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

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        handle: &RLPxConnectionHandle,
        state: &mut Self::State,
    ) -> CallResponse<Self::OutMsg> {
        match message {
            Self::CallMsg::Init(stream) => match init(state, handle, stream).await {
                Ok((node, framed)) => {
                    CallResponse::Reply(Ok(OutMessage::InitResponse { node, framed }))
                }
                Err(e) => CallResponse::Reply(Err(e)),
            },
        }
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &RLPxConnectionHandle,
        state: &mut Self::State,
    ) -> CastResponse {
        if let InnerState::Established(mut established_state) = state.0.clone() {
            match message {
                // TODO: handle all these "let _"
                Self::CastMsg::PeerMessage(message) => {
                    let _ = handle_peer_message(&mut established_state, message).await;
                }
                Self::CastMsg::BroadcastMessage => todo!(),
                Self::CastMsg::BackendMessage(message) => {
                    let _ = handle_backend_message(&mut established_state, message).await;
                }
                Self::CastMsg::SendPing => {
                    let _ = send(&mut established_state, Message::Ping(PingMessage {})).await;
                    log_peer_debug(&established_state.node, "Ping sent");
                    // TODO this should be removed when spawned_concurrency::tasks::send_interval is implemented.
                    send_after(
                        PERIODIC_PING_INTERVAL,
                        handle.clone(),
                        CastMessage::SendPing,
                    );
                }
                Self::CastMsg::SendNewPooledTxHashes => {
                    let _ = send_new_pooled_tx_hashes(&mut established_state).await;
                    // TODO this should be removed when spawned_concurrency::tasks::send_interval is implemented.
                    send_after(
                        PERIODIC_TX_BROADCAST_INTERVAL,
                        handle.clone(),
                        CastMessage::SendNewPooledTxHashes,
                    );
                }
            }
            // Update the state state
            state.0 = InnerState::Established(established_state);
            CastResponse::NoReply
        } else {
            // Received a Cast message but connection is not ready. Log an error but keep the connection alive.
            error!("Connection not yet established");
            CastResponse::NoReply
        }
    }
}

async fn tcp_stream(addr: SocketAddr) -> Result<TcpStream, std::io::Error> {
    TcpSocket::new_v4()?.connect(addr).await
}

async fn init(
    state: &mut RLPxConnectionState,
    handle: &RLPxConnectionHandle,
    stream: TcpStream,
) -> Result<(Node, Arc<Mutex<Framed<TcpStream, RLPxCodec>>>), RLPxError> {
    let mut established_state = handshake::perform(state, stream).await?;
    log_peer_debug(&established_state.node, "Starting RLPx connection");
    if let Err(reason) = post_handshake_checks(established_state.table.clone()).await {
        connection_failed(
            &mut established_state,
            "Post handshake validations failed",
            RLPxError::DisconnectSent(reason),
        )
        .await;
        return Err(RLPxError::Disconnected());
    }

    if let Err(e) = exchange_hello_messages(&mut established_state).await {
        connection_failed(&mut established_state, "Hello messages exchange failed", e).await;
        return Err(RLPxError::Disconnected());
    } else {
        // Handshake OK: handle connection
        // Create channels to communicate directly to the peer
        let (peer_channels, sender) = PeerChannels::create(handle.clone());

        // Updating the state to establish the backend channel
        established_state.backend_channel = Some(sender);

        // NOTE: if the peer came from the discovery server it will already be inserted in the table
        // but that might not always be the case, so we try to add it to the table
        // Note: we don't ping the node we let the validation service do its job
        {
            let mut table_lock = established_state.table.lock().await;
            table_lock.insert_node_forced(established_state.node.clone());
            table_lock.init_backend_communication(
                established_state.node.node_id(),
                peer_channels,
                established_state.capabilities.clone(),
                established_state.inbound,
            );
        }
        init_peer_conn(&mut established_state).await?;
        log_peer_debug(&established_state.node, "Peer connection initialized.");
        // Subscribe this connection to the broadcasting channel.
        // TODO this channel is not yet connected. Broadcast is not working
        let broadcaster_receive = if established_state.negotiated_eth_capability.is_some() {
            Some(
                established_state
                    .connection_broadcast_send
                    .clone()
                    .subscribe(),
            )
        } else {
            None
        };
        // Send transactions transaction hashes from mempool at connection start
        send_new_pooled_tx_hashes(&mut established_state)
            .await
            .unwrap();

        // TODO this should be replaced with spawned_concurrency::tasks::send_interval once it is properly implemented.
        send_after(
            PERIODIC_TX_BROADCAST_INTERVAL,
            handle.clone(),
            CastMessage::SendNewPooledTxHashes,
        );

        // TODO this should be replaced with spawned_concurrency::tasks::send_interval once it is properly implemented.
        send_after(
            PERIODIC_PING_INTERVAL,
            handle.clone(),
            CastMessage::SendPing,
        );

        let node = established_state.clone().node;
        let framed = established_state.clone().framed;
        // New state
        state.0 = InnerState::Established(established_state);
        Ok((node, framed))
    }
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
                &format!("Sent {} transactions to peer", tx_count),
            );
        }
    }
    Ok(())
}

// async fn connection_loop(
//     &mut self,
//     sender: tokio::sync::mpsc::Sender<Message>,
//     mut receiver: tokio::sync::mpsc::Receiver<Message>,
// ) -> Result<(), RLPxError> {

//     // Start listening for messages,
//     loop {
//         tokio::select! {
//             // Expect a message from the remote peer
//             Some(message) = self.receive() => {
//                 match message {
//                     Ok(message) => {
//                         log_peer_debug(&self.node, &format!("Received message {}", message));
//                         self.handle_message(message, sender.clone()).await?;
//                     },
//                     Err(e) => {
//                         log_peer_debug(&self.node, &format!("Received RLPX Error in msg {}", e));
//                         return Err(e);
//                     }
//                 }
//             }
//             // Expect a message from the backend
//             Some(message) = receiver.recv() => {
//                 log_peer_debug(&self.node, &format!("Sending message {}", message));
//                 self.send(message).await?;
//             }
//             // This is not ideal, but using the receiver without
//             // this function call, causes the loop to take ownwership
//             // of the variable and the compiler will complain about it,
//             // with this function, we avoid that.
//             // If the broadcaster is Some (i.e. we're connected to a peer that supports an eth protocol),
//             // we'll receive broadcasted messages from another connections through a channel, otherwise
//             // the function below will yield immediately but the select will not match and
//             // ignore the returned value.
//             Some(broadcasted_msg) = Self::maybe_wait_for_broadcaster(&mut broadcaster_receive) => {
//                 self.handle_broadcast(broadcasted_msg?).await?
//             }
//             // Allow an interruption to check periodic tasks
//             _ = sleep(PERIODIC_TASKS_CHECK_INTERVAL) => (), // noop
//         }
//         self.check_periodic_tasks().await?;
//     }
// }

async fn init_peer_conn(state: &mut Established) -> Result<(), RLPxError> {
    // Sending eth Status if peer supports it
    if let Some(eth) = state.negotiated_eth_capability.clone() {
        let status = backend::get_status(&state.storage, eth.version).await?;
        log_peer_debug(&state.node, "Sending status");
        send(state, Message::Status(status)).await?;
        // The next immediate message in the ETH protocol is the
        // status, reference here:
        // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
        let msg = match receive(state).await {
            Some(msg) => msg?,
            None => return Err(RLPxError::Disconnected()),
        };
        match msg {
            Message::Status(msg_data) => {
                log_peer_debug(&state.node, "Received Status");
                backend::validate_status(msg_data, &state.storage, eth.version).await?
            }
            Message::Disconnect(disconnect) => {
                return Err(RLPxError::HandshakeError(format!(
                    "Peer disconnected due to: {}",
                    disconnect.reason()
                )))
            }
            _ => {
                return Err(RLPxError::HandshakeError(
                    "Expected a Status message".to_string(),
                ))
            }
        }
    }
    Ok(())
}

async fn post_handshake_checks(
    table: Arc<Mutex<crate::kademlia::KademliaTable>>,
) -> Result<(), DisconnectReason> {
    // Check if connected peers exceed the limit
    let peer_count = {
        let table_lock = table.lock().await;
        table_lock.count_connected_peers()
    };

    if peer_count >= MAX_PEERS_TCP_CONNECTIONS {
        return Err(DisconnectReason::TooManyPeers);
    }

    Ok(())
}

async fn send_disconnect_message(state: &mut Established, reason: Option<DisconnectReason>) {
    send(state, Message::Disconnect(DisconnectMessage { reason }))
        .await
        .unwrap_or_else(|_| {
            log_peer_debug(
                &state.node,
                &format!("Could not send Disconnect message: ({:?}).", reason),
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

    let _ = state.framed.lock().await.close().await;
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

async fn exchange_hello_messages(state: &mut Established) -> Result<(), RLPxError> {
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
    let msg = match receive(state).await {
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
    state.framed.lock().await.send(message).await
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
async fn receive(state: &mut Established) -> Option<Result<Message, RLPxError>> {
    state.framed.lock().await.next().await
}

fn spawn_listener(
    mut conn: RLPxConnectionHandle,
    node: Node,
    framed: Arc<Mutex<Framed<TcpStream, RLPxCodec>>>,
) {
    spawned_rt::tasks::spawn(async move {
        loop {
            match framed.lock().await.next().await {
                Some(message) => match message {
                    Ok(message) => {
                        log_peer_debug(&node, &format!("Received message {}", message));
                        let _ = conn.cast(CastMessage::PeerMessage(message)).await;
                        return Ok(());
                    }
                    Err(e) => {
                        log_peer_debug(&node, &format!("Received RLPX Error in msg {}", e));
                        return Err(e);
                    }
                },
                None => todo!(),
            }
        }
    });
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
                backend::validate_status(msg_data, &state.storage, eth.version).await?
            };
        }
        Message::GetAccountRange(req) => {
            let response = process_account_range_request(req, state.storage.clone())?;
            send(state, Message::AccountRange(response)).await?
        }
        // TODO(#1129) Add the transaction to the mempool once received.
        Message::Transactions(txs) if peer_supports_eth => {
            if state.blockchain.is_synced() {
                let mut valid_txs = vec![];
                for tx in &txs.transactions {
                    if let Err(e) = state.blockchain.add_transaction_to_pool(tx.clone()).await {
                        log_peer_warn(&state.node, &format!("Error adding transaction: {}", e));
                        continue;
                    }
                    valid_txs.push(tx.clone());
                }
                broadcast_message(state, Message::Transactions(Transactions::new(valid_txs)))?;
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
            let mut receipts = Vec::new();
            for hash in block_hashes.iter() {
                receipts.push(state.storage.get_receipts_for_block(hash)?);
            }
            let response = Receipts { id, receipts };
            send(state, Message::Receipts(response)).await?;
        }
        Message::NewPooledTransactionHashes(new_pooled_transaction_hashes) if peer_supports_eth => {
            //TODO(#1415): evaluate keeping track of requests to avoid sending the same twice.
            let hashes =
                new_pooled_transaction_hashes.get_transactions_to_request(&state.blockchain)?;

            //TODO(#1416): Evaluate keeping track of the request-id.
            let request = GetPooledTransactions::new(random(), hashes);
            send(state, Message::GetPooledTransactions(request)).await?;
        }
        Message::GetPooledTransactions(msg) => {
            let response = msg.handle(&state.blockchain)?;
            send(state, Message::PooledTransactions(response)).await?;
        }
        Message::PooledTransactions(msg) if peer_supports_eth => {
            if state.blockchain.is_synced() {
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
                .unwrap()
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
    log_peer_debug(&state.node, &format!("Sending message {}", message));
    send(state, message).await?;
    Ok(())
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
