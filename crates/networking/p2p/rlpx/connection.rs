use std::sync::Arc;

use crate::{
    peer_channels::PeerChannels,
    rlpx::{
        eth::{
            backend,
            blocks::{BlockBodies, BlockHeaders},
            receipts::Receipts,
            transactions::Transactions,
        },
        handshake::encode_ack_message,
        message::Message,
        p2p::{self, DisconnectMessage, PingMessage, PongMessage},
        utils::id2pubkey,
    },
    snap::{
        process_account_range_request, process_byte_codes_request, process_storage_ranges_request,
        process_trie_nodes_request,
    },
    MAX_DISC_PACKET_SIZE,
};

use super::{
    error::RLPxError,
    eth::{receipts::GetReceipts, transactions::GetPooledTransactions},
    frame::RLPxCodec,
    handshake::{decode_ack_message, decode_auth_message, encode_auth_message},
    message as rlpx,
    p2p::Capability,
    utils::pubkey2id,
};
use ethrex_blockchain::mempool::{self};
use ethrex_core::{H256, H512};
use ethrex_storage::Store;
use futures::SinkExt;
use k256::{
    ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey},
    PublicKey, SecretKey,
};
use rand::random;
use sha3::{Digest, Keccak256};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{
        broadcast::{self, error::RecvError},
        mpsc, Mutex,
    },
    task,
    time::{sleep, Instant},
};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::{debug, error};
const CAP_P2P: (Capability, u8) = (Capability::P2p, 5);
const CAP_ETH: (Capability, u8) = (Capability::Eth, 68);
const CAP_SNAP: (Capability, u8) = (Capability::Snap, 1);
const SUPPORTED_CAPABILITIES: [(Capability, u8); 3] = [CAP_P2P, CAP_ETH, CAP_SNAP];
const PERIODIC_TASKS_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

pub(crate) type Aes256Ctr64BE = ctr::Ctr64BE<aes::Aes256>;

/// Fully working RLPx connection.
pub(crate) struct RLPxConnection<S> {
    signer: SigningKey,
    remote_node_id: H512,
    state: RLPxConnectionState,
    framed: Framed<S, RLPxCodec>,
    storage: Store,
    capabilities: Vec<(Capability, u8)>,
    next_periodic_task_check: Instant,
    /// Send end of the channel used to broadcast messages
    /// to other connected peers, is ok to have it here,
    /// since internally it's an Arc.
    /// The ID is to ignore the message sent from the same task.
    /// This is used both to send messages and to received broadcasted
    /// messages from other connections (sent from other peers).
    /// The receive end is instantiated after the handshake is completed
    /// under `handle_peer`.
    connection_broadcast_send: broadcast::Sender<(task::Id, Arc<Message>)>,
}

impl<S: AsyncWrite + AsyncRead + std::marker::Unpin> RLPxConnection<S> {
    fn new(
        signer: SigningKey,
        remote_node_id: H512,
        stream: S,
        state: RLPxConnectionState,
        storage: Store,
        connection_broadcast: broadcast::Sender<(task::Id, Arc<Message>)>,
    ) -> Self {
        Self {
            signer,
            remote_node_id,
            state,
            // Creating RLPxCodec with default values. They will be updated during the handshake
            framed: Framed::new(stream, RLPxCodec::default()),
            storage,
            capabilities: vec![],
            next_periodic_task_check: Instant::now() + PERIODIC_TASKS_CHECK_INTERVAL,
            connection_broadcast_send: connection_broadcast,
        }
    }

    pub fn receiver(
        signer: SigningKey,
        stream: S,
        storage: Store,
        connection_broadcast: broadcast::Sender<(task::Id, Arc<Message>)>,
    ) -> Self {
        let mut rng = rand::thread_rng();
        Self::new(
            signer,
            // remote_node_id not yet provided. It will be replaced later with correct one.
            H512::default(),
            stream,
            RLPxConnectionState::Receiver(Receiver {
                nonce: H256::random_using(&mut rng),
                ephemeral_key: SecretKey::random(&mut rng),
            }),
            storage,
            connection_broadcast,
        )
    }

    pub fn initiator(
        signer: SigningKey,
        msg: &[u8],
        stream: S,
        storage: Store,
        connection_broadcast_send: broadcast::Sender<(task::Id, Arc<Message>)>,
    ) -> Result<Self, RLPxError> {
        let mut rng = rand::thread_rng();
        let digest = Keccak256::digest(msg.get(65..).ok_or(RLPxError::InvalidMessageLength())?);
        let signature = &Signature::from_bytes(
            msg.get(..64)
                .ok_or(RLPxError::InvalidMessageLength())?
                .into(),
        )?;
        let rid = RecoveryId::from_byte(*msg.get(64).ok_or(RLPxError::InvalidMessageLength())?)
            .ok_or(RLPxError::InvalidRecoveryId())?;
        let peer_pk = VerifyingKey::recover_from_prehash(&digest, signature, rid)?;
        let state = RLPxConnectionState::Initiator(Initiator {
            nonce: H256::random_using(&mut rng),
            ephemeral_key: SecretKey::random(&mut rng),
        });
        Ok(RLPxConnection::new(
            signer,
            pubkey2id(&peer_pk.into()),
            stream,
            state,
            storage,
            connection_broadcast_send,
        ))
    }

    /// Starts a handshake and runs the peer connection.
    /// It runs in it's own task and blocks until the connection is dropped
    pub async fn start_peer(&mut self, table: Arc<Mutex<crate::kademlia::KademliaTable>>) {
        // Perform handshake
        if let Err(e) = self.handshake().await {
            self.peer_conn_failed("Handshake failed", e, table).await;
        } else {
            // Handshake OK: handle connection
            // Create channels to communicate directly to the peer
            let (peer_channels, sender, receiver) = PeerChannels::create();
            let capabilities = self
                .capabilities
                .iter()
                .map(|(cap, _)| cap.clone())
                .collect();
            table.lock().await.init_backend_communication(
                self.remote_node_id,
                peer_channels,
                capabilities,
            );
            if let Err(e) = self.handle_peer_conn(sender, receiver).await {
                self.peer_conn_failed("Error during RLPx connection", e, table)
                    .await;
            }
        }
    }

    async fn peer_conn_failed(
        &mut self,
        error_text: &str,
        error: RLPxError,
        table: Arc<Mutex<crate::kademlia::KademliaTable>>,
    ) {
        self.send(Message::Disconnect(DisconnectMessage {
            reason: self.match_disconnect_reason(&error),
        }))
        .await
        .unwrap_or_else(|e| error!("Could not send Disconnect message: ({e})."));

        // Discard peer from kademlia table
        let remote_node_id = self.remote_node_id;
        error!("{error_text}: ({error}), discarding peer {remote_node_id}");
        table.lock().await.replace_peer(remote_node_id);
    }

    fn match_disconnect_reason(&self, error: &RLPxError) -> Option<u8> {
        match error {
            RLPxError::RLPDecodeError(_) => Some(2_u8),
            // TODO build a proper matching between error types and disconnection reasons
            _ => None,
        }
    }

    async fn handshake(&mut self) -> Result<(), RLPxError> {
        match &self.state {
            RLPxConnectionState::Initiator(_) => {
                self.send_auth().await?;
                self.receive_ack().await?;
            }
            RLPxConnectionState::Receiver(_) => {
                self.receive_auth().await?;
                self.send_ack().await?;
            }
            _ => {
                return Err(RLPxError::HandshakeError(
                    "Invalid connection state for handshake".to_string(),
                ))
            }
        };
        debug!("Completed handshake!");

        self.exchange_hello_messages().await?;
        Ok(())
    }

    async fn exchange_hello_messages(&mut self) -> Result<(), RLPxError> {
        let hello_msg = Message::Hello(p2p::HelloMessage::new(
            SUPPORTED_CAPABILITIES.to_vec(),
            PublicKey::from(self.signer.verifying_key()),
        ));

        self.send(hello_msg).await?;

        // Receive Hello message
        match self.receive().await? {
            Message::Hello(hello_message) => {
                self.capabilities = hello_message.capabilities;

                // Check if we have any capability in common
                for cap in self.capabilities.clone() {
                    if SUPPORTED_CAPABILITIES.contains(&cap) {
                        return Ok(());
                    }
                }
                // Return error if not
                Err(RLPxError::HandshakeError(
                    "No matching capabilities".to_string(),
                ))
            }
            Message::Disconnect(disconnect) => Err(RLPxError::HandshakeError(format!(
                "Peer disconnected due to: {}",
                disconnect.reason()
            ))),
            _ => {
                // Fail if it is not a hello message
                Err(RLPxError::HandshakeError(
                    "Expected Hello message".to_string(),
                ))
            }
        }
    }

    async fn handle_peer_conn(
        &mut self,
        sender: mpsc::Sender<rlpx::Message>,
        mut receiver: mpsc::Receiver<rlpx::Message>,
    ) -> Result<(), RLPxError> {
        if let RLPxConnectionState::Established() = &self.state {
            self.init_peer_conn().await?;
            debug!("Started peer main loop");
            // Wait for eth status message or timeout.
            let mut broadcaster_receive = {
                if self.capabilities.contains(&CAP_ETH) {
                    Some(self.connection_broadcast_send.subscribe())
                } else {
                    None
                }
            };

            // Status message received, start listening for connections,
            // and subscribe this connection to the broadcasting.
            loop {
                tokio::select! {
                    // Expect a message from the remote peer
                    message = self.receive() => {
                        self.handle_message(message?, sender.clone()).await?;
                    }
                    // Expect a message from the backend
                    Some(message) = receiver.recv() => {
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
        } else {
            Err(RLPxError::InvalidState())
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
            debug!("Ping sent");
            self.next_periodic_task_check = Instant::now() + PERIODIC_TASKS_CHECK_INTERVAL;
        };
        Ok(())
    }

    async fn handle_message(
        &mut self,
        message: Message,
        sender: mpsc::Sender<Message>,
    ) -> Result<(), RLPxError> {
        let peer_supports_eth = self.capabilities.contains(&CAP_ETH);
        match message {
            Message::Disconnect(msg_data) => {
                debug!("Received Disconnect: {}", msg_data.reason());
                // Returning a Disconnect error to be handled later at the call stack
                return Err(RLPxError::Disconnect());
            }
            Message::Ping(_) => {
                self.send(Message::Pong(PongMessage {})).await?;
                debug!("Pong sent");
            }
            Message::Pong(_) => {
                // We ignore received Pong messages
            }
            Message::Status(msg_data) if !peer_supports_eth => {
                backend::validate_status(msg_data, &self.storage)?
            }
            Message::GetAccountRange(req) => {
                let response = process_account_range_request(req, self.storage.clone())?;
                self.send(Message::AccountRange(response)).await?
            }
            // TODO(#1129) Add the transaction to the mempool once received.
            Message::Transactions(txs) if peer_supports_eth => {
                for tx in &txs.transactions {
                    mempool::add_transaction(tx.clone(), &self.storage)?;
                }
                self.broadcast_message(Message::Transactions(txs)).await?;
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
                msg.handle(&self.storage)?;
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
                    error!("Unsupported message was broadcasted: {msg}");
                    return Err(RLPxError::BroadcastError(format!(
                        "Non-supported message broadcasted {}",
                        msg
                    )));
                }
            }
        }
        Ok(())
    }

    async fn init_peer_conn(&mut self) -> Result<(), RLPxError> {
        // Sending eth Status if peer supports it
        if self.capabilities.contains(&CAP_ETH) {
            let status = backend::get_status(&self.storage)?;
            debug!("Sending status");
            self.send(Message::Status(status)).await?;
            // The next immediate message in the ETH protocol is the
            // status, reference here:
            // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
            match self.receive().await? {
                Message::Status(msg_data) => {
                    // TODO: Check message status is correct.
                    debug!("Received Status");
                    backend::validate_status(msg_data, &self.storage)?
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

    async fn send_auth(&mut self) -> Result<(), RLPxError> {
        if let RLPxConnectionState::Initiator(initiator_state) = &self.state {
            let secret_key: SecretKey = self.signer.clone().into();
            let peer_pk = id2pubkey(self.remote_node_id).ok_or(RLPxError::InvalidPeerId())?;

            // Clonning previous state to avoid ownership issues
            let previous_state = initiator_state.clone();

            let msg = encode_auth_message(
                &secret_key,
                previous_state.nonce,
                &peer_pk,
                &previous_state.ephemeral_key,
            )?;

            self.send_handshake_msg(&msg).await?;

            self.state = RLPxConnectionState::InitiatedAuth(InitiatedAuth {
                local_nonce: previous_state.nonce,
                local_ephemeral_key: previous_state.ephemeral_key,
                local_init_message: msg,
            });
            Ok(())
        } else {
            Err(RLPxError::InvalidState())
        }
    }

    async fn send_ack(&mut self) -> Result<(), RLPxError> {
        if let RLPxConnectionState::ReceivedAuth(received_auth_state) = &self.state {
            let peer_pk = id2pubkey(self.remote_node_id).ok_or(RLPxError::InvalidPeerId())?;

            // Clonning previous state to avoid ownership issues
            let previous_state = received_auth_state.clone();

            let msg = encode_ack_message(
                &previous_state.local_ephemeral_key,
                previous_state.local_nonce,
                &peer_pk,
            )?;

            self.send_handshake_msg(&msg).await?;

            // keccak256(nonce || initiator-nonce)
            // Remote node is initator
            let hashed_nonces = Keccak256::digest(
                [previous_state.local_nonce.0, previous_state.remote_nonce.0].concat(),
            )
            .into();

            self.framed.codec_mut().update_secrets(
                msg,
                previous_state.local_nonce,
                previous_state.local_ephemeral_key,
                hashed_nonces,
                previous_state.remote_init_message,
                previous_state.remote_nonce,
                previous_state.remote_ephemeral_key,
            );

            self.state = RLPxConnectionState::Established();

            Ok(())
        } else {
            Err(RLPxError::InvalidState())
        }
    }

    async fn receive_auth(&mut self) -> Result<(), RLPxError> {
        if let RLPxConnectionState::Receiver(receiver_state) = &self.state {
            let secret_key: SecretKey = self.signer.clone().into();
            // Clonning previous state to avoid ownership issues
            let previous_state = receiver_state.clone();
            let msg_bytes = self.receive_handshake_msg().await?;
            let size_data = &msg_bytes
                .get(..2)
                .ok_or(RLPxError::InvalidMessageLength())?;
            let msg = &msg_bytes
                .get(2..)
                .ok_or(RLPxError::InvalidMessageLength())?;
            let (auth, remote_ephemeral_key) = decode_auth_message(&secret_key, msg, size_data)?;
            self.remote_node_id = auth.node_id;

            // Build next state
            self.state = RLPxConnectionState::ReceivedAuth(ReceivedAuth {
                local_nonce: previous_state.nonce,
                local_ephemeral_key: previous_state.ephemeral_key,
                remote_nonce: auth.nonce,
                remote_ephemeral_key,
                remote_init_message: msg_bytes.to_owned(),
            });
            Ok(())
        } else {
            Err(RLPxError::InvalidState())
        }
    }

    async fn receive_ack(&mut self) -> Result<(), RLPxError> {
        if let RLPxConnectionState::InitiatedAuth(initiated_auth_state) = &self.state {
            let secret_key: SecretKey = self.signer.clone().into();
            // Clonning previous state to avoid ownership issues
            let previous_state = initiated_auth_state.clone();
            let msg_bytes = self.receive_handshake_msg().await?;
            let size_data = &msg_bytes
                .get(..2)
                .ok_or(RLPxError::InvalidMessageLength())?;
            let msg = &msg_bytes
                .get(2..)
                .ok_or(RLPxError::InvalidMessageLength())?;
            let ack = decode_ack_message(&secret_key, msg, size_data)?;
            let remote_ephemeral_key = ack
                .get_ephemeral_pubkey()
                .ok_or(RLPxError::NotFound("Remote ephemeral key".to_string()))?;

            // keccak256(nonce || initiator-nonce)
            // Local node is initator
            let hashed_nonces =
                Keccak256::digest([ack.nonce.0, previous_state.local_nonce.0].concat()).into();

            self.framed.codec_mut().update_secrets(
                previous_state.local_init_message,
                previous_state.local_nonce,
                previous_state.local_ephemeral_key,
                hashed_nonces,
                msg_bytes.to_owned(),
                ack.nonce,
                remote_ephemeral_key,
            );

            self.state = RLPxConnectionState::Established();
            Ok(())
        } else {
            Err(RLPxError::InvalidState())
        }
    }

    async fn send_handshake_msg(&mut self, msg: &[u8]) -> Result<(), RLPxError> {
        self.framed.get_mut().write_all(msg).await?;
        Ok(())
    }

    async fn receive_handshake_msg(&mut self) -> Result<Vec<u8>, RLPxError> {
        let mut buf = vec![0; MAX_DISC_PACKET_SIZE];

        // Read the message's size
        self.framed.get_mut().read_exact(&mut buf[..2]).await?;
        let ack_data = [buf[0], buf[1]];
        let msg_size = u16::from_be_bytes(ack_data) as usize;

        // Read the rest of the message
        self.framed
            .get_mut()
            .read_exact(&mut buf[2..msg_size + 2])
            .await?;
        let ack_bytes = &buf[..msg_size + 2];
        Ok(ack_bytes.to_vec())
    }

    async fn send(&mut self, message: rlpx::Message) -> Result<(), RLPxError> {
        if let RLPxConnectionState::Established() = &mut self.state {
            self.framed.send(message).await
        } else {
            Err(RLPxError::InvalidState())
        }
    }

    async fn receive(&mut self) -> Result<rlpx::Message, RLPxError> {
        if let RLPxConnectionState::Established() = &mut self.state {
            if let Some(message) = self.framed.next().await {
                message
            } else {
                Err(RLPxError::Disconnect())
            }
        } else {
            Err(RLPxError::InvalidState())
        }
    }

    pub async fn broadcast_message(&self, msg: Message) -> Result<(), RLPxError> {
        match msg {
            txs_msg @ Message::Transactions(_) => {
                let txs = Arc::new(txs_msg);
                let task_id = tokio::task::id();
                let Ok(_) = self.connection_broadcast_send.send((task_id, txs)) else {
                    error!("Could not broadcast message in task!");
                    return Err(RLPxError::BroadcastError(
                        "Could not broadcast received transactions".to_owned(),
                    ));
                };
                Ok(())
            }
            msg => {
                error!("Non supported message: {msg} was tried to be broadcasted");
                Err(RLPxError::BroadcastError(format!(
                    "Broadcasting for msg: {msg} is not supported"
                )))
            }
        }
    }
}

enum RLPxConnectionState {
    Initiator(Initiator),
    Receiver(Receiver),
    ReceivedAuth(ReceivedAuth),
    InitiatedAuth(InitiatedAuth),
    Established(),
}

#[derive(Clone)]
struct Receiver {
    pub(crate) nonce: H256,
    pub(crate) ephemeral_key: SecretKey,
}

#[derive(Clone)]
struct Initiator {
    pub(crate) nonce: H256,
    pub(crate) ephemeral_key: SecretKey,
}

#[derive(Clone)]
struct ReceivedAuth {
    pub(crate) local_nonce: H256,
    pub(crate) local_ephemeral_key: SecretKey,
    pub(crate) remote_nonce: H256,
    pub(crate) remote_ephemeral_key: PublicKey,
    pub(crate) remote_init_message: Vec<u8>,
}

#[derive(Clone)]
struct InitiatedAuth {
    pub(crate) local_nonce: H256,
    pub(crate) local_ephemeral_key: SecretKey,
    pub(crate) local_init_message: Vec<u8>,
}

#[derive(Clone)]
pub struct Established {}
