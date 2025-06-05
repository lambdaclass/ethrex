use std::{collections::HashSet, net::SocketAddr, sync::Arc, time::Instant};

use ethrex_blockchain::Blockchain;
use ethrex_common::{types::{MempoolTransaction, Transaction}, H256};
use ethrex_storage::Store;
use k256::{ecdsa::SigningKey, PublicKey};
use rand::random;
use spawned_concurrency::tasks::{CallResponse, CastResponse, GenServer, GenServerHandle, GenServerInMsg};
use spawned_rt::tasks::mpsc::Sender;
use tokio::{net::{TcpSocket, TcpStream}, sync::{broadcast, Mutex}};
use tokio_util::codec::Framed;
use tokio_stream::StreamExt;
use futures::SinkExt;
use tracing::{debug, error};

use crate::{discv4::server::MAX_PEERS_TCP_CONNECTIONS, kademlia::{KademliaTable, PeerChannels}, network::P2PContext, rlpx::{error::RLPxError, eth::{backend, blocks::{BlockBodies, BlockHeaders}, receipts::{GetReceipts, Receipts}, transactions::{GetPooledTransactions, NewPooledTransactionHashes, Transactions}}, message::Message, p2p::{self, Capability, DisconnectMessage, DisconnectReason, PongMessage, SUPPORTED_ETH_CAPABILITIES, SUPPORTED_P2P_CAPABILITIES, SUPPORTED_SNAP_CAPABILITIES}, utils::{log_peer_debug, log_peer_error, log_peer_warn}}, snap::{process_account_range_request, process_byte_codes_request, process_storage_ranges_request, process_trie_nodes_request}, types::Node};

use super::{codec::RLPxCodec, handshake};

pub(crate) type RLPxConnBroadcastSender = broadcast::Sender<(tokio::task::Id, Arc<Message>)>;

const PERIODIC_TX_BROADCAST_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
const PERIODIC_TASKS_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

type MsgResult = Result<OutMessage, RLPxError>;
type RLPxConnectionHandle = GenServerHandle<RLPxConnection>;

#[derive(Clone)]
pub struct RLPxConnectionState {
    pub(crate) signer: SigningKey,
    pub(crate)  node: Node,
    pub(crate)  framed: Arc<Mutex<Framed<TcpStream, RLPxCodec>>>,
    pub(crate)  storage: Store,
    pub(crate)  blockchain: Arc<Blockchain>,
    pub(crate)  capabilities: Vec<Capability>,
    pub(crate)  negotiated_eth_capability: Option<Capability>,
    pub(crate)  negotiated_snap_capability: Option<Capability>,
    pub(crate)  next_periodic_ping: Instant,
    pub(crate)  next_tx_broadcast: Instant,
    pub(crate)  broadcasted_txs: HashSet<H256>,
    pub(crate)  client_version: String,
    //// Send end of the channel used to broadcast messages
    //// to other connected peers, is ok to have it here,
    //// since internally it's an Arc.
    //// The ID is to ignore the message sent from the same task.
    //// This is used both to send messages and to received broadcasted
    //// messages from other connections (sent from other peers).
    //// The receive end is instantiated after the handshake is completed
    //// under `handle_peer`.
    connection_broadcast_send: RLPxConnBroadcastSender,
    table: Arc<Mutex<KademliaTable>>,
    inbound: bool,
}

impl RLPxConnectionState {
    pub async fn new_as_receiver(context: P2PContext, peer_addr: SocketAddr, stream: TcpStream) -> Result<Self, RLPxError> {
        let (framed, remote_key) = handshake::new_as_receiver(context.clone(), stream).await?;
        let node = Node::new(
            peer_addr.ip(),
            peer_addr.port(),
            peer_addr.port(),
            remote_key,
        );
        Ok(Self{
            signer: context.signer,
            node,
            framed: Arc::new(Mutex::new(framed)),
            storage: context.storage,
            blockchain: context.blockchain,
            capabilities: vec![],
            negotiated_eth_capability: None,
            negotiated_snap_capability: None,
            next_periodic_ping: Instant::now() + PERIODIC_TASKS_CHECK_INTERVAL,
            next_tx_broadcast: Instant::now() + PERIODIC_TX_BROADCAST_INTERVAL,
            broadcasted_txs: HashSet::new(),
            client_version: context.client_version,
            connection_broadcast_send: context.broadcast,
            table: context.table,
            inbound: true,
        })
    }

    pub async fn new_as_initiator(context: P2PContext, node: &Node, stream: TcpStream) -> Result<Self, RLPxError> {
        let framed = handshake::new_as_initiator(context.clone(), node, stream).await?;
        Ok(Self{
            signer: context.signer,
            node: node.clone(),
            framed: Arc::new(Mutex::new(framed)),
            storage: context.storage,
            blockchain: context.blockchain,
            capabilities: vec![],
            negotiated_eth_capability: None,
            negotiated_snap_capability: None,
            next_periodic_ping: Instant::now() + PERIODIC_TASKS_CHECK_INTERVAL,
            next_tx_broadcast: Instant::now() + PERIODIC_TX_BROADCAST_INTERVAL,
            broadcasted_txs: HashSet::new(),
            client_version: context.client_version,
            connection_broadcast_send: context.broadcast,
            table: context.table,
            inbound: false,
        })
    }
}

#[derive(Clone)]
pub enum InMessage {
    PeerMessage(Message),
    BroadcastMessage,
    BackendMessage,
    PeriodicCheck,
}

#[allow(dead_code)]
#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
    Error,
}
    
pub struct RLPxConnection {}

impl RLPxConnection {

    pub async fn spawn_as_receiver(context: P2PContext, peer_addr: SocketAddr, stream: TcpStream) {
        match RLPxConnectionState::new_as_receiver(context, peer_addr, stream).await {
            Ok(mut state) => {
                init(&mut state).await;
                let node = state.node.clone();
                let framed = state.framed.clone();
                let conn = RLPxConnection::start(state);
                // Send Init message to perform post handshake and initial checks.
                spawn_listener(conn, node, framed);
            }
            Err(error) => error!("Error starting RLPxConnection: {}", error),
        };
    }

    pub async fn spawn_as_initiator(context: P2PContext, node: &Node) {
        let addr = SocketAddr::new(node.ip, node.tcp_port);
        let stream = match tcp_stream(addr).await {
            Ok(result) => result,
            Err(error) => {
                log_peer_debug(node, &format!("Error creating tcp connection {error}"));
                context.table.lock().await.replace_peer(node.node_id());
                return;
            }
        };
        let table = context.table.clone();
        match RLPxConnectionState::new_as_initiator(context, node, stream).await {
            Ok(mut state) => {
                init(&mut state).await;
                let node = state.node.clone();
                let framed = state.framed.clone();
                let conn = RLPxConnection::start(state);
                // Send Init message to perform post handshake and initial checks.
                spawn_listener(conn, node, framed);
            }
            Err(error) => {
                log_peer_debug(node, &format!("Error starting RLPxConnection: {error}"));
                table.lock().await.replace_peer(node.node_id());
            }
        };
    }

    // pub async fn spawn_as_receiver_old(context: P2PContext, peer_addr: SocketAddr, stream: TcpStream) {
    //     let table = context.table.clone();
    //     match handshake::as_receiver(context, peer_addr, stream).await {
    //         Ok(mut conn) => conn.start(table, true).await,
    //         Err(e) => {
    //             debug!("Error creating tcp connection with peer at {peer_addr}: {e}")
    //         }
    //     }
    // }
}

impl GenServer for RLPxConnection {
    type InMsg = InMessage;
    type OutMsg = MsgResult;
    type State = RLPxConnectionState;
    type Error = RLPxError;

    fn new() -> Self {
        Self {}
    }


    async fn handle_call(
        &mut self,
        message: Self::InMsg,
        _tx: &Sender<GenServerInMsg<Self>>,
        state: &mut Self::State,
    ) -> CallResponse<Self::OutMsg> {
        match message.clone() {
            InMessage::PeerMessage(message) => {
                let _ = handle_message(state, message).await;
                CallResponse::Reply(Ok(OutMessage::Done))
            },
            InMessage::BroadcastMessage => todo!(),
            InMessage::BackendMessage => todo!(),
            InMessage::PeriodicCheck => todo!(),
        }
    }

    async fn handle_cast(
        &mut self,
        _message: Self::InMsg,
        _tx: &Sender<GenServerInMsg<Self>>,
        _state: &mut Self::State,
    ) -> CastResponse {
        CastResponse::NoReply
    }
}

async fn tcp_stream(addr: SocketAddr) -> Result<TcpStream, std::io::Error> {
    TcpSocket::new_v4()?.connect(addr).await
}

async fn init(state: &mut RLPxConnectionState) {
        log_peer_debug(&state.node, "Starting RLPx connection");

    if let Err(reason) = post_handshake_checks(state.table.clone()).await {
        connection_failed(
            state,
            "Post handshake validations failed",
            RLPxError::DisconnectSent(reason),
        )
        .await;
        return;
    }

    if let Err(e) = exchange_hello_messages(state).await {
        connection_failed(state, "Hello messages exchange failed", e)
            .await;
    } else {
        // Handshake OK: handle connection
        // Create channels to communicate directly to the peer
        let (peer_channels, sender, receiver) = PeerChannels::create();

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
        // TODO Handle this unwrap
        let _ = init_peer_conn(state).await.unwrap();
        log_peer_debug(&state.node, "Started peer main loop");
        // Subscribe this connection to the broadcasting channel.
        let mut broadcaster_receive = if state.negotiated_eth_capability.is_some() {
            Some(state.connection_broadcast_send.subscribe())
        } else {
            None
        };
        // Send transactions transaction hashes from mempool at connection start
        send_new_pooled_tx_hashes(state).await.unwrap();

        // TCP listener loop
        let framed = state.framed.clone();
        let node = state.node.clone();
        
        // if let Err(e) = self.connection_loop(sender, receiver).await {
        //     self.connection_failed("Error during RLPx connection", e, state.table.clone())
        //         .await;
        // }
    }
}

async fn send_new_pooled_tx_hashes(state: &mut RLPxConnectionState) -> Result<(), RLPxError> {
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
                send(state, Message::NewPooledTransactionHashes(
                    NewPooledTransactionHashes::new(vec![(*tx).clone()], &state.blockchain)?,
                ))
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

async fn init_peer_conn(state: &mut RLPxConnectionState) -> Result<(), RLPxError> {
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

async fn send_disconnect_message(state: &mut RLPxConnectionState, reason: Option<DisconnectReason>) {
    send(state, Message::Disconnect(DisconnectMessage { reason }))
        .await
        .unwrap_or_else(|_| {
            log_peer_debug(
                &state.node,
                &format!("Could not send Disconnect message: ({:?}).", reason),
            );
        });
}

async fn connection_failed(
    state: &mut RLPxConnectionState,
    error_text: &str,
    error: RLPxError,
) {
    log_peer_debug(&state.node, &format!("{error_text}: ({error})"));

    // Send disconnect message only if error is different than RLPxError::DisconnectRequested
    // because if it is a DisconnectRequested error it means that the peer requested the disconnection, not us.
    if !matches!(error, RLPxError::DisconnectReceived(_)) {
        send_disconnect_message(state, match_disconnect_reason(&error))
            .await;
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

async fn exchange_hello_messages(state: &mut RLPxConnectionState) -> Result<(), RLPxError> {
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
                state.negotiated_snap_capability =
                    Some(Capability::snap(negotiated_snap_version));
            }

            state.node.version = Some(hello_message.client_id);

            Ok(())
        }
        Message::Disconnect(disconnect) => {
            Err(RLPxError::DisconnectReceived(disconnect.reason()))
        }
        _ => {
            // Fail if it is not a hello message
            Err(RLPxError::BadRequest("Expected Hello message".to_string()))
        }
    }
}

async fn send(state: &mut RLPxConnectionState, message: Message) -> Result<(), RLPxError> {
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
async fn receive(state: &mut RLPxConnectionState) -> Option<Result<Message, RLPxError>> {
    state.framed.lock().await.next().await
}

fn spawn_listener(mut conn: RLPxConnectionHandle, node: Node, framed: Arc<Mutex<Framed<TcpStream, RLPxCodec>>>) {
    spawned_rt::tasks::spawn(async move {
        loop {
            match framed.lock().await.next().await {
                Some(message) => match message {
                    Ok(message) => {
                        log_peer_debug(&node, &format!("Received message {}", message));
                        conn.call(InMessage::PeerMessage(message)).await;
                        return Ok(())
                    },
                    Err(e) => {
                        log_peer_debug(&node, &format!("Received RLPX Error in msg {}", e));
                        return Err(e);
                    }
                }
                None => todo!(),
            }
        }
    });
}

async fn handle_message(
    state: &mut RLPxConnectionState,
    message: Message,
) -> Result<(), RLPxError> {
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
        Message::NewPooledTransactionHashes(new_pooled_transaction_hashes)
            if peer_supports_eth =>
        {
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
        // message @ Message::AccountRange(_)
        // | message @ Message::StorageRanges(_)
        // | message @ Message::ByteCodes(_)
        // | message @ Message::TrieNodes(_)
        // | message @ Message::BlockBodies(_)
        // | message @ Message::BlockHeaders(_)
        // | message @ Message::Receipts(_) => sender.send(message).await?,
        // TODO: Add new message types and handlers as they are implemented
        message => return Err(RLPxError::MessageNotHandled(format!("{message}"))),
    };
    Ok(())
}

fn broadcast_message(state: &RLPxConnectionState, msg: Message) -> Result<(), RLPxError> {
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