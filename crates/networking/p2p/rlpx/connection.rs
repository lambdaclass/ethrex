use crate::{
    kademlia::PeerChannels,
    rlpx::{
        error::RLPxError,
        eth::{
            backend,
            blocks::{BlockBodies, BlockHeaders},
            receipts::{GetReceipts, Receipts},
            transactions::{GetPooledTransactions, Transactions},
        },
        frame::RLPxCodec,
        message::Message,
        p2p::{self, Capability, DisconnectMessage, PingMessage, PongMessage},
        utils::{log_peer_debug, log_peer_error},
    },
    snap::{
        process_account_range_request, process_byte_codes_request, process_storage_ranges_request,
        process_trie_nodes_request,
    },
    types::Node,
};
use ethrex_blockchain::mempool::{self};
use ethrex_common::{H256, H512};
use ethrex_storage::Store;
use futures::SinkExt;
use k256::{ecdsa::SigningKey, PublicKey, SecretKey};
use rand::random;
use std::{io::Read, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{
        broadcast::{self, error::RecvError},
        mpsc, Mutex,
    },
    task,
    time::{sleep, Instant},
};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;

use super::utils::log_peer_warn;

const CAP_P2P_5: (Capability, u8) = (Capability::P2p, 5);
const CAP_ETH_68: (Capability, u8) = (Capability::Eth, 68);
const CAP_SNAP_1: (Capability, u8) = (Capability::Snap, 1);
const SUPPORTED_CAPABILITIES: [(Capability, u8); 3] = [CAP_P2P_5, CAP_ETH_68, CAP_SNAP_1];
const PERIODIC_TASKS_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

pub(crate) type Aes256Ctr64BE = ctr::Ctr64BE<aes::Aes256>;

pub(crate) type RLPxConnBroadcastSender = broadcast::Sender<(tokio::task::Id, Arc<Message>)>;

pub(crate) struct RemoteState {
    pub(crate) node_id: H512,
    pub(crate) nonce: H256,
    pub(crate) ephemeral_key: PublicKey,
    pub(crate) init_message: Vec<u8>,
}

pub(crate) struct LocalState {
    pub(crate) nonce: H256,
    pub(crate) ephemeral_key: SecretKey,
    pub(crate) init_message: Vec<u8>,
}

static FILE_PATH: &str = "peers_conn_status.json";

/// stores a connection/disconnection of peers under peers_conn_status.json relative to the execution of the program
/// the connection is stored once the the main loop has started (i.e: after handshake, hello and status messages)
///
/// @param connected: if false, logs disconnection, if true logs connection
async fn update_peer_conn_status(node: Node, connected: bool) {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct PeerConn {
        ip: std::net::IpAddr,
        udp_port: u16,
        tcp_port: u16,
        node_id: H512,
        connected: bool,
    }

    let file_path = std::path::Path::new(FILE_PATH);

    let mut peers = vec![];

    if file_path.exists() {
        let mut file = std::fs::File::open(file_path).expect("Failed to open file");
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .expect("Failed to read file");

        if !contents.is_empty() {
            peers = serde_json::from_str::<Vec<PeerConn>>(&contents).unwrap_or(vec![]);
        }
    }

    let mut found = false;
    for peer in peers.iter_mut() {
        if peer.node_id == node.node_id {
            peer.connected = connected;
            found = true;
            break;
        }
    }
    if !found {
        let new_entry = PeerConn {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
            node_id: node.node_id,
            connected,
        };
        peers.push(new_entry);
    }

    let data = serde_json::to_string(&peers).expect("Failed to serialize JSON");
    std::fs::write(file_path, data).expect("Failed to open file for writing");
}

/// Fully working RLPx connection.
pub(crate) struct RLPxConnection<S> {
    signer: SigningKey,
    node: Node,
    framed: Framed<S, RLPxCodec>,
    storage: Store,
    capabilities: Vec<(Capability, u8)>,
    negotiated_eth_version: u8,
    negotiated_snap_version: u8,
    next_periodic_task_check: Instant,
    /// Send end of the channel used to broadcast messages
    /// to other connected peers, is ok to have it here,
    /// since internally it's an Arc.
    /// The ID is to ignore the message sent from the same task.
    /// This is used both to send messages and to received broadcasted
    /// messages from other connections (sent from other peers).
    /// The receive end is instantiated after the handshake is completed
    /// under `handle_peer`.
    connection_broadcast_send: RLPxConnBroadcastSender,
}

impl<S: AsyncWrite + AsyncRead + std::marker::Unpin> RLPxConnection<S> {
    pub fn new(
        signer: SigningKey,
        node: Node,
        stream: S,
        codec: RLPxCodec,
        storage: Store,
        connection_broadcast: RLPxConnBroadcastSender,
    ) -> Self {
        Self {
            signer,
            node,
            framed: Framed::new(stream, codec),
            storage,
            capabilities: vec![],
            negotiated_eth_version: 0,
            negotiated_snap_version: 0,
            next_periodic_task_check: Instant::now() + PERIODIC_TASKS_CHECK_INTERVAL,
            connection_broadcast_send: connection_broadcast,
        }
    }

    /// Handshake already performed, now it starts a peer connection.
    /// It runs in it's own task and blocks until the connection is dropped
    pub async fn start(&mut self, table: Arc<Mutex<crate::kademlia::KademliaTable>>) {
        log_peer_debug(&self.node, "Starting RLPx connection");
        if let Err(e) = self.exchange_hello_messages().await {
            self.connection_failed("Hello messages exchange failed", e, table)
                .await;
        } else {
            // Handshake OK: handle connection
            // Create channels to communicate directly to the peer
            let (peer_channels, sender, receiver) = PeerChannels::create();
            let capabilities = self
                .capabilities
                .iter()
                .map(|(cap, _)| cap.clone())
                .collect();

            // NOTE: if the peer came from the discovery server it will already be inserted in the table
            // but that might not always be the case, so we try to add it to the table
            // Note: we don't ping the node we let the validation service do its job
            {
                let mut table_lock = table.lock().await;
                table_lock.insert_node_forced(self.node);
                table_lock.init_backend_communication(
                    self.node.node_id,
                    peer_channels,
                    capabilities,
                );
            }
            if let Err(e) = self.connection_loop(sender, receiver).await {
                self.connection_failed("Error during RLPx connection", e, table)
                    .await;
            }
        }
    }

    async fn connection_failed(
        &mut self,
        error_text: &str,
        error: RLPxError,
        table: Arc<Mutex<crate::kademlia::KademliaTable>>,
    ) {
        self.send(Message::Disconnect(DisconnectMessage {
            reason: self.match_disconnect_reason(&error),
        }))
        .await
        .unwrap_or_else(|_| {
            log_peer_error(
                &self.node,
                &format!(
                    "Could not send Disconnect message: ({:?}).",
                    self.match_disconnect_reason(&error)
                ),
            )
        });

        // Discard peer from kademlia table
        match error {
            // already connected, don't discard it
            RLPxError::DisconnectRequested(s) if s == "Already connected" => {
                log_peer_debug(&self.node, "Peer already connected don't replace it");
            }
            RLPxError::NoMatchingCapabilities() => {
                log_peer_debug(
                    &self.node,
                    "Can't start connection as no matching capability during hello message exchange.",
                );
            }
            _ => {
                let remote_node_id = self.node.node_id;
                log_peer_error(
                    &self.node,
                    &format!("{error_text}: ({error}), discarding peer {remote_node_id}"),
                );
                table.lock().await.replace_peer(remote_node_id);
            }
        }

        let _ = self.framed.close().await;
    }

    fn match_disconnect_reason(&self, error: &RLPxError) -> Option<u8> {
        match error {
            RLPxError::RLPDecodeError(_) => Some(2_u8),
            // TODO build a proper matching between error types and disconnection reasons
            _ => None,
        }
    }

    async fn exchange_hello_messages(&mut self) -> Result<(), RLPxError> {
        let hello_msg = Message::Hello(p2p::HelloMessage::new(
            SUPPORTED_CAPABILITIES.to_vec(),
            PublicKey::from(self.signer.verifying_key()),
        ));

        self.send(hello_msg).await?;

        // Receive Hello message
        let msg = match self.receive().await {
            Some(msg) => msg?,
            None => return Err(RLPxError::Disconnected()),
        };

        match msg {
            Message::Hello(hello_message) => {
                let mut negotiated_eth_cap = (Capability::Eth, 0);
                let mut negotiated_snap_cap = (Capability::Snap, 0);

                log_peer_debug(
                    &self.node,
                    &format!(
                        "Hello message capabilities {:?}",
                        hello_message.capabilities
                    ),
                );

                // Check if we have any capability in common and store the highest version
                for cap in &hello_message.capabilities {
                    match *cap {
                        CAP_ETH_68 if CAP_ETH_68.1 > negotiated_eth_cap.1 => {
                            negotiated_eth_cap = CAP_ETH_68
                        }
                        CAP_SNAP_1 if CAP_SNAP_1.1 > negotiated_snap_cap.1 => {
                            negotiated_snap_cap = CAP_SNAP_1
                        }
                        _ => {}
                    }
                }

                self.capabilities = hello_message.capabilities;

                if negotiated_eth_cap.1 == 0 {
                    return Err(RLPxError::NoMatchingCapabilities());
                }
                self.negotiated_eth_version = negotiated_eth_cap.1;

                if negotiated_snap_cap.1 != 0 {
                    self.negotiated_snap_version = negotiated_snap_cap.1;
                }

                Ok(())
            }
            Message::Disconnect(disconnect) => Err(RLPxError::DisconnectRequested(
                disconnect.reason().to_string(),
            )),
            _ => {
                // Fail if it is not a hello message
                Err(RLPxError::BadRequest("Expected Hello message".to_string()))
            }
        }
    }

    async fn connection_loop(
        &mut self,
        sender: mpsc::Sender<Message>,
        mut receiver: mpsc::Receiver<Message>,
    ) -> Result<(), RLPxError> {
        self.init_peer_conn().await?;
        log_peer_debug(&self.node, "Started peer main loop");
        update_peer_conn_status(self.node, true).await;

        // Subscribe this connection to the broadcasting channel.
        let mut broadcaster_receive = {
            if self
                .capabilities
                .contains(&(Capability::Eth, self.negotiated_eth_version))
            {
                Some(self.connection_broadcast_send.subscribe())
            } else {
                None
            }
        };

        // Start listening for messages,
        loop {
            tokio::select! {
                // Expect a message from the remote peer
                Some(message) = self.receive() => {
                    match message {
                        Ok(message) => {
                            log_peer_debug(&self.node, &format!("Received message {}", message));
                            self.handle_message(message, sender.clone()).await?;
                        },
                        Err(e) => {
                            log_peer_debug(&self.node, &format!("Received RLPX Error in msg {}", e));
                            return Err(e);
                        }
                    }
                }
                // Expect a message from the backend
                Some(message) = receiver.recv() => {
                    log_peer_debug(&self.node, &format!("Sending message {}", message));
                    self.send(message).await?;
                }
                // This is not ideal, but using the receiver without
                // this function call, causes the loop to take ownwership
                // of the variable and the compiler will complain about it,
                // with this function, we avoid that.
                // If the broadcaster is Some (i.e. we're connected to a peer that supports an eth protocol),
                // we'll receive broadcasted messages from another connections through a channel, otherwise
                // the function below will yield immediately but the select will not match and
                // ignore the returned value.
                Some(broadcasted_msg) = Self::maybe_wait_for_broadcaster(&mut broadcaster_receive) => {
                    self.handle_broadcast(broadcasted_msg?).await?
                }
                // Allow an interruption to check periodic tasks
                _ = sleep(PERIODIC_TASKS_CHECK_INTERVAL) => () // noop
            }
            self.check_periodic_tasks().await?;
        }
    }

    async fn maybe_wait_for_broadcaster(
        receiver: &mut Option<broadcast::Receiver<(task::Id, Arc<Message>)>>,
    ) -> Option<Result<(task::Id, Arc<Message>), RecvError>> {
        match receiver {
            None => None,
            Some(rec) => Some(rec.recv().await),
        }
    }

    async fn check_periodic_tasks(&mut self) -> Result<(), RLPxError> {
        if Instant::now() >= self.next_periodic_task_check {
            self.send(Message::Ping(PingMessage {})).await?;
            log_peer_debug(&self.node, "Ping sent");
            self.next_periodic_task_check = Instant::now() + PERIODIC_TASKS_CHECK_INTERVAL;
        };
        Ok(())
    }

    async fn handle_message(
        &mut self,
        message: Message,
        sender: mpsc::Sender<Message>,
    ) -> Result<(), RLPxError> {
        let peer_supports_eth = self.negotiated_eth_version != 0;
        let is_synced = self.storage.is_synced()?;
        match message {
            Message::Disconnect(msg_data) => {
                log_peer_debug(
                    &self.node,
                    &format!("Received Disconnect: {}", msg_data.reason()),
                );
                // TODO handle the disconnection request
                return Err(RLPxError::DisconnectRequested(
                    msg_data.reason().to_string(),
                ));
            }
            Message::Ping(_) => {
                log_peer_debug(&self.node, "Sending pong message");
                self.send(Message::Pong(PongMessage {})).await?;
            }
            Message::Pong(_) => {
                // We ignore received Pong messages
            }
            Message::Status(msg_data) if !peer_supports_eth => backend::validate_status(
                msg_data,
                &self.storage,
                self.negotiated_eth_version as u32,
            )?,
            Message::GetAccountRange(req) => {
                let response = process_account_range_request(req, self.storage.clone())?;
                self.send(Message::AccountRange(response)).await?
            }
            // TODO(#1129) Add the transaction to the mempool once received.
            Message::Transactions(txs) if peer_supports_eth => {
                if is_synced {
                    let mut valid_txs = vec![];
                    for tx in &txs.transactions {
                        if let Err(e) = mempool::add_transaction(tx.clone(), &self.storage) {
                            log_peer_warn(&self.node, &format!("Error adding transaction: {}", e));
                            continue;
                        }
                        valid_txs.push(tx.clone());
                    }
                    self.broadcast_message(Message::Transactions(Transactions::new(valid_txs)))?;
                }
            }
            Message::GetBlockHeaders(msg_data) if peer_supports_eth => {
                let response = BlockHeaders {
                    id: msg_data.id,
                    block_headers: msg_data.fetch_headers(&self.storage),
                };
                self.send(Message::BlockHeaders(response)).await?;
            }
            Message::GetBlockBodies(msg_data) if peer_supports_eth => {
                let response = BlockBodies {
                    id: msg_data.id,
                    block_bodies: msg_data.fetch_blocks(&self.storage),
                };
                self.send(Message::BlockBodies(response)).await?;
            }
            Message::GetReceipts(GetReceipts { id, block_hashes }) if peer_supports_eth => {
                let receipts: Result<_, _> = block_hashes
                    .iter()
                    .map(|hash| self.storage.get_receipts_for_block(hash))
                    .collect();
                let response = Receipts {
                    id,
                    receipts: receipts?,
                };
                self.send(Message::Receipts(response)).await?;
            }
            Message::NewPooledTransactionHashes(new_pooled_transaction_hashes)
                if peer_supports_eth =>
            {
                //TODO(#1415): evaluate keeping track of requests to avoid sending the same twice.
                let hashes =
                    new_pooled_transaction_hashes.get_transactions_to_request(&self.storage)?;

                //TODO(#1416): Evaluate keeping track of the request-id.
                let request = GetPooledTransactions::new(random(), hashes);
                self.send(Message::GetPooledTransactions(request)).await?;
            }
            Message::GetPooledTransactions(msg) => {
                let response = msg.handle(&self.storage)?;
                self.send(Message::PooledTransactions(response)).await?;
            }
            Message::PooledTransactions(msg) if peer_supports_eth => {
                if is_synced {
                    msg.handle(&self.node, &self.storage)?;
                }
            }
            Message::GetStorageRanges(req) => {
                let response = process_storage_ranges_request(req, self.storage.clone())?;
                self.send(Message::StorageRanges(response)).await?
            }
            Message::GetByteCodes(req) => {
                let response = process_byte_codes_request(req, self.storage.clone())?;
                self.send(Message::ByteCodes(response)).await?
            }
            Message::GetTrieNodes(req) => {
                let response = process_trie_nodes_request(req, self.storage.clone())?;
                self.send(Message::TrieNodes(response)).await?
            }
            // Send response messages to the backend
            message @ Message::AccountRange(_)
            | message @ Message::StorageRanges(_)
            | message @ Message::ByteCodes(_)
            | message @ Message::TrieNodes(_)
            | message @ Message::BlockBodies(_)
            | message @ Message::BlockHeaders(_)
            | message @ Message::Receipts(_) => sender.send(message).await?,
            // TODO: Add new message types and handlers as they are implemented
            message => return Err(RLPxError::MessageNotHandled(format!("{message}"))),
        };
        Ok(())
    }

    async fn handle_broadcast(
        &mut self,
        (id, broadcasted_msg): (task::Id, Arc<Message>),
    ) -> Result<(), RLPxError> {
        if id != tokio::task::id() {
            match broadcasted_msg.as_ref() {
                Message::Transactions(ref txs) => {
                    // TODO(#1131): Avoid cloning this vector.
                    let cloned = txs.transactions.clone();
                    let new_msg = Message::Transactions(Transactions {
                        transactions: cloned,
                    });
                    self.send(new_msg).await?;
                }
                msg => {
                    let error_message = format!("Non-supported message broadcasted: {msg}");
                    log_peer_error(&self.node, &error_message);
                    return Err(RLPxError::BroadcastError(error_message));
                }
            }
        }
        Ok(())
    }

    async fn init_peer_conn(&mut self) -> Result<(), RLPxError> {
        // Sending eth Status if peer supports it
        if self
            .capabilities
            .contains(&(Capability::Eth, self.negotiated_eth_version))
        {
            let status = backend::get_status(&self.storage, self.negotiated_eth_version as u32)?;
            log_peer_debug(&self.node, "Sending status");
            self.send(Message::Status(status)).await?;
            // The next immediate message in the ETH protocol is the
            // status, reference here:
            // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
            let msg = match self.receive().await {
                Some(msg) => msg?,
                None => return Err(RLPxError::Disconnected()),
            };
            match msg {
                Message::Status(msg_data) => {
                    log_peer_debug(&self.node, "Received Status");
                    backend::validate_status(
                        msg_data,
                        &self.storage,
                        self.negotiated_eth_version as u32,
                    )?
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

    async fn send(&mut self, message: Message) -> Result<(), RLPxError> {
        self.framed.send(message).await
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
    async fn receive(&mut self) -> Option<Result<Message, RLPxError>> {
        self.framed.next().await
    }

    fn broadcast_message(&self, msg: Message) -> Result<(), RLPxError> {
        match msg {
            txs_msg @ Message::Transactions(_) => {
                let txs = Arc::new(txs_msg);
                let task_id = tokio::task::id();
                let Ok(_) = self.connection_broadcast_send.send((task_id, txs)) else {
                    let error_message = "Could not broadcast received transactions";
                    log_peer_error(&self.node, error_message);
                    return Err(RLPxError::BroadcastError(error_message.to_owned()));
                };
                Ok(())
            }
            msg => {
                let error_message = format!("Broadcasting for msg: {msg} is not supported");
                log_peer_error(&self.node, &error_message);
                Err(RLPxError::BroadcastError(error_message))
            }
        }
    }
}
