use super::{
    eth::{transactions::NewPooledTransactionHashes, update::BlockRangeUpdate},
    p2p::DisconnectReason,
    utils::log_peer_warn,
};
#[cfg(feature = "l2")]
use crate::rlpx::based::GetBatchSealedResponseMessage;
#[cfg(feature = "l2")]
use crate::rlpx::based::NewBatchSealedMessage;
#[cfg(feature = "l2")]
use crate::rlpx::based::get_hash_batch_sealed;
use crate::rlpx::utils::get_pub_key;
use crate::{
    kademlia::PeerChannels,
    rlpx::{
        based::NewBlockMessage,
        error::RLPxError,
        eth::{
            backend,
            blocks::{BlockBodies, BlockHeaders},
            receipts::{GetReceipts, Receipts},
            status::StatusMessage,
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
#[cfg(feature = "l2")]
use ethrex_blockchain::sequencer_state::{SequencerState, SequencerStatus};
use ethrex_blockchain::{Blockchain, error::ChainError, fork_choice::apply_fork_choice};
use ethrex_common::{
    Address, H256, H512,
    types::{Block, MempoolTransaction, Transaction},
};
use ethrex_storage::Store;
#[cfg(feature = "l2")]
use ethrex_storage_rollup::StoreRollup;
use futures::SinkExt;
use k256::{PublicKey, SecretKey, ecdsa::SigningKey};
use rand::random;
#[cfg(feature = "l2")]
use secp256k1::Message as SignedMessage;
#[cfg(feature = "l2")]
use secp256k1::SecretKey as SigningKeySecp256k1;
use std::collections::BTreeMap;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{
        Mutex,
        broadcast::{self, error::RecvError},
        mpsc,
    },
    task,
    time::{Instant, sleep},
};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::debug;
use tracing::info;
const PERIODIC_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const PERIODIC_TX_BROADCAST_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
const PERIODIC_BLOCK_BROADCAST_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(500);
const PERIODIC_BATCH_BROADCAST_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(300);
const PERIODIC_TASKS_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
const PERIODIC_BLOCK_RANGE_UPDATE_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(60);
pub const MAX_PEERS_TCP_CONNECTIONS: usize = 200;

pub(crate) type Aes256Ctr64BE = ctr::Ctr64BE<aes::Aes256>;

pub(crate) type RLPxConnBroadcastSender = broadcast::Sender<(tokio::task::Id, H256, Arc<Message>)>;

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
    next_block_range_update: Instant,
    last_block_range_update_block: u64,
    broadcasted_txs: HashSet<H256>,
    next_block_broadcast: Instant,
    next_batch_broadcast: Instant,
    #[cfg(feature = "l2")]
    latest_block_sent: u64,
    #[cfg(feature = "l2")]
    last_block_broadcasted: u64,
    latest_block_added: u64,
    blocks_on_queue: BTreeMap<u64, Block>,
    #[cfg(feature = "l2")]
    latest_batch_sent: u64,
    requested_pooled_txs: HashMap<u64, NewPooledTransactionHashes>,
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
    #[cfg(feature = "l2")]
    committer_key: Option<SigningKeySecp256k1>,
    #[cfg(feature = "l2")]
    sequencer_state: SequencerState,
}

impl<S: AsyncWrite + AsyncRead + std::marker::Unpin> RLPxConnection<S> {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
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
        #[cfg(feature = "l2")] committer_key: Option<SigningKeySecp256k1>,
        #[cfg(feature = "l2")] sequencer_state: SequencerState,
    ) -> Self {
        #[cfg(feature = "l2")]
        let latest_batch_on_store = store_rollup.get_latest_batch_number().await.unwrap_or(0);
        #[cfg(feature = "l2")]
        let latest_block_on_store = storage.get_latest_block_number().await.unwrap_or(0);
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
            next_batch_broadcast: Instant::now() + PERIODIC_BATCH_BROADCAST_INTERVAL,
            next_block_range_update: Instant::now() + PERIODIC_BLOCK_RANGE_UPDATE_INTERVAL,
            last_block_range_update_block: 0,
            broadcasted_txs: HashSet::new(),
            #[cfg(feature = "l2")]
            latest_block_sent: latest_block_on_store,
            #[cfg(feature = "l2")]
            last_block_broadcasted: 0,
            latest_block_added: 0,
            blocks_on_queue: BTreeMap::new(),
            #[cfg(feature = "l2")]
            latest_batch_sent: latest_batch_on_store,
            requested_pooled_txs: HashMap::new(),
            client_version,
            connection_broadcast_send: connection_broadcast,
            #[cfg(feature = "l2")]
            store_rollup,
            based,
            #[cfg(feature = "l2")]
            committer_key,
            #[cfg(feature = "l2")]
            sequencer_state,
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
            RLPxError::BlockchainError(chain_err) => {
                log_peer_error(
                    &self.node,
                    &format!("Got chain err, peer will not be discarded: {chain_err}"),
                );
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
                debug!("Received disconnect message: {}", disconnect.reason());
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
                            self.handle_message(message.clone(), sender.clone()).await.inspect_err(|e| {log_peer_error(
                                &self.node,
                                &format!(
                                    "Error handling message {message}, error: {e}"
                                ),
                            );})?;
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
        receiver: &mut Option<broadcast::Receiver<(task::Id, H256, Arc<Message>)>>,
    ) -> Option<Result<(task::Id, H256, Arc<Message>), RecvError>> {
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
        if Instant::now() >= self.next_batch_broadcast {
            self.send_sealed_batch().await?;
            self.next_batch_broadcast = Instant::now() + PERIODIC_BATCH_BROADCAST_INTERVAL;
        }
        if Instant::now() >= self.next_block_range_update {
            self.next_block_range_update = Instant::now() + PERIODIC_BLOCK_RANGE_UPDATE_INTERVAL;
            if self.should_send_block_range_update().await? {
                self.send_block_range_update().await?;
            }
        };
        Ok(())
    }

    async fn send_new_pooled_tx_hashes(&mut self) -> Result<(), RLPxError> {
        if SUPPORTED_ETH_CAPABILITIES
            .iter()
            .any(|cap| self.capabilities.contains(cap))
        {
            // Exclude privileged transactions as they are created via the OnChainProposer contract
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
        // This section is conditionally compiled based on the "l2" feature flag due to dependencies on the rollup store.
        #[cfg(feature = "l2")]
        {
            if !self.capabilities.contains(&SUPPORTED_BASED_CAPABILITIES) {
                return Ok(());
            }
            let latest_block_number = self.storage.get_latest_block_number().await?;
            if !self.blockchain.is_synced() {
                // We do this since we are syncing, we don't want to broadcast old blocks
                self.latest_block_sent = latest_block_number;
                return Ok(());
            }
            for i in self.latest_block_sent + 1..=latest_block_number {
                debug!(
                    "Broadcasting new block, current: {}, last broadcasted: {}",
                    i, self.latest_block_sent
                );

                let new_block_body =
                    self.storage
                        .get_block_body(i)
                        .await?
                        .ok_or(RLPxError::InternalError(
                            "Block body not found after querying for the block number".to_owned(),
                        ))?;
                let new_block_header =
                    self.storage
                        .get_block_header(i)?
                        .ok_or(RLPxError::InternalError(
                            "Block header not found after querying for the block number".to_owned(),
                        ))?;
                let new_block = Block {
                    header: new_block_header,
                    body: new_block_body,
                };
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
                    let Some(secret_key) = self.committer_key else {
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
            self.latest_block_sent = latest_block_number;

            Ok(())
        }
        #[cfg(not(feature = "l2"))]
        {
            Ok(())
        }
    }

    async fn send_sealed_batch(&mut self) -> Result<(), RLPxError> {
        #[cfg(feature = "l2")]
        {
            if !self.blockchain.is_synced() {
                // We do this since we are syncing, we don't want to broadcast old batches
                self.latest_batch_sent = self.store_rollup.get_latest_batch_number().await?;
                return Ok(());
            }
            let next_batch_to_send = self.latest_batch_sent + 1;
            if !self
                .store_rollup
                .contains_batch(&next_batch_to_send)
                .await?
            {
                return Ok(());
            }
            let Some(batch) = self.store_rollup.get_batch(next_batch_to_send).await? else {
                return Ok(());
            };
            debug!(
                "Broadcasting new batch, current: {}, last broadcasted: {}",
                next_batch_to_send, self.latest_batch_sent
            );

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
                let Some(secret_key) = self.committer_key else {
                    return Err(RLPxError::InternalError(
                        "Secret key is not set for based connection".to_string(),
                    ));
                };
                let (recovery_id, signature) = secp256k1::SECP256K1
                    .sign_ecdsa_recoverable(
                        &SignedMessage::from_digest(get_hash_batch_sealed(&batch)),
                        &secret_key,
                    )
                    .serialize_compact();
                let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();
                (signature, recovery_id)
            };

            let msg = Message::NewBatchSealed(NewBatchSealedMessage {
                batch,
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
    async fn send_block_range_update(&mut self) -> Result<(), RLPxError> {
        // BlockRangeUpdate was introduced in eth/69
        if let Some(eth) = &self.negotiated_eth_capability {
            if eth.version >= 69 {
                log_peer_debug(&self.node, "Sending BlockRangeUpdate");
                let update = BlockRangeUpdate::new(&self.storage).await?;
                let lastet_block = update.lastest_block;
                self.send(Message::BlockRangeUpdate(update)).await?;
                self.last_block_range_update_block = lastet_block - (lastet_block % 32);
            }
        }
        Ok(())
    }

    async fn should_send_block_range_update(&mut self) -> Result<bool, RLPxError> {
        let latest_block = self.storage.get_latest_block_number().await?;
        if latest_block < self.last_block_range_update_block
            || latest_block - self.last_block_range_update_block >= 32
        {
            return Ok(true);
        }
        Ok(false)
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
                    backend::validate_status(msg_data, &self.storage, eth).await?
                };
            }
            Message::GetAccountRange(req) => {
                let response = process_account_range_request(req, self.storage.clone())?;
                self.send(Message::AccountRange(response)).await?
            }
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
                    if !valid_txs.is_empty() {
                        self.broadcast_message(Message::Transactions(Transactions::new(
                            valid_txs,
                        )))?;
                    }
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
                if let Some(eth) = &self.negotiated_eth_capability {
                    let mut receipts = Vec::new();
                    for hash in block_hashes.iter() {
                        receipts.push(self.storage.get_receipts_for_block(hash)?);
                    }
                    let response = Receipts::new(id, receipts, eth)?;
                    self.send(Message::Receipts(response)).await?;
                }
            }
            Message::BlockRangeUpdate(update) => {
                if update.earliest_block > update.lastest_block {
                    return Err(RLPxError::InvalidBlockRange);
                }

                //TODO implement the logic
                log_peer_debug(
                    &self.node,
                    &format!(
                        "Range block update: {} to {}",
                        update.earliest_block, update.lastest_block
                    ),
                );
            }
            Message::NewPooledTransactionHashes(new_pooled_transaction_hashes)
                if peer_supports_eth =>
            {
                let hashes =
                    new_pooled_transaction_hashes.get_transactions_to_request(&self.blockchain)?;

                let request_id = random();
                self.requested_pooled_txs
                    .insert(request_id, new_pooled_transaction_hashes);

                let request = GetPooledTransactions::new(request_id, hashes);
                self.send(Message::GetPooledTransactions(request)).await?;
            }
            Message::GetPooledTransactions(msg) => {
                let response = msg.handle(&self.blockchain)?;
                self.send(Message::PooledTransactions(response)).await?;
            }
            Message::PooledTransactions(msg) if peer_supports_eth => {
                if self.blockchain.is_synced() {
                    if let Some(requested) = self.requested_pooled_txs.get(&msg.id) {
                        if let Err(error) = msg.validate_requested(requested).await {
                            log_peer_warn(
                                &self.node,
                                &format!("disconnected from peer. Reason: {}", error),
                            );
                            self.send_disconnect_message(Some(DisconnectReason::SubprotocolError))
                                .await;
                            return Err(RLPxError::DisconnectSent(
                                DisconnectReason::SubprotocolError,
                            ));
                        } else {
                            self.requested_pooled_txs.remove(&msg.id);
                        }
                    }
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
            Message::NewBlock(_req) if peer_supports_based => {
                #[cfg(feature = "l2")]
                {
                    if let SequencerStatus::Following = self.sequencer_state.status().await {
                        if self.should_process_new_block(&_req).await? {
                            debug!("adding block to queue: {}", _req.block.header.number);
                            self.blocks_on_queue
                                .entry(_req.block.header.number)
                                .or_insert_with(|| _req.block.clone());
                            if _req.block.header.number > self.last_block_broadcasted {
                                self.broadcast_message(Message::NewBlock(_req.clone()))?;
                                self.last_block_broadcasted = _req.block.header.number;
                            }
                        }
                        // If the sequencer is following, we process the new block
                        // to keep the state updated.
                        self.process_block_on_queue(&_req).await?;
                    }
                }
            }
            Message::NewBatchSealed(_req) => {
                #[cfg(feature = "l2")]
                {
                    if self.should_process_batch_sealed(&_req).await? {
                        self.process_batch_sealed(&_req).await?;
                        // for now we broadcast valid messages
                        let new_latest_batch_sent = _req.batch.number;
                        self.broadcast_message(Message::NewBatchSealed(_req))?;
                        self.latest_batch_sent = new_latest_batch_sent;
                    }
                }
            }
            Message::GetBatchSealed(_req) => {
                #[cfg(feature = "l2")]
                {
                    if let SequencerStatus::Syncing = self.sequencer_state.status().await {
                        // If the sequencer is syncing, we won't send any batches
                        self.send(Message::GetBatchSealedResponse(
                            GetBatchSealedResponseMessage {
                                batches: vec![], // empty response
                                                 // for now skipping signatures
                            },
                        ))
                        .await?;
                        return Ok(());
                    }
                    let mut batches = vec![];
                    for batch_number in _req.first_batch..=_req.last_batch {
                        let Some(batch) = self.store_rollup.get_batch(batch_number).await? else {
                            return Err(RLPxError::InternalError(
                                "The node is not syncing and was asked for a batch it does not have".to_string(),
                            ));
                        };
                        batches.push(batch);
                    }

                    self.send(Message::GetBatchSealedResponse(
                        GetBatchSealedResponseMessage {
                            batches,
                            // for now skipping signatures
                        },
                    ))
                    .await?;
                }
            }
            Message::GetBatchSealedResponse(_req) if peer_supports_based => {
                #[cfg(feature = "l2")]
                {
                    if self.blockchain.is_synced() {
                        return Ok(());
                    }
                    sender.send(Message::GetBatchSealedResponse(_req)).await?;
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
        (id, node_id, broadcasted_msg): (task::Id, H256, Arc<Message>),
    ) -> Result<(), RLPxError> {
        if id != tokio::task::id() && node_id != self.node.node_id() {
            match broadcasted_msg.as_ref() {
                Message::Transactions(txs) => {
                    // TODO(#1131): Avoid cloning this vector.
                    let cloned = txs.transactions.clone();
                    let new_msg = Message::Transactions(Transactions {
                        transactions: cloned,
                    });
                    self.send(new_msg).await?;
                }
                Message::NewBlock(block_msg) => {
                    let new_msg = Message::NewBlock(block_msg.clone());
                    self.send(new_msg).await?;
                }
                Message::NewBatchSealed(batch_msg) => {
                    let new_msg = Message::NewBatchSealed(batch_msg.clone());
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

    async fn should_process_new_block(&mut self, msg: &NewBlockMessage) -> Result<bool, RLPxError> {
        if self.blocks_on_queue.contains_key(&msg.block.header.number) {
            debug!(
                "Block {} already in queue, ignoring it",
                msg.block.header.number
            );
            return Ok(false);
        }

        let block_hash = msg.block.hash();

        let recovered_lead_sequencer = get_pub_key(
            msg.recovery_id,
            &msg.signature,
            *block_hash.as_fixed_bytes(),
        )
        .map_err(|e| {
            log_peer_error(
                &self.node,
                &format!("Failed to recover lead sequencer: {e}"),
            );
            RLPxError::CryptographyError(e.to_string())
        })?;

        if !Self::validate_signature(recovered_lead_sequencer) {
            self.blocks_on_queue.remove(&msg.block.header.number);
            return Ok(false);
        }
        #[cfg(feature = "l2")]
        {
            // TODO: signature creation should be moved to a correct place, when producing the block
            let mut signature = [0u8; 68];
            signature[..64].copy_from_slice(&msg.signature[..]);
            signature[64..].copy_from_slice(&msg.recovery_id[..]);
            self.store_rollup
                .store_signature_by_block(block_hash, signature)
                .await?;
        }
        Ok(true)
    }

    fn validate_signature(_recovered_lead_sequencer: Address) -> bool {
        // Until the RPC module can be included in the P2P crate, we skip the validation
        true
    }

    async fn process_block_on_queue(&mut self, _msg: &NewBlockMessage) -> Result<(), RLPxError> {
        if !self.blockchain.is_synced() {
            let latest_block_in_storage = self.storage.get_latest_block_number().await?;
            self.latest_block_added = latest_block_in_storage;
            return Ok(());
        }
        let mut next_block_to_add = self.latest_block_added + 1;
        #[cfg(feature = "l2")]
        {
            let latest_batch_number = self.store_rollup.get_latest_batch_number().await?;
            if let Some(latest_batch) = self.store_rollup.get_batch(latest_batch_number).await? {
                next_block_to_add = next_block_to_add.max(latest_batch.last_block + 1);
            }
        }

        // TODO: clean old blocks that were added and then sync to a newer head
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
                    RLPxError::BlockchainError(ChainError::Custom(format!(
                        "Error adding new block {} with hash {:?}, error: {e}",
                        block.header.number,
                        block.hash()
                    )))
                })?;
            info!(
                "Added new block {} with hash {:?}",
                next_block_to_add, block_hash
            );
            self.latest_block_added = next_block_to_add;
            next_block_to_add += 1;
        }
        Ok(())
    }

    #[cfg(feature = "l2")]
    async fn should_process_batch_sealed(
        &mut self,
        msg: &NewBatchSealedMessage,
    ) -> Result<bool, RLPxError> {
        if !self.blockchain.is_synced() {
            debug!("Not processing new block, blockchain is not synced");
            return Ok(false);
        }
        if self.store_rollup.contains_batch(&msg.batch.number).await? {
            debug!("Batch {} already sealed, ignoring it", msg.batch.number);
            return Ok(false);
        }
        if msg.batch.first_block == msg.batch.last_block {
            // is empty batch
            return Ok(false);
        }
        if self.latest_block_added < msg.batch.last_block {
            debug!(
                "Not processing batch {} because the last block {} is not added yet",
                msg.batch.number, msg.batch.last_block
            );
            return Ok(false);
        }

        let hash = get_hash_batch_sealed(&msg.batch);

        let recovered_lead_sequencer =
            get_pub_key(msg.recovery_id, &msg.signature, hash).map_err(|e| {
                log_peer_error(
                    &self.node,
                    &format!("Failed to recover lead sequencer: {e}"),
                );
                RLPxError::CryptographyError(e.to_string())
            })?;

        if !Self::validate_signature(recovered_lead_sequencer) {
            return Ok(false);
        }

        let mut signature = [0u8; 68];
        signature[..64].copy_from_slice(&msg.signature[..]);
        signature[64..].copy_from_slice(&msg.recovery_id[..]);
        self.store_rollup
            .store_signature_by_batch(msg.batch.number, signature)
            .await?;
        Ok(true)
    }

    #[cfg(feature = "l2")]
    async fn process_batch_sealed(&mut self, msg: &NewBatchSealedMessage) -> Result<(), RLPxError> {
        self.store_rollup.seal_batch(msg.batch.clone()).await?;
        info!(
            "Sealed batch {} with blocks from {} to {}",
            msg.batch.number, msg.batch.first_block, msg.batch.last_block
        );
        Ok(())
    }
    async fn init_peer_conn(&mut self) -> Result<(), RLPxError> {
        // Sending eth Status if peer supports it
        if let Some(eth) = self.negotiated_eth_capability.clone() {
            let status = StatusMessage::new(&self.storage, &eth).await?;
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
                    backend::validate_status(msg_data, &self.storage, &eth).await?
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

    fn broadcast_message(&mut self, msg: Message) -> Result<(), RLPxError> {
        let node_id = self.node.node_id();
        match msg {
            txs_msg @ Message::Transactions(_) => {
                let txs = Arc::new(txs_msg);
                let task_id = tokio::task::id();
                let Ok(_) = self.connection_broadcast_send.send((task_id, node_id, txs)) else {
                    let error_message = "Could not broadcast received transactions";
                    log_peer_error(&self.node, error_message);
                    return Err(RLPxError::BroadcastError(error_message.to_owned()));
                };
                Ok(())
            }
            Message::NewBlock(new_block_msg) => {
                #[cfg(feature = "l2")]
                if self.last_block_broadcasted >= new_block_msg.block.header.number {
                    debug!(
                        "Block {} already broadcasted, ignoring it",
                        new_block_msg.block.header.number
                    );
                    return Ok(());
                } else {
                    self.last_block_broadcasted = new_block_msg.block.header.number;
                }
                let block = Arc::new(Message::NewBlock(new_block_msg));
                let task_id = tokio::task::id();
                let Ok(_) = self
                    .connection_broadcast_send
                    .send((task_id, node_id, block))
                else {
                    let error_message = "Could not broadcast received block";
                    log_peer_error(&self.node, error_message);
                    return Err(RLPxError::BroadcastError(error_message.to_owned()));
                };
                Ok(())
            }
            batch_msg @ Message::NewBatchSealed(_) => {
                let batch = Arc::new(batch_msg);
                let task_id = tokio::task::id();
                let Ok(_) = self
                    .connection_broadcast_send
                    .send((task_id, node_id, batch))
                else {
                    let error_message = "Could not broadcast received batch";
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

#[cfg(feature = "l2")]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::public_key_from_signing_key;

    use ethrex_blockchain::payload::{BuildPayloadArgs, create_payload};
    use ethrex_blockchain::sequencer_state::SequencerStatus;
    use ethrex_common::types::batch::Batch;
    use ethrex_common::{
        H160,
        types::{BlockHeader, ELASTICITY_MULTIPLIER},
    };
    use ethrex_storage::EngineType;
    use k256::SecretKey;
    use sha3::{Digest, Keccak256};
    use std::fs::File;
    use std::io::BufReader;
    use tokio::io::duplex;
    use tokio::sync::mpsc;

    /// Creates a new in-memory store for testing purposes
    /// Copied behavior from smoke_test.rs
    async fn test_store(path: &str) -> Store {
        // Get genesis
        let file = File::open("../../../test_data/genesis-execution-api.json")
            .expect("Failed to open genesis file");
        let reader = BufReader::new(file);
        let genesis = serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

        // Build store with genesis
        let store = Store::new(path, EngineType::InMemory).expect("Failed to build DB for testing");

        store
            .add_initial_state(genesis)
            .await
            .expect("Failed to add genesis state");

        store
    }

    /// Creates a new block using the blockchain's payload building logic,
    /// Copied behavior from smoke_test.rs
    async fn new_block(store: &Store, parent: &BlockHeader) -> Block {
        let args = BuildPayloadArgs {
            parent: parent.hash(),
            timestamp: parent.timestamp + 1, // Increment timestamp to be valid
            fee_recipient: H160::random(),
            random: H256::random(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::random()),
            version: 1,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
        };

        // Create a temporary blockchain instance to use its building logic
        let blockchain = Blockchain::default_with_store(store.clone());

        let block = create_payload(&args, store).unwrap();
        let result = blockchain.build_payload(block).await.unwrap();
        blockchain.add_block(&result.payload).await.unwrap();
        result.payload
    }

    /// A helper function to create an RLPxConnection for testing
    async fn create_rlpx_connection(
        signer: SigningKey,
        stream: tokio::io::DuplexStream,
        codec: RLPxCodec,
    ) -> RLPxConnection<tokio::io::DuplexStream> {
        let node = Node::new(
            "127.0.0.1".parse().unwrap(),
            30303,
            30303,
            public_key_from_signing_key(&signer),
        );
        let storage = test_store("store.db").await;
        let blockchain = Arc::new(Blockchain::default_with_store(storage.clone()));
        let (broadcast, _) = broadcast::channel(10);
        #[cfg(feature = "l2")]
        let committer_key = Some(SigningKeySecp256k1::new(&mut rand::rngs::OsRng));

        let mut connection = RLPxConnection::new(
            signer,
            node,
            stream,
            codec,
            storage,
            blockchain,
            "test-client/0.1.0".to_string(),
            broadcast,
            #[cfg(feature = "l2")]
            StoreRollup::default(),
            true,
            #[cfg(feature = "l2")]
            committer_key,
            #[cfg(feature = "l2")]
            SequencerState::from(SequencerStatus::default()),
        )
        .await;
        connection.capabilities.push(SUPPORTED_BASED_CAPABILITIES);
        connection.negotiated_eth_capability = Some(SUPPORTED_ETH_CAPABILITIES[0].clone());
        connection.blockchain.set_synced();
        // all have the same signing key for testing, for now the signature is not verified. In the future this will change
        connection.committer_key = Some(
            SigningKeySecp256k1::from_slice(
                &hex::decode("385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924")
                    .unwrap(),
            )
            .unwrap(),
        );

        connection
    }

    /// Helper function to create and send a NewBlock message for the test.
    async fn send_block(_conn: &mut RLPxConnection<tokio::io::DuplexStream>, _block: &Block) {
        let secret_key = _conn.committer_key.as_ref().unwrap();
        let (recovery_id, signature) = secp256k1::SECP256K1
            .sign_ecdsa_recoverable(
                &SignedMessage::from_digest(_block.hash().to_fixed_bytes()),
                secret_key,
            )
            .serialize_compact();
        let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();

        let message_to_send = Message::NewBlock(NewBlockMessage {
            block: _block.clone(),
            signature,
            recovery_id,
        });
        println!(
            "Sender (conn_a) sending block {} with hash {:?}.",
            _block.header.number,
            _block.hash()
        );
        _conn.send(message_to_send).await.unwrap();
    }

    /// Helper function to create and send a BatchSealed message for the test.
    async fn send_sealed_batch(
        conn: &mut RLPxConnection<tokio::io::DuplexStream>,
        batch_number: u64,
        first_block: u64,
        last_block: u64,
    ) {
        let batch = Batch {
            number: batch_number,
            first_block,
            last_block,
            ..Default::default()
        };
        let secret_key = conn.committer_key.as_ref().unwrap();
        let (recovery_id, signature) = secp256k1::SECP256K1
            .sign_ecdsa_recoverable(
                &SignedMessage::from_digest(get_hash_batch_sealed(&batch)),
                secret_key,
            )
            .serialize_compact();
        let recovery_id: [u8; 4] = recovery_id.to_i32().to_be_bytes();

        let message_to_send = Message::NewBatchSealed(NewBatchSealedMessage {
            batch,
            signature,
            recovery_id,
        });
        println!(
            "Sender (conn_a) sending sealed batch {} for blocks {}-{}.",
            batch_number, first_block, last_block
        );
        conn.send(message_to_send).await.unwrap();
    }

    async fn test_connections() -> (
        RLPxConnection<tokio::io::DuplexStream>,
        RLPxConnection<tokio::io::DuplexStream>,
    ) {
        // Stream for testing
        let (stream_a, stream_b) = duplex(4096);

        let eph_sk_a = SecretKey::random(&mut rand::rngs::OsRng);
        let nonce_a = H256::random();
        let eph_sk_b = SecretKey::random(&mut rand::rngs::OsRng);
        let nonce_b = H256::random();
        let hashed_nonces = Keccak256::digest([nonce_b.0, nonce_a.0].concat()).into();

        let local_state_a = LocalState {
            nonce: nonce_a,
            ephemeral_key: eph_sk_a.clone(),
            init_message: vec![],
        };
        let remote_state_a = RemoteState {
            nonce: nonce_b,
            ephemeral_key: eph_sk_b.public_key(),
            init_message: vec![],
            public_key: H512::zero(),
        };
        let codec_a = RLPxCodec::new(&local_state_a, &remote_state_a, hashed_nonces).unwrap();

        let local_state_b = LocalState {
            nonce: nonce_b,
            ephemeral_key: eph_sk_b,
            init_message: vec![],
        };
        let remote_state_b = RemoteState {
            nonce: nonce_a,
            ephemeral_key: eph_sk_a.public_key(),
            init_message: vec![],
            public_key: H512::zero(),
        };
        let codec_b = RLPxCodec::new(&local_state_b, &remote_state_b, hashed_nonces).unwrap();

        // Create the two RLPxConnection instances
        let conn_a = create_rlpx_connection(
            SigningKey::random(&mut rand::rngs::OsRng),
            stream_a,
            codec_a,
        )
        .await;
        let conn_b = create_rlpx_connection(
            SigningKey::random(&mut rand::rngs::OsRng),
            stream_b,
            codec_b,
        )
        .await;

        (conn_a, conn_b)
    }

    #[tokio::test]
    /// Tests to ensure that blocks are added in the correct order to the RLPxConnection when received out of order.
    async fn add_block_in_correct_order() {
        let (mut conn_a, mut conn_b) = test_connections().await;

        let b_task = tokio::spawn(async move {
            println!("Receiver task (conn_b) started.");
            let mut blocks_received_count = 0;

            loop {
                let Some(Ok(message)) = conn_b.receive().await else {
                    println!("Receiver task (conn_b) stream ended or failed.");
                    break;
                };

                let Message::NewBlock(msg) = message else {
                    continue;
                };

                blocks_received_count += 1;
                println!(
                    "Receiver task received block {}. Total received: {}",
                    msg.block.header.number, blocks_received_count
                );

                // Process the message
                let (dummy_tx, _) = mpsc::channel(1);
                match conn_b
                    .handle_message(Message::NewBlock(msg.clone()), dummy_tx)
                    .await
                {
                    Ok(_) | Err(RLPxError::BroadcastError(_)) => {}
                    Err(e) => panic!("handle_message failed: {:?}", e),
                }

                // Perform assertions based on how many blocks have been received
                match blocks_received_count {
                    1 => {
                        // Received block 3. No checks yet.
                    }
                    2 => {
                        // Received block 2. Now check intermediate state.
                        println!("Receiver task: Checking intermediate state...");
                        assert_eq!(
                            conn_b.blocks_on_queue.len(),
                            2,
                            "Queue should contain blocks 2 and 3"
                        );
                        assert!(conn_b.blocks_on_queue.contains_key(&2));
                        assert!(conn_b.blocks_on_queue.contains_key(&3));
                        assert_eq!(
                            conn_b.latest_block_added, 0,
                            "No blocks should be added to the chain yet"
                        );
                    }
                    3 => {
                        // Received block 1. Now check final state.
                        println!("Receiver task: Checking final state...");
                        assert!(
                            conn_b.blocks_on_queue.is_empty(),
                            "Queue should be empty after processing"
                        );
                        assert_eq!(
                            conn_b.latest_block_added, 3,
                            "All blocks up to 3 should have been added"
                        );
                        break; // Test is complete, exit the loop
                    }
                    _ => panic!("Received more blocks than expected"),
                }
            }
        });

        // Here we create a new store for simulating another node and create blocks to be sent
        let storage_2 = test_store("store_2.db").await;
        let genesis_header = storage_2.get_block_header(0).unwrap().unwrap();
        let block1 = new_block(&storage_2, &genesis_header).await;
        let block2 = new_block(&storage_2, &block1.header).await;
        let block3 = new_block(&storage_2, &block2.header).await;

        // Send blocks in reverse order
        send_block(&mut conn_a, &block3).await;
        send_block(&mut conn_a, &block2).await;

        // Send the final block that allows the queue to be processed
        send_block(&mut conn_a, &block1).await;

        // wait for the receiver task to finish
        match b_task.await {
            Ok(_) => println!("Receiver task completed successfully."),
            Err(e) => panic!("Receiver task failed: {:?}", e),
        }
    }

    #[tokio::test]
    /// Tests that a batch can be sealed after all its blocks have been received.
    async fn test_seal_batch_with_blocks() {
        let (mut conn_a, mut conn_b) = test_connections().await;

        let b_task = tokio::spawn(async move {
            println!("Receiver task (conn_b) started.");
            let mut blocks_received_count = 0;

            loop {
                let Some(Ok(message)) = conn_b.receive().await else {
                    println!("Receiver task (conn_b) stream ended or failed.");
                    break;
                };

                let (dummy_tx, _) = mpsc::channel(1);
                match message {
                    Message::NewBlock(msg) => {
                        blocks_received_count += 1;
                        println!(
                            "Receiver task received block {}. Total received: {}",
                            msg.block.header.number, blocks_received_count
                        );

                        // Process the message
                        match conn_b
                            .handle_message(Message::NewBlock(msg.clone()), dummy_tx)
                            .await
                        {
                            Ok(_) | Err(RLPxError::BroadcastError(_)) => {}
                            Err(e) => panic!("handle_message failed: {:?}", e),
                        }

                        if blocks_received_count == 3 {
                            println!("Receiver task: All blocks received, checking state...");
                            assert_eq!(
                                conn_b.latest_block_added, 3,
                                "All blocks up to 3 should have been added"
                            );
                        }
                    }
                    Message::NewBatchSealed(msg) => {
                        println!("Receiver task received sealed batch {}.", msg.batch.number);
                        // Process the message
                        match conn_b
                            .handle_message(Message::NewBatchSealed(msg.clone()), dummy_tx)
                            .await
                        {
                            Ok(_) | Err(RLPxError::BroadcastError(_)) => {}
                            Err(e) => panic!("handle_message failed: {:?}", e),
                        }

                        println!("Receiver task: Checking for sealed batch...");
                        assert!(
                            conn_b.store_rollup.contains_batch(&1).await.unwrap(),
                            "Batch 1 should be sealed in the store"
                        );
                        break; // Test complete
                    }
                    _ => panic!("Received unexpected message type in receiver task"),
                }
            }
        });

        let storage = test_store("store_for_sending.db").await;
        let genesis_header = storage.get_block_header(0).unwrap().unwrap();
        let block1 = new_block(&storage, &genesis_header).await;
        let block2 = new_block(&storage, &block1.header).await;
        let block3 = new_block(&storage, &block2.header).await;

        // Send blocks in order
        send_block(&mut conn_a, &block1).await;
        send_block(&mut conn_a, &block2).await;
        send_block(&mut conn_a, &block3).await;

        // Now send the sealed batch message
        send_sealed_batch(&mut conn_a, 1, 1, 3).await;

        // Wait for the receiver task to finish
        match b_task.await {
            Ok(_) => println!("Receiver task completed successfully."),
            Err(e) => panic!("Receiver task failed: {:?}", e),
        }
    }

    #[tokio::test]
    /// Tests that a batch cannot be sealed after all its blocks have been received.
    async fn test_batch_not_seal_with_missing_blocks() {
        let (mut conn_a, mut conn_b) = test_connections().await;

        let b_task = tokio::spawn(async move {
            println!("Receiver task (conn_b) started.");
            let mut blocks_received_count = 0;

            loop {
                let Some(Ok(message)) = conn_b.receive().await else {
                    println!("Receiver task (conn_b) stream ended or failed.");
                    break;
                };

                let (dummy_tx, _) = mpsc::channel(1);
                match message {
                    Message::NewBlock(msg) => {
                        blocks_received_count += 1;
                        println!(
                            "Receiver task received block {}. Total received: {}",
                            msg.block.header.number, blocks_received_count
                        );

                        // Process the message
                        match conn_b
                            .handle_message(Message::NewBlock(msg.clone()), dummy_tx)
                            .await
                        {
                            Ok(_) | Err(RLPxError::BroadcastError(_)) => {}
                            Err(e) => panic!("handle_message failed: {:?}", e),
                        }

                        if blocks_received_count == 3 {
                            println!("Receiver task: All blocks received, checking state...");
                            assert_eq!(
                                conn_b.latest_block_added, 3,
                                "All blocks up to 3 should have been added"
                            );
                        }
                    }
                    Message::NewBatchSealed(msg) => {
                        println!("Receiver task received sealed batch {}.", msg.batch.number);
                        // Process the message
                        match conn_b
                            .handle_message(Message::NewBatchSealed(msg.clone()), dummy_tx)
                            .await
                        {
                            Ok(_) | Err(RLPxError::BroadcastError(_)) => {}
                            Err(e) => panic!("handle_message failed: {:?}", e),
                        }

                        println!("Receiver task: Checking for sealed batch...");
                        assert!(
                            !conn_b.store_rollup.contains_batch(&1).await.unwrap(),
                            "Batch 1 should not be sealed in the store"
                        );
                        break; // Test complete
                    }
                    _ => panic!("Received unexpected message type in receiver task"),
                }
            }
        });

        let storage = test_store("store_for_sending.db").await;
        let genesis_header = storage.get_block_header(0).unwrap().unwrap();
        let block1 = new_block(&storage, &genesis_header).await;
        let block2 = new_block(&storage, &block1.header).await;
        // Skip the third block

        // Send blocks in order
        send_block(&mut conn_a, &block1).await;
        send_block(&mut conn_a, &block2).await;
        // Skip the third block

        // Now send the sealed batch message
        send_sealed_batch(&mut conn_a, 1, 1, 3).await;

        // Wait for the receiver task to finish
        match b_task.await {
            Ok(_) => println!("Receiver task completed successfully."),
            Err(e) => panic!("Receiver task failed: {:?}", e),
        }
    }
}
