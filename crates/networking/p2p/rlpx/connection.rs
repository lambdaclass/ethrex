use crate::{
    rlpx::{
        eth::{
            backend,
            blocks::{BlockBodies, BlockHeaders},
        },
        handshake::encode_ack_message,
        message::Message,
        p2p,
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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::info;
const CAP_P2P: (Capability, u8) = (Capability::P2p, 5);
const CAP_ETH: (Capability, u8) = (Capability::Eth, 68);
const CAP_SNAP: (Capability, u8) = (Capability::Snap, 1);
const SUPPORTED_CAPABILITIES: [(Capability, u8); 3] = [CAP_P2P, CAP_ETH, CAP_SNAP];

pub(crate) type Aes256Ctr64BE = ctr::Ctr64BE<aes::Aes256>;

/// Fully working RLPx connection.
pub(crate) struct RLPxConnection<S> {
    signer: SigningKey,
    state: RLPxConnectionState,
    stream: S,
    storage: Store,
    capabilities: Vec<(Capability, u8)>,
}

impl<S: AsyncWrite + AsyncRead + std::marker::Unpin> RLPxConnection<S> {
    fn new(signer: SigningKey, stream: S, state: RLPxConnectionState, storage: Store) -> Self {
        Self {
            signer,
            state,
            stream,
            storage,
            capabilities: vec![],
        }
    }

    pub fn receiver(signer: SigningKey, stream: S, storage: Store) -> Self {
        let mut rng = rand::thread_rng();
        Self::new(
            signer,
            stream,
            RLPxConnectionState::Receiver(Receiver::new(
                H256::random_using(&mut rng),
                SecretKey::random(&mut rng),
            )),
            storage,
        )
    }

    pub async fn initiator(
        signer: SigningKey,
        msg: &[u8],
        stream: S,
        storage: Store,
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
        Ok(RLPxConnection::new(signer, stream, state, storage))
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
            _ => {
                // Fail if it is not a hello message
                Err(RLPxError::HandshakeError(
                    "Expected Hello message".to_string(),
                ))
            }
        }
    }

    pub async fn handle_peer(&mut self) -> Result<(), RLPxError> {
        self.start_capabilities().await?;
        match &self.state {
            RLPxConnectionState::Established(_) => {
                info!("Started peer main loop");
                loop {
                    match self.receive().await? {
                        // TODO: implement handlers for each message type
                        // https://github.com/lambdaclass/lambda_ethereum_rust/issues/1030
                        Message::Disconnect(_) => info!("Received Disconnect"),
                        Message::Ping(_) => info!("Received Ping"),
                        Message::Pong(_) => info!("Received Pong"),
                        Message::Status(_) => info!("Received Status"),
                        Message::GetAccountRange(req) => {
                            let response =
                                process_account_range_request(req, self.storage.clone())?;
                            self.send(Message::AccountRange(response)).await?
                        }
                        Message::GetBlockHeaders(msg_data) => {
                            let response = BlockHeaders {
                                id: msg_data.id,
                                block_headers: msg_data.fetch_headers(&self.storage),
                            };
                            self.send(Message::BlockHeaders(response)).await?
                        }
                        Message::GetBlockBodies(msg_data) => {
                            let response = BlockBodies {
                                id: msg_data.id,
                                block_bodies: msg_data.fetch_blocks(&self.storage),
                            };
                            self.send(Message::BlockBodies(response)).await?
                        }
                        Message::GetStorageRanges(req) => {
                            let response =
                                process_storage_ranges_request(req, self.storage.clone())?;
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
                        _ => return Err(RLPxError::MessageNotHandled()),
                    };
                }
            }
            _ => Err(RLPxError::InvalidState()),
        }
    }

    pub fn get_remote_node_id(&self) -> Result<H512, RLPxError> {
        match &self.state {
            RLPxConnectionState::Established(state) => Ok(state.remote_node_id),
            _ => Err(RLPxError::InvalidState()),
        }
    }

    async fn start_capabilities(&mut self) -> Result<(), RLPxError> {
        // Sending eth Status if peer supports it
        if self.capabilities.contains(&CAP_ETH) {
            let status = backend::get_status(&self.storage)?;
            self.send(Message::Status(status)).await?;
        }
        // TODO: add new capabilities startup when required (eg. snap)
        Ok(())
    }

    async fn send_auth(&mut self) -> Result<(), RLPxError> {
        match &self.state {
            RLPxConnectionState::Initiator(initiator_state) => {
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
            }
            _ => Err(RLPxError::InvalidState()),
        }
    }

    async fn send_ack(&mut self) -> Result<(), RLPxError> {
        match &self.state {
            RLPxConnectionState::ReceivedAuth(received_auth_state) => {
                let peer_pk = id2pubkey(received_auth_state.remote_node_id)
                    .ok_or(RLPxError::InvalidPeerId())?;

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
            }
            _ => Err(RLPxError::InvalidState()),
        }
    }

    async fn receive_auth(&mut self) -> Result<(), RLPxError> {
        match &self.state {
            RLPxConnectionState::Receiver(receiver_state) => {
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
                let (auth, remote_ephemeral_key) =
                    decode_auth_message(&secret_key, msg, size_data)?;

                // Build next state
                self.state = RLPxConnectionState::ReceivedAuth(ReceivedAuth::new(
                    previous_state,
                    auth.node_id,
                    msg_bytes.to_owned(),
                    auth.nonce,
                    remote_ephemeral_key,
                ));
                Ok(())
            }
            _ => Err(RLPxError::InvalidState()),
        }
    }

    async fn receive_ack(&mut self) -> Result<(), RLPxError> {
        match &self.state {
            RLPxConnectionState::InitiatedAuth(initiated_auth_state) => {
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
                self.state =
                    RLPxConnectionState::Established(Box::new(Established::for_initiator(
                        previous_state,
                        msg_bytes.to_owned(),
                        ack.nonce,
                        remote_ephemeral_key,
                    )));
                Ok(())
            }
            _ => Err(RLPxError::InvalidState()),
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
        match &mut self.state {
            RLPxConnectionState::Established(state) => {
                let mut frame_buffer = vec![];
                message.encode(&mut frame_buffer)?;
                frame::write(frame_buffer, state, &mut self.stream).await?;
                Ok(())
            }
            _ => Err(RLPxError::InvalidState()),
        }
    }

    async fn receive(&mut self) -> Result<rlpx::Message, RLPxError> {
        match &mut self.state {
            RLPxConnectionState::Established(state) => {
                let frame_data = frame::read(state, &mut self.stream).await?;
                let (msg_id, msg_data): (u8, _) = RLPDecode::decode_unfinished(&frame_data)?;
                Ok(rlpx::Message::decode(msg_id, msg_data)?)
            }
            _ => Err(RLPxError::InvalidState()),
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
