#[cfg(feature = "l2")]
use crate::rlpx::based::get_hash_batch_sealed;
use crate::rlpx::utils::get_pub_key;
use crate::{
    kademlia::PeerChannels,
    rlpx::{
        based::{BatchSealedMessage, NewBlockMessage},
        error::RLPxError,
        eth::{
            backend,
            blocks::{BlockBodies, BlockHeaders},
            receipts::{GetReceipts, Receipts},
            transactions::{GetPooledTransactions, Transactions},
        },
        frame::RLPxCodec,
        message::Message,
        p2p::{
            self, Capability, DisconnectMessage, PingMessage, PongMessage,
            SUPPORTED_BASED_CAPABILITIES, SUPPORTED_ETH_CAPABILITIES, SUPPORTED_P2P_CAPABILITIES,
            SUPPORTED_SNAP_CAPABILITIES,
        },
        utils::{log_peer_debug, log_peer_error},
    },
    snap::{
        process_account_range_request, process_byte_codes_request, process_storage_ranges_request,
        process_trie_nodes_request,
    },
    types::Node,
};
use ethrex_blockchain::{fork_choice::apply_fork_choice, Blockchain};
use ethrex_common::{
    types::{Block, MempoolTransaction, Transaction},
    Address, H256, H512,
};
use ethrex_storage::Store;
#[cfg(feature = "l2")]
use ethrex_storage_rollup::StoreRollup;
use futures::SinkExt;
use k256::{ecdsa::SigningKey, PublicKey, SecretKey};
use lazy_static::lazy_static;
use rand::random;
use secp256k1::Message as SignedMessage;
use secp256k1::SecretKey as SigningKeySecp256k1;
use std::collections::BTreeMap;
use std::{collections::HashSet, sync::Arc};
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
use tracing::{debug, info, warn};

use super::{
    eth::transactions::NewPooledTransactionHashes, p2p::DisconnectReason, utils::log_peer_warn,
};

const PERIODIC_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const PERIODIC_TX_BROADCAST_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
const PERIODIC_BLOCK_BROADCAST_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(500);
const PERIODIC_TASKS_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
pub const MAX_PEERS_TCP_CONNECTIONS: usize = 100;

lazy_static! {
    pub static ref ADDRESS_LEAD_SEQUENCER: Address =
        Address::from_slice(&hex::decode("3d1e15a1a55578f7c920884a9943b3b35d0d885b").unwrap());
}

pub(crate) type Aes256Ctr64BE = ctr::Ctr64BE<aes::Aes256>;

pub(crate) type RLPxConnBroadcastSender = broadcast::Sender<(tokio::task::Id, Arc<Message>)>;

pub(crate) struct RemoteState {
    pub(crate) public_key: H512,
    pub(crate) nonce: H256,
    pub(crate) ephemeral_key: PublicKey,
    pub(crate) init_message: Vec<u8>,
}

pub(crate) struct LocalState {
    pub(crate) nonce: H256,
    pub(crate) ephemeral_key: SecretKey,
    pub(crate) init_message: Vec<u8>,
}

/// Fully working RLPx connection.
pub(crate) struct RLPxConnection<S> {
    signer: SigningKey,
    node: Node,
    framed: Framed<S, RLPxCodec>,
    storage: Store,
    blockchain: Arc<Blockchain>,
    capabilities: Vec<Capability>,
    negotiated_eth_capability: Option<Capability>,
    negotiated_snap_capability: Option<Capability>,
    next_periodic_ping: Instant,
    next_tx_broadcast: Instant,
    next_block_broadcast: Instant,
    broadcasted_txs: HashSet<H256>,
    latest_block_sent: u64,
    latest_block_added: u64,
    blocks_on_queue: BTreeMap<u64, Block>,
    latest_batch_sent: u64,
    client_version: String,
    /// Send end of the channel used to broadcast messages
    /// to other connected peers, is ok to have it here,
    /// since internally it's an Arc.
    /// The ID is to ignore the message sent from the same task.
    /// This is used both to send messages and to received broadcasted
    /// messages from other connections (sent from other peers).
    /// The receive end is instantiated after the handshake is completed
    /// under `handle_peer`.
    connection_broadcast_send: RLPxConnBroadcastSender,
    #[cfg(feature = "l2")]
    store_rollup: StoreRollup,
    based: bool,
    secret_key: Option<SigningKeySecp256k1>,
}

impl<S: AsyncWrite + AsyncRead + std::marker::Unpin> RLPxConnection<S> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        signer: SigningKey,
        node: Node,
        stream: S,
        codec: RLPxCodec,
        storage: Store,
        blockchain: Arc<Blockchain>,
        client_version: String,
        connection_broadcast: RLPxConnBroadcastSender,
        #[cfg(feature = "l2")] store_rollup: StoreRollup,
        based: bool,
        secret_key: Option<SigningKeySecp256k1>,
    ) -> Self {
        Self {
            signer,
            node,
            framed: Framed::new(stream, codec),
            storage,
            blockchain,
            capabilities: vec![],
            negotiated_eth_capability: None,
            negotiated_snap_capability: None,
            next_periodic_ping: Instant::now() + PERIODIC_TASKS_CHECK_INTERVAL,
            next_tx_broadcast: Instant::now() + PERIODIC_TX_BROADCAST_INTERVAL,
            next_block_broadcast: Instant::now() + PERIODIC_BLOCK_BROADCAST_INTERVAL,
            broadcasted_txs: HashSet::new(),
            latest_block_sent: 0,
            latest_block_added: 0,
            blocks_on_queue: BTreeMap::new(),
            latest_batch_sent: 0,
            client_version,
            connection_broadcast_send: connection_broadcast,
            #[cfg(feature = "l2")]
            store_rollup,
            based,
            secret_key,
        }
    }

    async fn post_handshake_checks(
        &self,
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

    /// Handshake already performed, now it starts a peer connection.
    /// It runs in it's own task and blocks until the connection is dropped
    pub async fn start(
        &mut self,
        table: Arc<Mutex<crate::kademlia::KademliaTable>>,
        inbound: bool,
    ) {
        log_peer_debug(&self.node, "Starting RLPx connection");

        if let Err(reason) = self.post_handshake_checks(table.clone()).await {
            self.connection_failed(
                "Post handshake validations failed",
                RLPxError::DisconnectSent(reason),
                table,
            )
            .await;
            return;
        }

        if let Err(e) = self.exchange_hello_messages().await {
            self.connection_failed("Hello messages exchange failed", e, table)
                .await;
        } else {
            // Handshake OK: handle connection
            // Create channels to communicate directly to the peer
            let (peer_channels, sender, receiver) = PeerChannels::create();

            // NOTE: if the peer came from the discovery server it will already be inserted in the table
            // but that might not always be the case, so we try to add it to the table
            // Note: we don't ping the node we let the validation service do its job
            {
                let mut table_lock = table.lock().await;
                table_lock.insert_node_forced(self.node.clone());
                table_lock.init_backend_communication(
                    self.node.node_id(),
                    peer_channels,
                    self.capabilities.clone(),
                    inbound,
                );
            }
            if let Err(e) = self.connection_loop(sender, receiver).await {
                self.connection_failed("Error during RLPx connection", e, table)
                    .await;
            }
        }
    }

    async fn send_disconnect_message(&mut self, reason: Option<DisconnectReason>) {
        self.send(Message::Disconnect(DisconnectMessage { reason }))
            .await
            .unwrap_or_else(|_| {
                log_peer_debug(
                    &self.node,
                    &format!("Could not send Disconnect message: ({:?}).", reason),
                );
            });
    }

    async fn connection_failed(
        &mut self,
        error_text: &str,
        error: RLPxError,
        table: Arc<Mutex<crate::kademlia::KademliaTable>>,
    ) {
        log_peer_debug(&self.node, &format!("{error_text}: ({error})"));

        // Send disconnect message only if error is different than RLPxError::DisconnectRequested
        // because if it is a DisconnectRequested error it means that the peer requested the disconnection, not us.
        if !matches!(error, RLPxError::DisconnectReceived(_)) {
            self.send_disconnect_message(self.match_disconnect_reason(&error))
                .await;
        }

        // Discard peer from kademlia table in some cases
        match error {
            // already connected, don't discard it
            RLPxError::DisconnectReceived(DisconnectReason::AlreadyConnected)
            | RLPxError::DisconnectSent(DisconnectReason::AlreadyConnected) => {
                log_peer_debug(&self.node, "Peer already connected, don't replace it");
            }
            _ => {
                let remote_public_key = self.node.public_key;
                log_peer_debug(
                    &self.node,
                    &format!("{error_text}: ({error}), discarding peer {remote_public_key}"),
                );
                table.lock().await.replace_peer(self.node.node_id());
            }
        }

        let _ = self.framed.close().await;
    }

    fn match_disconnect_reason(&self, error: &RLPxError) -> Option<DisconnectReason> {
        match error {
            RLPxError::DisconnectSent(reason) => Some(*reason),
            RLPxError::DisconnectReceived(reason) => Some(*reason),
            RLPxError::RLPDecodeError(_) => Some(DisconnectReason::NetworkError),
            // TODO build a proper matching between error types and disconnection reasons
            _ => None,
        }
    }

    async fn exchange_hello_messages(&mut self) -> Result<(), RLPxError> {
        let mut supported_capabilities: Vec<Capability> = [
            &SUPPORTED_ETH_CAPABILITIES[..],
            &SUPPORTED_SNAP_CAPABILITIES[..],
            &SUPPORTED_P2P_CAPABILITIES[..],
        ]
        .concat();
        if self.based {
            supported_capabilities.push(SUPPORTED_BASED_CAPABILITIES);
        }
        let hello_msg = Message::Hello(p2p::HelloMessage::new(
            supported_capabilities,
            PublicKey::from(self.signer.verifying_key()),
            self.client_version.clone(),
        ));

        self.send(hello_msg).await?;

        // Receive Hello message
        let msg = match self.receive().await {
            Some(msg) => msg?,
            None => return Err(RLPxError::Disconnected()),
        };

        match msg {
            Message::Hello(hello_message) => {
                let mut negotiated_eth_version = 0;
                let mut negotiated_snap_version = 0;

                log_peer_debug(
                    &self.node,
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

                self.capabilities = hello_message.capabilities;

                if negotiated_eth_version == 0 {
                    return Err(RLPxError::NoMatchingCapabilities());
                }
                debug!("Negotatied eth version: eth/{}", negotiated_eth_version);
                self.negotiated_eth_capability = Some(Capability::eth(negotiated_eth_version));

                if negotiated_snap_version != 0 {
                    debug!("Negotatied snap version: snap/{}", negotiated_snap_version);
                    self.negotiated_snap_capability =
                        Some(Capability::snap(negotiated_snap_version));
                }

                self.node.version = Some(hello_message.client_id);

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

    async fn connection_loop(
        &mut self,
        sender: mpsc::Sender<Message>,
        mut receiver: mpsc::Receiver<Message>,
    ) -> Result<(), RLPxError> {
        self.init_peer_conn().await?;
        log_peer_debug(&self.node, "Started peer main loop");

        // Subscribe this connection to the broadcasting channel.
        let mut broadcaster_receive = if self.negotiated_eth_capability.is_some() {
            Some(self.connection_broadcast_send.subscribe())
        } else {
            None
        };

        // Send transactions transaction hashes from mempool at connection start
        self.send_new_pooled_tx_hashes().await?;
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
                // this function call, causes the loop to take ownership
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
                _ = sleep(PERIODIC_TASKS_CHECK_INTERVAL) => (), // noop
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
        if Instant::now() >= self.next_periodic_ping {
            self.send(Message::Ping(PingMessage {})).await?;
            log_peer_debug(&self.node, "Ping sent");
            self.next_periodic_ping = Instant::now() + PERIODIC_PING_INTERVAL;
        };
        if Instant::now() >= self.next_tx_broadcast {
            self.send_new_pooled_tx_hashes().await?;
            self.next_tx_broadcast = Instant::now() + PERIODIC_TX_BROADCAST_INTERVAL;
        }
        if Instant::now() >= self.next_block_broadcast {
            self.send_new_block().await?;
            self.next_block_broadcast = Instant::now() + PERIODIC_BLOCK_BROADCAST_INTERVAL;
        }
        if Instant::now() >= self.next_periodic_ping - PERIODIC_PING_INTERVAL {
            self.send_sealed_batch().await?;
        }
        Ok(())
    }

    async fn send_new_pooled_tx_hashes(&mut self) -> Result<(), RLPxError> {
        if SUPPORTED_ETH_CAPABILITIES
            .iter()
            .any(|cap| self.capabilities.contains(cap))
        {
            let filter = |tx: &Transaction| -> bool {
                !self.broadcasted_txs.contains(&tx.compute_hash())
                    && !matches!(&tx, Transaction::PrivilegedL2Transaction(_))
            };
            let txs: Vec<MempoolTransaction> = self
                .blockchain
                .mempool
                .filter_transactions_with_filter_fn(&filter)?
                .into_values()
                .flatten()
                .collect();
            if !txs.is_empty() {
                let tx_count = txs.len();
                for tx in txs {
                    self.send(Message::NewPooledTransactionHashes(
                        NewPooledTransactionHashes::new(vec![(*tx).clone()], &self.blockchain)?,
                    ))
                    .await?;
                    // Possible improvement: the mempool already knows the hash but the filter function does not return it
                    self.broadcasted_txs.insert((*tx).compute_hash());
                }
                log_peer_debug(
                    &self.node,
                    &format!("Sent {} transactions to peer", tx_count),
                );
            }
        }
        Ok(())
    }

    async fn send_new_block(&mut self) -> Result<(), RLPxError> {
        if !self.capabilities.contains(&SUPPORTED_BASED_CAPABILITIES) {
            return Ok(());
        }
        let latest_block_number = self.storage.get_latest_block_number().await?;
        for i in self.latest_block_sent + 1..=latest_block_number {
            debug!(
                "Broadcasting new block, current: {}, last broadcasted: {}",
                i, self.latest_block_sent
            );
            let new_block_body = self.storage.get_block_body(i).await?.unwrap();
            let new_block_header = self.storage.get_block_header(i)?.unwrap();
            let new_block = Block {
                header: new_block_header,
                body: new_block_body,
            };
            #[cfg(feature = "l2")]
            {
                let (signature, recovery_id) = if let Some(recovered_sig) = self
                    .store_rollup
                    .get_signature_by_block(new_block.hash())
                    .await?
                {
                    let mut signature = [0u8; 64];
                    let mut recovery_id = [0u8; 4];
                    signature.copy_from_slice(&recovered_sig[..64]);
                    recovery_id.copy_from_slice(&recovered_sig[64..68]);
                    (signature, recovery_id)
                } else {
                    let Some(secret_key) = self.secret_key else {
                        return Err(RLPxError::InternalError(
                            "Secret key is not set for based connection".to_string(),
                        ));
                    };
                    let (recovery_id, signature) = secp256k1::SECP256K1
                        .sign_ecdsa_recoverable(
                            &SignedMessage::from_digest(new_block.hash().to_fixed_bytes()),
                            &secret_key,
                        )
                        .serialize_compact();
                    let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();
                    (signature, recovery_id)
                };
                self.send(Message::NewBlock(NewBlockMessage {
                    block: new_block,
                    signature,
                    recovery_id,
                }))
                .await?;
            }
        }
        self.latest_block_sent = latest_block_number;

        Ok(())
    }

    async fn send_sealed_batch(&mut self) -> Result<(), RLPxError> {
        #[cfg(feature = "l2")]
        {
            let next_batch_to_send = self.latest_batch_sent + 1;
            if !self
                .store_rollup
                .contains_batch(&next_batch_to_send)
                .await?
            {
                return Ok(());
            }
            let block_numbers = self
                .store_rollup
                .get_block_numbers_by_batch(self.latest_batch_sent + 1)
                .await?
                .ok_or(RLPxError::InternalError(
                    "No batch found after containing check".to_string(),
                ))?;
            let withdrawal_hashes = self
                .store_rollup
                .get_withdrawal_hashes_by_batch(next_batch_to_send)
                .await?
                .ok_or(RLPxError::InternalError(
                    "No withdrawal hashes found for the batch".to_string(),
                ))?;

            let (signature, recovery_id) = if let Some(recovered_sig) = self
                .store_rollup
                .get_signature_by_batch(next_batch_to_send)
                .await?
            {
                let mut signature = [0u8; 64];
                let mut recovery_id = [0u8; 4];
                signature.copy_from_slice(&recovered_sig[..64]);
                recovery_id.copy_from_slice(&recovered_sig[64..68]);
                (signature, recovery_id)
            } else {
                let Some(secret_key) = self.secret_key else {
                    return Err(RLPxError::InternalError(
                        "Secret key is not set for based connection".to_string(),
                    ));
                };
                let (recovery_id, signature) = secp256k1::SECP256K1
                    .sign_ecdsa_recoverable(
                        &SignedMessage::from_digest(get_hash_batch_sealed(
                            next_batch_to_send,
                            &block_numbers,
                            &withdrawal_hashes,
                        )),
                        &secret_key,
                    )
                    .serialize_compact();
                let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();
                (signature, recovery_id)
            };

            let msg = Message::BatchSealed(BatchSealedMessage {
                batch_number: next_batch_to_send,
                block_numbers,
                withdrawal_hashes,
                signature,
                recovery_id,
            });
            self.send(msg).await?;
            self.latest_batch_sent += 1;
            Ok(())
        }
        #[cfg(not(feature = "l2"))]
        {
            Ok(())
        }
    }

    async fn handle_message(
        &mut self,
        message: Message,
        sender: mpsc::Sender<Message>,
    ) -> Result<(), RLPxError> {
        let peer_supports_eth = self.negotiated_eth_capability.is_some();
        let peer_supports_based = self.capabilities.contains(&SUPPORTED_BASED_CAPABILITIES);
        match message {
            Message::Disconnect(msg_data) => {
                log_peer_debug(
                    &self.node,
                    &format!("Received Disconnect: {}", msg_data.reason()),
                );
                // TODO handle the disconnection request
                return Err(RLPxError::DisconnectReceived(msg_data.reason()));
            }
            Message::Ping(_) => {
                log_peer_debug(&self.node, "Sending pong message");
                self.send(Message::Pong(PongMessage {})).await?;
            }
            Message::Pong(_) => {
                // We ignore received Pong messages
            }
            Message::Status(msg_data) => {
                if let Some(eth) = &self.negotiated_eth_capability {
                    backend::validate_status(msg_data, &self.storage, eth.version).await?
                };
            }
            Message::GetAccountRange(req) => {
                let response = process_account_range_request(req, self.storage.clone())?;
                self.send(Message::AccountRange(response)).await?
            }
            // TODO(#1129) Add the transaction to the mempool once received.
            Message::Transactions(txs) if peer_supports_eth => {
                if self.blockchain.is_synced() {
                    let mut valid_txs = vec![];
                    for tx in &txs.transactions {
                        if let Err(e) = self.blockchain.add_transaction_to_pool(tx.clone()).await {
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
                    block_headers: msg_data.fetch_headers(&self.storage).await,
                };
                self.send(Message::BlockHeaders(response)).await?;
            }
            Message::GetBlockBodies(msg_data) if peer_supports_eth => {
                let response = BlockBodies {
                    id: msg_data.id,
                    block_bodies: msg_data.fetch_blocks(&self.storage).await,
                };
                self.send(Message::BlockBodies(response)).await?;
            }
            Message::GetReceipts(GetReceipts { id, block_hashes }) if peer_supports_eth => {
                let mut receipts = Vec::new();
                for hash in block_hashes.iter() {
                    receipts.push(self.storage.get_receipts_for_block(hash)?);
                }
                let response = Receipts { id, receipts };
                self.send(Message::Receipts(response)).await?;
            }
            Message::NewPooledTransactionHashes(new_pooled_transaction_hashes)
                if peer_supports_eth =>
            {
                //TODO(#1415): evaluate keeping track of requests to avoid sending the same twice.
                let hashes =
                    new_pooled_transaction_hashes.get_transactions_to_request(&self.blockchain)?;

                //TODO(#1416): Evaluate keeping track of the request-id.
                let request = GetPooledTransactions::new(random(), hashes);
                self.send(Message::GetPooledTransactions(request)).await?;
            }
            Message::GetPooledTransactions(msg) => {
                let response = msg.handle(&self.blockchain)?;
                self.send(Message::PooledTransactions(response)).await?;
            }
            Message::PooledTransactions(msg) if peer_supports_eth => {
                if self.blockchain.is_synced() {
                    msg.handle(&self.node, &self.blockchain).await?;
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
            Message::NewBlock(req) if peer_supports_based => {
                if self.validate_new_block(&req).await? {
                    self.process_new_block(&req).await?;
                    // for now we broadcast valid messages, but this should be reviewed
                    // self.broadcast_message(Message::NewBlock(req))?;
                }
            }
            Message::BatchSealed(req) => {
                #[cfg(feature = "l2")]
                {
                    if self.validate_batch_sealed(&req).await? {
                        self.process_batch_sealed(req).await?;
                    }
                }
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
                Message::NewBlock(ref block_msg) => {
                    let new_msg = Message::NewBlock(block_msg.clone());
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

    async fn validate_new_block(&mut self, msg: &NewBlockMessage) -> Result<bool, RLPxError> {
        if self.latest_block_added >= msg.block.header.number
            || self.blocks_on_queue.contains_key(&msg.block.header.number)
        {
            debug!(
                "Block {} received by peer already stored, ignoring it",
                msg.block.header.number
            );
            return Ok(false);
        }

        let block_hash = msg.block.hash();

        let recovered_lead_sequencer = get_pub_key(
            msg.recovery_id,
            &msg.signature,
            *block_hash.as_fixed_bytes(),
        );

        if recovered_lead_sequencer != *ADDRESS_LEAD_SEQUENCER {
            debug!(
                "Received block from wrong lead sequencer: {}. Expected: {}",
                recovered_lead_sequencer, *ADDRESS_LEAD_SEQUENCER
            );
            return Ok(false);
        }
        #[cfg(feature = "l2")]
        {
            let mut signature = [0u8; 68];
            signature[..64].copy_from_slice(&msg.signature[..]);
            signature[64..].copy_from_slice(&msg.recovery_id[..]);
            self.store_rollup
                .store_signature_by_block(block_hash, signature)
                .await?;
        }
        Ok(true)
    }

    async fn process_new_block(&mut self, msg: &NewBlockMessage) -> Result<(), RLPxError> {
        self.blocks_on_queue
            .entry(msg.block.header.number)
            .or_insert_with(|| msg.block.clone());

        let mut next_block_to_add = self.latest_block_added + 1;
        while let Some(block) = self.blocks_on_queue.remove(&next_block_to_add) {
            // This check is necessary if a connection to another peer already applied the block but this connection
            // did not register that update.
            if let Ok(Some(_)) = self.storage.get_block_body(next_block_to_add).await {
                self.latest_block_added = next_block_to_add;
                next_block_to_add += 1;
                continue;
            }
            self.blockchain.add_block(&block).await.inspect_err(|e| {
                log_peer_error(
                    &self.node,
                    &format!(
                        "Error adding new block {} with hash {:?}, error: {e}",
                        block.header.number,
                        block.hash()
                    ),
                );
            })?;
            let block_hash = block.hash();

            apply_fork_choice(&self.storage, block_hash, block_hash, block_hash)
                .await
                .map_err(|e| {
                    RLPxError::BadRequest(format!(
                        "Error adding new block {} with hash {:?}, error: {e}",
                        block.header.number,
                        block.hash()
                    ))
                })?;

            self.latest_block_added = next_block_to_add;
            next_block_to_add += 1;
        }
        Ok(())
    }

    async fn validate_batch_sealed(&mut self, msg: &BatchSealedMessage) -> Result<bool, RLPxError> {
        #[cfg(feature = "l2")]
        {
            if self.store_rollup.contains_batch(&msg.batch_number).await? {
                debug!("Batch {} already sealed, ignoring it", msg.batch_number);
                return Ok(false);
            }
            if msg.block_numbers.is_empty() {
                return Ok(false);
            }

            let hash =
                get_hash_batch_sealed(msg.batch_number, &msg.block_numbers, &msg.withdrawal_hashes);

            let recovered_lead_sequencer = get_pub_key(msg.recovery_id, &msg.signature, hash);

            if recovered_lead_sequencer != *ADDRESS_LEAD_SEQUENCER {
                warn!(
                    "Received batch from wrong lead sequencer: {}. Expected: {}",
                    recovered_lead_sequencer, *ADDRESS_LEAD_SEQUENCER
                );
                return Ok(false);
            }
            let mut signature = [0u8; 68];
            signature[..64].copy_from_slice(&msg.signature[..]);
            signature[64..].copy_from_slice(&msg.recovery_id[..]);
            self.store_rollup
                .store_signature_by_batch(msg.batch_number, signature)
                .await?;
            Ok(true)
        }
        #[cfg(not(feature = "l2"))]
        {
            Err(RLPxError::InternalError(
                "This function cannot won't be called without the l2 feature flag".to_string(),
            ))
        }
    }

    async fn process_batch_sealed(&mut self, msg: BatchSealedMessage) -> Result<(), RLPxError> {
        #[cfg(feature = "l2")]
        {
            let first_block_number = msg.block_numbers.first().ok_or(RLPxError::BadRequest(
                "No block numbers found in BatchSealed message".to_string(),
            ))?;
            let last_block_number = msg.block_numbers.last().ok_or(RLPxError::BadRequest(
                "No block numbers found in BatchSealed message".to_string(),
            ))?;
            self.store_rollup
                .seal_batch(
                    msg.batch_number,
                    *first_block_number,
                    *last_block_number,
                    msg.withdrawal_hashes,
                )
                .await?;
            info!(
                "Sealed batch {} with blocks from {} to {}",
                msg.batch_number, first_block_number, last_block_number
            );
            Ok(())
        }
        #[cfg(not(feature = "l2"))]
        {
            Err(RLPxError::InternalError(
                "This function cannot won't be called without the l2 feature flag".to_string(),
            ))
        }
    }
    async fn init_peer_conn(&mut self) -> Result<(), RLPxError> {
        // Sending eth Status if peer supports it
        if let Some(eth) = self.negotiated_eth_capability.clone() {
            let status = backend::get_status(&self.storage, eth.version).await?;
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
                    backend::validate_status(msg_data, &self.storage, eth.version).await?
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
            block_msg @ Message::NewBlock(_) => {
                let block = Arc::new(block_msg);
                let task_id = tokio::task::id();
                let Ok(_) = self.connection_broadcast_send.send((task_id, block)) else {
                    let error_message = "Could not broadcast received block";
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
