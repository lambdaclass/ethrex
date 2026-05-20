use crate::{
    discv4::messages::{FindNodeMessage, Message, Packet},
    utils::{get_msg_expiration_from_seconds, node_id, public_key_from_signing_key},
};
use bytes::BytesMut;
use ethrex_common::{H256, H512};
use rand::rngs::OsRng;
use secp256k1::SecretKey;
use std::{collections::HashMap, net::SocketAddr, time::Instant};

pub const EXPIRATION_SECONDS: u64 = 20;

/// Discv4-specific state held within the unified DiscoveryServer.
#[derive(Debug)]
pub struct Discv4State {
    /// The last `FindNode` message sent, cached due to message
    /// signatures being expensive.
    pub find_node_message: BytesMut,
    /// Tracks pending FindNode requests by node_id -> sent_at.
    /// Used to reject unsolicited Neighbors responses.
    pub pending_find_node: HashMap<H256, Instant>,
}

impl Discv4State {
    pub fn new(signer: &SecretKey) -> Self {
        Self {
            find_node_message: Self::random_message(signer),
            pending_find_node: HashMap::new(),
        }
    }

    /// Generate a FindNodeMessage with a random key.
    /// We send the same message on discovery lookup.
    /// Changed every CHANGE_FIND_NODE_MESSAGE_INTERVAL.
    pub fn random_message(signer: &SecretKey) -> BytesMut {
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);
        let random_priv_key = SecretKey::new(&mut OsRng);
        let random_pub_key = public_key_from_signing_key(&random_priv_key);
        let msg = Message::FindNode(FindNodeMessage::new(random_pub_key, expiration));
        let mut buf = BytesMut::new();
        msg.encode_with_header(&mut buf, signer);
        buf
    }
}

#[derive(Debug, Clone)]
pub struct Discv4Message {
    pub(crate) from: SocketAddr,
    pub(crate) message: Message,
    pub(crate) hash: H256,
    pub(crate) sender_public_key: H512,
}

impl Discv4Message {
    pub fn from(packet: Packet, from: SocketAddr) -> Self {
        Self {
            from,
            message: packet.get_message().clone(),
            hash: packet.get_hash(),
            sender_public_key: packet.get_public_key(),
        }
    }

    pub fn get_node_id(&self) -> H256 {
        node_id(&self.sender_public_key)
    }
}
