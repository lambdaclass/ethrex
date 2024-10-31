use crate::{
    rlpx::{eth::backend, handshake::encode_ack_message, message::Message, p2p, utils::id2pubkey},
    snap::process_account_range_request,
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
use bytes::BufMut as _;
use ethereum_rust_core::{H256, H512};
use ethereum_rust_rlp::decode::RLPDecode;
use ethereum_rust_storage::Store;
use k256::{
    ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey},
    PublicKey, SecretKey,
};
use sha3::{Digest, Keccak256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{error, info};
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

    pub async fn initiator(signer: SigningKey, msg: &[u8], stream: S, storage: Store) -> Self {
        let mut rng = rand::thread_rng();
        let digest = Keccak256::digest(&msg[65..]);
        let signature = &Signature::from_bytes(msg[..64].into()).unwrap();
        let rid = RecoveryId::from_byte(msg[64]).unwrap();
        let peer_pk = VerifyingKey::recover_from_prehash(&digest, signature, rid).unwrap();
        let state = RLPxConnectionState::Initiator(Initiator::new(
            H256::random_using(&mut rng),
            SecretKey::random(&mut rng),
            pubkey2id(&peer_pk.into()),
        ));
        RLPxConnection::new(signer, stream, state, storage)
    }

    pub async fn handshake(&mut self) -> Result<(), RLPxError> {
        match &self.state {
            RLPxConnectionState::Initiator(_) => {
                self.send_auth().await;
                self.receive_ack().await;
            }
            RLPxConnectionState::Receiver(_) => {
                self.receive_auth().await;
                self.send_ack().await;
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

        self.send(hello_msg).await;

        // Receive Hello message
        match self.receive().await {
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
                    match self.receive().await {
                        // TODO: implement handlers for each message type
                        Message::Disconnect(_) => info!("Received Disconnect"),
                        Message::Ping(_) => info!("Received Ping"),
                        Message::Pong(_) => info!("Received Pong"),
                        Message::Status(_) => info!("Received Status"),
                        Message::GetAccountRange(req) => {
                            let response =
                                process_account_range_request(req, self.storage.clone())?;
                            self.send(Message::AccountRange(response)).await
                        }
                        // TODO: Add new message types and handlers as they are implemented
                        message => return Err(RLPxError::UnexpectedMessage(message)),
                    };
                }
            }
            _ => Err(RLPxError::InvalidState(
                "Invalid connection state".to_string(),
            )),
        }
    }

    pub fn get_remote_node_id(&self) -> H512 {
        match &self.state {
            RLPxConnectionState::Established(state) => state.remote_node_id,
            // TODO proper error
            _ => panic!("Invalid state"),
        }
    }

    async fn start_capabilities(&mut self) -> Result<(), RLPxError> {
        // Sending eth Status if peer supports it
        if self.capabilities.contains(&CAP_ETH) {
            let status = backend::get_status(&self.storage).unwrap();
            self.send(Message::Status(status)).await;
        }
        // TODO: add new capabilities startup when required (eg. snap)
        Ok(())
    }

    async fn send_auth(&mut self) {
        match &self.state {
            RLPxConnectionState::Initiator(initiator_state) => {
                let secret_key: SecretKey = self.signer.clone().into();
                let peer_pk = id2pubkey(initiator_state.remote_node_id).unwrap();

                let mut auth_message = vec![];
                let msg = encode_auth_message(
                    &secret_key,
                    initiator_state.nonce,
                    &peer_pk,
                    &initiator_state.ephemeral_key,
                );

                auth_message.put_slice(&msg);
                self.stream.write_all(&auth_message).await.unwrap();

                self.state = RLPxConnectionState::InitiatedAuth(InitiatedAuth::new(
                    initiator_state,
                    auth_message,
                ))
            }
            // TODO proper error
            _ => panic!("Invalid state to send auth message"),
        };
    }

    async fn send_ack(&mut self) {
        match &self.state {
            RLPxConnectionState::ReceivedAuth(received_auth_state) => {
                let peer_pk = id2pubkey(received_auth_state.remote_node_id).unwrap();

                let mut ack_message = vec![];
                let msg = encode_ack_message(
                    &received_auth_state.local_ephemeral_key,
                    received_auth_state.local_nonce,
                    &peer_pk,
                );

                ack_message.put_slice(&msg);
                self.stream.write_all(&ack_message).await.unwrap();

                self.state = RLPxConnectionState::Established(Box::new(Established::for_receiver(
                    received_auth_state,
                    ack_message,
                )))
            }
            // TODO proper error
            _ => panic!("Invalid state to send ack message"),
        };
    }

    async fn receive_auth(&mut self) {
        match &self.state {
            RLPxConnectionState::Receiver(receiver_state) => {
                let secret_key: SecretKey = self.signer.clone().into();
                let mut buf = vec![0; MAX_DISC_PACKET_SIZE];

                // Read the auth message's size
                self.stream.read_exact(&mut buf[..2]).await.unwrap();
                let auth_data = buf[..2].try_into().unwrap();
                let msg_size = u16::from_be_bytes(auth_data) as usize;

                // Read the rest of the auth message
                self.stream
                    .read_exact(&mut buf[2..msg_size + 2])
                    .await
                    .unwrap();
                let auth_bytes = &buf[..msg_size + 2];
                let msg = &buf[2..msg_size + 2];
                let (auth, remote_ephemeral_key) = decode_auth_message(&secret_key, msg, auth_data);

                // Build next state
                self.state = RLPxConnectionState::ReceivedAuth(ReceivedAuth::new(
                    receiver_state,
                    auth.node_id,
                    auth_bytes.to_owned(),
                    auth.nonce,
                    remote_ephemeral_key,
                ))
            }
            // TODO proper error
            _ => panic!("Received an unexpected auth message"),
        };
    }

    async fn receive_ack(&mut self) {
        match &self.state {
            RLPxConnectionState::InitiatedAuth(initiated_auth_state) => {
                let secret_key: SecretKey = self.signer.clone().into();
                let mut buf = vec![0; MAX_DISC_PACKET_SIZE];

                // Read the ack message's size
                self.stream.read_exact(&mut buf[..2]).await.unwrap();
                let ack_data = buf[..2].try_into().unwrap();
                let msg_size = u16::from_be_bytes(ack_data) as usize;

                // Read the rest of the ack message
                self.stream
                    .read_exact(&mut buf[2..msg_size + 2])
                    .await
                    .unwrap();
                let ack_bytes = &buf[..msg_size + 2];
                let msg = &buf[2..msg_size + 2];
                let ack = decode_ack_message(&secret_key, msg, ack_data);
                let remote_ephemeral_key = ack.get_ephemeral_pubkey().unwrap();

                // Build next state
                self.state = RLPxConnectionState::Established(Box::new(Established::for_initiator(
                    initiated_auth_state,
                    ack_bytes.to_owned(),
                    ack.nonce,
                    remote_ephemeral_key,
                )))
            }
            // TODO proper error
            _ => panic!("Received an unexpected ack message"),
        };
    }

    async fn send(&mut self, message: rlpx::Message) {
        match &mut self.state {
            RLPxConnectionState::Established(state) => {
                let mut frame_buffer = vec![];
                match message.encode(&mut frame_buffer) {
                    Ok(_) => {}
                    Err(e) => {
                        // TODO: better error handling
                        error!("Failed to encode message: {:?}", e);
                    }
                };
                frame::write(frame_buffer, state, &mut self.stream).await;
            }
            // TODO proper error
            _ => panic!("Invalid state to send message"),
        }
    }

    async fn receive(&mut self) -> rlpx::Message {
        match &mut self.state {
            RLPxConnectionState::Established(state) => {
                let frame_data = frame::read(state, &mut self.stream).await;
                let (msg_id, msg_data): (u8, _) =
                    RLPDecode::decode_unfinished(&frame_data).unwrap();
                rlpx::Message::decode(msg_id, msg_data).unwrap()
            }
            // TODO proper error
            _ => panic!("Received an unexpected message"),
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
        previous_state: &Receiver,
        remote_node_id: H512,
        remote_init_message: Vec<u8>,
        remote_nonce: H256,
        remote_ephemeral_key: PublicKey,
    ) -> Self {
        Self {
            local_nonce: previous_state.nonce,
            local_ephemeral_key: previous_state.ephemeral_key.clone(),
            remote_node_id,
            remote_nonce,
            remote_ephemeral_key,
            remote_init_message,
        }
    }
}

struct InitiatedAuth {
    pub(crate) remote_node_id: H512,
    pub(crate) local_nonce: H256,
    pub(crate) local_ephemeral_key: SecretKey,
    pub(crate) local_init_message: Vec<u8>,
}

impl InitiatedAuth {
    pub fn new(previous_state: &Initiator, local_init_message: Vec<u8>) -> Self {
        Self {
            remote_node_id: previous_state.remote_node_id,
            local_nonce: previous_state.nonce,
            local_ephemeral_key: previous_state.ephemeral_key.clone(),
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
    fn for_receiver(previous_state: &ReceivedAuth, init_message: Vec<u8>) -> Self {
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
            previous_state.local_ephemeral_key.clone(),
            hashed_nonces,
            previous_state.remote_init_message.clone(),
            previous_state.remote_nonce,
            previous_state.remote_ephemeral_key,
        )
    }

    fn for_initiator(
        previous_state: &InitiatedAuth,
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
            previous_state.local_init_message.clone(),
            previous_state.local_nonce,
            previous_state.local_ephemeral_key.clone(),
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

// TODO fix this test now that RLPxClient does no longer exist
// https://github.com/lambdaclass/lambda_ethereum_rust/issues/843
#[cfg(test)]
mod tests {
    // use hex_literal::hex;
    // use k256::SecretKey;

    #[test]
    fn test_ack_decoding() {
        // // This is the Ack₂ message from EIP-8.
        // let msg = hex!("01ea0451958701280a56482929d3b0757da8f7fbe5286784beead59d95089c217c9b917788989470b0e330cc6e4fb383c0340ed85fab836ec9fb8a49672712aeabbdfd1e837c1ff4cace34311cd7f4de05d59279e3524ab26ef753a0095637ac88f2b499b9914b5f64e143eae548a1066e14cd2f4bd7f814c4652f11b254f8a2d0191e2f5546fae6055694aed14d906df79ad3b407d94692694e259191cde171ad542fc588fa2b7333313d82a9f887332f1dfc36cea03f831cb9a23fea05b33deb999e85489e645f6aab1872475d488d7bd6c7c120caf28dbfc5d6833888155ed69d34dbdc39c1f299be1057810f34fbe754d021bfca14dc989753d61c413d261934e1a9c67ee060a25eefb54e81a4d14baff922180c395d3f998d70f46f6b58306f969627ae364497e73fc27f6d17ae45a413d322cb8814276be6ddd13b885b201b943213656cde498fa0e9ddc8e0b8f8a53824fbd82254f3e2c17e8eaea009c38b4aa0a3f306e8797db43c25d68e86f262e564086f59a2fc60511c42abfb3057c247a8a8fe4fb3ccbadde17514b7ac8000cdb6a912778426260c47f38919a91f25f4b5ffb455d6aaaf150f7e5529c100ce62d6d92826a71778d809bdf60232ae21ce8a437eca8223f45ac37f6487452ce626f549b3b5fdee26afd2072e4bc75833c2464c805246155289f4");

        // let static_key = hex!("49a7b37aa6f6645917e7b807e9d1c00d4fa71f18343b0d4122a4d2df64dd6fee");
        // let nonce = hex!("7e968bba13b6c50e2c4cd7f241cc0d64d1ac25c7f5952df231ac6a2bda8ee5d6");
        // let ephemeral_key =
        //     hex!("869d6ecf5211f1cc60418a13b9d870b22959d0c16f02bec714c960dd2298a32d");

        // let mut client = RLPxClient::new(
        //     true,
        //     nonce.into(),
        //     SecretKey::from_slice(&ephemeral_key).unwrap(),
        // );

        // assert_eq!(
        //     &client.local_ephemeral_key.to_bytes()[..],
        //     &ephemeral_key[..]
        // );
        // assert_eq!(client.local_nonce.0, nonce);

        // let auth_data = msg[..2].try_into().unwrap();

        // client.local_init_message = Some(vec![]);

        // let state = client.decode_ack_message(
        //     &SecretKey::from_slice(&static_key).unwrap(),
        //     &msg[2..],
        //     auth_data,
        // );

        // let expected_mac_secret =
        //     hex!("2ea74ec5dae199227dff1af715362700e989d889d7a493cb0639691efb8e5f98");

        // assert_eq!(state.mac_key.0, expected_mac_secret);
    }
}
