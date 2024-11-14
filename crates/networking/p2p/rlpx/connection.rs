use std::sync::Arc;

use crate::{
    rlpx::{
        eth::{
            backend,
            blocks::{BlockBodies, BlockHeaders},
            transactions::Transactions,
        },
        handshake::encode_ack_message,
        message::Message,
        p2p::{self, PingMessage, PongMessage},
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
    frame,
    handshake::{decode_ack_message, decode_auth_message, encode_auth_message},
    message as rlpx,
    p2p::Capability,
    utils::{ecdh_xchng, pubkey2id},
};
use aes::cipher::KeyIvInit;
use ethereum_rust_core::{H256, H512};
use ethereum_rust_rlp::decode::RLPDecode;
use ethereum_rust_storage::Store;
use k256::{
    ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey},
    PublicKey, SecretKey,
};
use sha3::{Digest, Keccak256};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::broadcast::{self, error::RecvError},
    task,
    time::{sleep, Instant},
};
use tracing::{error, info};
const CAP_P2P: (Capability, u8) = (Capability::P2p, 5);
const CAP_ETH: (Capability, u8) = (Capability::Eth, 68);
const CAP_SNAP: (Capability, u8) = (Capability::Snap, 1);
const SUPPORTED_CAPABILITIES: [(Capability, u8); 3] = [CAP_P2P, CAP_ETH, CAP_SNAP];
const PERIODIC_TASKS_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

pub(crate) type Aes256Ctr64BE = ctr::Ctr64BE<aes::Aes256>;

/// Fully working RLPx connection.
pub(crate) struct RLPxConnection<S> {
    signer: SigningKey,
    state: RLPxConnectionState,
    stream: S,
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
        stream: S,
        state: RLPxConnectionState,
        storage: Store,
        connection_broadcast: broadcast::Sender<(task::Id, Arc<Message>)>,
    ) -> Self {
        Self {
            signer,
            state,
            stream,
            storage,
            capabilities: vec![],
            next_periodic_task_check: Instant::now() + PERIODIC_TASKS_CHECK_INTERVAL,
            connection_broadcast_send: connection_broadcast.clone(),
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
            stream,
            RLPxConnectionState::Receiver(Receiver::new(
                H256::random_using(&mut rng),
                SecretKey::random(&mut rng),
            )),
            storage,
            connection_broadcast,
        )
    }

    pub async fn initiator(
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
        let state = RLPxConnectionState::Initiator(Initiator::new(
            H256::random_using(&mut rng),
            SecretKey::random(&mut rng),
            pubkey2id(&peer_pk.into()),
        ));
        Ok(RLPxConnection::new(
            signer,
            stream,
            state,
            storage,
            connection_broadcast_send,
        ))
    }

    pub async fn handshake(&mut self) -> Result<(), RLPxError> {
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
        info!("Completed handshake!");

        self.exchange_hello_messages().await?;
        Ok(())
    }

    pub async fn exchange_hello_messages(&mut self) -> Result<(), RLPxError> {
        let hello_msg = Message::Hello(p2p::HelloMessage::new(
            SUPPORTED_CAPABILITIES.to_vec(),
            PublicKey::from(self.signer.verifying_key()),
        ));

        self.send(hello_msg).await?;

        // Receive Hello message
        if let Message::Hello(hello_message) = self.receive().await? {
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
        } else {
            // Fail if it is not a hello message
            Err(RLPxError::HandshakeError(
                "Expected Hello message".to_string(),
            ))
        }
    }

    pub async fn handle_peer_conn(&mut self) -> Result<(), RLPxError> {
        if let RLPxConnectionState::Established(_) = &self.state {
            self.init_peer_conn().await?;
            info!("Started peer main loop");
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
                    // TODO check if this is cancel safe, and fix it if not.
                    message = self.receive() => {
                        self.handle_message(message?).await?;
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
                    _ = sleep(PERIODIC_TASKS_CHECK_INTERVAL) => {
                        // no progress on other tasks, yield control to check
                        // periodic tasks
                    }
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

    pub fn get_remote_node_id(&self) -> Result<H512, RLPxError> {
        if let RLPxConnectionState::Established(state) = &self.state {
            Ok(state.remote_node_id)
        } else {
            Err(RLPxError::InvalidState())
        }
    }

    async fn check_periodic_tasks(&mut self) -> Result<(), RLPxError> {
        if Instant::now() >= self.next_periodic_task_check {
            self.send(Message::Ping(PingMessage {})).await?;
            info!("Ping sent");
            self.next_periodic_task_check = Instant::now() + PERIODIC_TASKS_CHECK_INTERVAL;
        };
        Ok(())
    }

    async fn handle_message(&mut self, message: Message) -> Result<(), RLPxError> {
        let peer_supports_eth = self.capabilities.contains(&CAP_ETH);
        match message {
            Message::Disconnect(msg_data) => {
                info!("Received Disconnect: {:?}", msg_data.reason);
                // Returning a Disonnect error to be handled later at the call stack
                return Err(RLPxError::Disconnect());
            }
            Message::Ping(_) => {
                info!("Received Ping");
                self.send(Message::Pong(PongMessage {})).await?;
                info!("Pong sent");
            }
            Message::Pong(_) => {
                // We ignore received Pong messages
            }
            // Implmenent Status vaidations
            // https://github.com/lambdaclass/lambda_ethereum_rust/issues/420
            Message::Status(_) if !peer_supports_eth => {
                info!("Received Status");
                // TODO: Check peer's status message.
            }
            // TODO: implement handlers for each message type
            Message::GetAccountRange(req) => {
                let response = process_account_range_request(req, self.storage.clone())?;
                self.send(Message::AccountRange(response)).await?
            }
            // TODO(#1129) Add the transaction to the mempool once received.
            txs_msg @ Message::Transactions(_) if peer_supports_eth => {
                self.broadcast_message(txs_msg).await?;
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
            self.send(Message::Status(status)).await?;
            // The next immediate message in the ETH protocol is the
            // status, reference here:
            // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#status-0x00
            // let Ok(Message::Status(_)) = self.receive().await else {
            //     self.capabilities.iter_mut().position(|cap| cap == &CAP_ETH).map(|indx| self.capabilities.remove(indx));
            // }
            match self.receive().await? {
                Message::Status(_) => {
                    // TODO: Check message status is correct.
                }
                _msg => {
                    return Err(RLPxError::HandshakeError(
                        "Expected a Status message".to_string(),
                    ))
                }
            }
        }
        // TODO: add new capabilities startup when required (eg. snap)
        Ok(())
    }

    async fn send_auth(&mut self) -> Result<(), RLPxError> {
        if let RLPxConnectionState::Initiator(initiator_state) = &self.state {
            let secret_key: SecretKey = self.signer.clone().into();
            let peer_pk =
                id2pubkey(initiator_state.remote_node_id).ok_or(RLPxError::InvalidPeerId())?;

            // Clonning previous state to avoid ownership issues
            let previous_state = initiator_state.clone();

            let msg = encode_auth_message(
                &secret_key,
                previous_state.nonce,
                &peer_pk,
                &previous_state.ephemeral_key,
            )?;

            self.send_handshake_msg(&msg).await?;

            self.state =
                RLPxConnectionState::InitiatedAuth(InitiatedAuth::new(previous_state, msg));
            Ok(())
        } else {
            Err(RLPxError::InvalidState())
        }
    }

    async fn send_ack(&mut self) -> Result<(), RLPxError> {
        if let RLPxConnectionState::ReceivedAuth(received_auth_state) = &self.state {
            let peer_pk =
                id2pubkey(received_auth_state.remote_node_id).ok_or(RLPxError::InvalidPeerId())?;

            // Clonning previous state to avoid ownership issues
            let previous_state = received_auth_state.clone();

            let msg = encode_ack_message(
                &previous_state.local_ephemeral_key,
                previous_state.local_nonce,
                &peer_pk,
            )?;

            self.send_handshake_msg(&msg).await?;

            self.state = RLPxConnectionState::Established(Box::new(Established::for_receiver(
                previous_state,
                msg,
            )));
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

            // Build next state
            self.state = RLPxConnectionState::ReceivedAuth(ReceivedAuth::new(
                previous_state,
                auth.node_id,
                msg_bytes.to_owned(),
                auth.nonce,
                remote_ephemeral_key,
            ));
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
            // Build next state
            self.state = RLPxConnectionState::Established(Box::new(Established::for_initiator(
                previous_state,
                msg_bytes.to_owned(),
                ack.nonce,
                remote_ephemeral_key,
            )));
            Ok(())
        } else {
            Err(RLPxError::InvalidState())
        }
    }

    async fn send_handshake_msg(&mut self, msg: &[u8]) -> Result<(), RLPxError> {
        self.stream
            .write_all(msg)
            .await
            .map_err(|_| RLPxError::ConnectionError("Could not send message".to_string()))?;
        Ok(())
    }

    async fn receive_handshake_msg(&mut self) -> Result<Vec<u8>, RLPxError> {
        let mut buf = vec![0; MAX_DISC_PACKET_SIZE];

        // Read the message's size
        self.stream
            .read_exact(&mut buf[..2])
            .await
            .map_err(|_| RLPxError::ConnectionError("Connection dropped".to_string()))?;
        let ack_data = [buf[0], buf[1]];
        let msg_size = u16::from_be_bytes(ack_data) as usize;

        // Read the rest of the message
        self.stream
            .read_exact(&mut buf[2..msg_size + 2])
            .await
            .map_err(|_| RLPxError::ConnectionError("Connection dropped".to_string()))?;
        let ack_bytes = &buf[..msg_size + 2];
        Ok(ack_bytes.to_vec())
    }

    async fn send(&mut self, message: rlpx::Message) -> Result<(), RLPxError> {
        if let RLPxConnectionState::Established(state) = &mut self.state {
            let mut frame_buffer = vec![];
            message.encode(&mut frame_buffer)?;
            frame::write(frame_buffer, state, &mut self.stream).await?;
            Ok(())
        } else {
            Err(RLPxError::InvalidState())
        }
    }

    async fn receive(&mut self) -> Result<rlpx::Message, RLPxError> {
        if let RLPxConnectionState::Established(state) = &mut self.state {
            let frame_data = frame::read(state, &mut self.stream).await?;
            let (msg_id, msg_data): (u8, _) = RLPDecode::decode_unfinished(&frame_data)?;
            Ok(rlpx::Message::decode(msg_id, msg_data)?)
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
    Established(Box<Established>),
}

#[derive(Clone)]
struct Receiver {
    pub(crate) nonce: H256,
    pub(crate) ephemeral_key: SecretKey,
}

impl Receiver {
    pub fn new(nonce: H256, ephemeral_key: SecretKey) -> Self {
        Self {
            nonce,
            ephemeral_key,
        }
    }
}

#[derive(Clone)]
struct Initiator {
    pub(crate) nonce: H256,
    pub(crate) ephemeral_key: SecretKey,
    pub(crate) remote_node_id: H512,
}

impl Initiator {
    pub fn new(nonce: H256, ephemeral_key: SecretKey, remote_node_id: H512) -> Self {
        Self {
            nonce,
            ephemeral_key,
            remote_node_id,
        }
    }
}

#[derive(Clone)]
struct ReceivedAuth {
    pub(crate) local_nonce: H256,
    pub(crate) local_ephemeral_key: SecretKey,
    pub(crate) remote_node_id: H512,
    pub(crate) remote_nonce: H256,
    pub(crate) remote_ephemeral_key: PublicKey,
    pub(crate) remote_init_message: Vec<u8>,
}

impl ReceivedAuth {
    pub fn new(
        previous_state: Receiver,
        remote_node_id: H512,
        remote_init_message: Vec<u8>,
        remote_nonce: H256,
        remote_ephemeral_key: PublicKey,
    ) -> Self {
        Self {
            local_nonce: previous_state.nonce,
            local_ephemeral_key: previous_state.ephemeral_key,
            remote_node_id,
            remote_nonce,
            remote_ephemeral_key,
            remote_init_message,
        }
    }
}

#[derive(Clone)]
struct InitiatedAuth {
    pub(crate) remote_node_id: H512,
    pub(crate) local_nonce: H256,
    pub(crate) local_ephemeral_key: SecretKey,
    pub(crate) local_init_message: Vec<u8>,
}

impl InitiatedAuth {
    pub fn new(previous_state: Initiator, local_init_message: Vec<u8>) -> Self {
        Self {
            remote_node_id: previous_state.remote_node_id,
            local_nonce: previous_state.nonce,
            local_ephemeral_key: previous_state.ephemeral_key,
            local_init_message,
        }
    }
}

pub struct Established {
    pub remote_node_id: H512,
    pub(crate) mac_key: H256,
    pub ingress_mac: Keccak256,
    pub egress_mac: Keccak256,
    pub ingress_aes: Aes256Ctr64BE,
    pub egress_aes: Aes256Ctr64BE,
}

impl Established {
    fn for_receiver(previous_state: ReceivedAuth, init_message: Vec<u8>) -> Self {
        // keccak256(nonce || initiator-nonce)
        // Remote node is initator
        let hashed_nonces = Keccak256::digest(
            [previous_state.local_nonce.0, previous_state.remote_nonce.0].concat(),
        )
        .into();

        Self::new(
            previous_state.remote_node_id,
            init_message,
            previous_state.local_nonce,
            previous_state.local_ephemeral_key,
            hashed_nonces,
            previous_state.remote_init_message,
            previous_state.remote_nonce,
            previous_state.remote_ephemeral_key,
        )
    }

    fn for_initiator(
        previous_state: InitiatedAuth,
        remote_init_message: Vec<u8>,
        remote_nonce: H256,
        remote_ephemeral_key: PublicKey,
    ) -> Self {
        // keccak256(nonce || initiator-nonce)
        // Local node is initator
        let hashed_nonces =
            Keccak256::digest([remote_nonce.0, previous_state.local_nonce.0].concat()).into();

        Self::new(
            previous_state.remote_node_id,
            previous_state.local_init_message,
            previous_state.local_nonce,
            previous_state.local_ephemeral_key,
            hashed_nonces,
            remote_init_message,
            remote_nonce,
            remote_ephemeral_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        remote_node_id: H512,
        local_init_message: Vec<u8>,
        local_nonce: H256,
        local_ephemeral_key: SecretKey,
        hashed_nonces: [u8; 32],
        remote_init_message: Vec<u8>,
        remote_nonce: H256,
        remote_ephemeral_key: PublicKey,
    ) -> Self {
        let ephemeral_key_secret = ecdh_xchng(&local_ephemeral_key, &remote_ephemeral_key);

        // shared-secret = keccak256(ephemeral-key || keccak256(nonce || initiator-nonce))
        let shared_secret =
            Keccak256::digest([ephemeral_key_secret, hashed_nonces].concat()).into();
        // aes-secret = keccak256(ephemeral-key || shared-secret)
        let aes_key =
            H256(Keccak256::digest([ephemeral_key_secret, shared_secret].concat()).into());
        // mac-secret = keccak256(ephemeral-key || aes-secret)
        let mac_key = H256(Keccak256::digest([ephemeral_key_secret, aes_key.0].concat()).into());

        // egress-mac = keccak256.init((mac-secret ^ remote-nonce) || auth)
        let egress_mac = Keccak256::default()
            .chain_update(mac_key ^ remote_nonce)
            .chain_update(&local_init_message);

        // ingress-mac = keccak256.init((mac-secret ^ initiator-nonce) || ack)
        let ingress_mac = Keccak256::default()
            .chain_update(mac_key ^ local_nonce)
            .chain_update(&remote_init_message);

        let ingress_aes = <Aes256Ctr64BE as KeyIvInit>::new(&aes_key.0.into(), &[0; 16].into());
        let egress_aes = ingress_aes.clone();
        Self {
            remote_node_id,
            mac_key,
            ingress_mac,
            egress_mac,
            ingress_aes,
            egress_aes,
        }
    }
}
