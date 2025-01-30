use crate::types::NodeId;
use ethrex_core::{H256, H512, H520};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use tokio::sync::mpsc;

pub const MAX_UDP_PAYLOAD_SIZE: usize = 1280;
pub const DEFAULT_UDP_PAYLOAD_BUF: [u8; MAX_UDP_PAYLOAD_SIZE] = [0u8; MAX_UDP_PAYLOAD_SIZE];
pub const HASH_LENGTH_IN_BYTES: usize = 32;
pub const HEADER_LENGTH_IN_BYTES: usize = HASH_LENGTH_IN_BYTES + 65;
pub const PACKET_TYPE_LENGTH_IN_BYTES: usize = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to send message: {0:?}, reason: {1}")]
    FailedToSend(Message, String),
    #[error("RLP decode error: {0}")]
    FailedToRLPDecode(#[from] RLPDecodeError),
}

#[derive(Debug, Clone)]
pub enum Message {}

impl From<Packet> for Message {
    fn from(packet: Packet) -> Self {
        match packet.data {
            PacketData::Auth { .. } => todo!(),
            PacketData::AuthAck { .. } => todo!(),
        }
    }
}

#[derive(Clone)]
pub struct Mailbox {
    sender: mpsc::Sender<Message>,
}

impl Mailbox {
    pub fn new(sender: mpsc::Sender<Message>) -> Self {
        Self { sender }
    }

    async fn send(&self, message: Message) -> Result<(), Error> {
        self.sender
            .send(message.clone())
            .await
            .map_err(|e| Error::FailedToSend(message, e.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct Packet {
    pub data: PacketData,
    pub node_id: NodeId,
}

impl std::fmt::Display for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Packet {{ data: {:?}, node_id: {} }}",
            self.data,
            hex::encode(&self.node_id.serialize()[1..])
        )
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Auth {
    pub signature: H520,
    pub initiator_pubkey: H512,
    pub nonce: H256,
    pub version: u8,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct AuthAck {
    pub recipient_ephemeral_pubk: H512,
    pub recipient_nonce: H256,
    pub ack_vsn: u8,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PacketData {
    Auth(Auth),
    AuthAck(AuthAck),
}

impl PacketData {
    pub fn encode(&self) -> Vec<u8> {
        self.encode_to_vec()
    }

    #[allow(clippy::result_large_err)]
    pub fn decode(rlp: &[u8]) -> Result<Self, Error> {
        <PacketData as RLPDecode>::decode(rlp).map_err(Error::from)
    }

    pub fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        <PacketData as RLPDecode>::decode_unfinished(rlp)
    }

    pub fn r#type(&self) -> u8 {
        match self {
            PacketData::Auth { .. } => todo!(),
            PacketData::AuthAck { .. } => todo!(),
        }
    }
}

impl RLPEncode for PacketData {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            PacketData::Auth(Auth {
                signature,
                initiator_pubkey,
                nonce,
                version,
            }) => Encoder::new(buf)
                .encode_field(signature)
                .encode_field(initiator_pubkey)
                .encode_field(nonce)
                .encode_field(version)
                .finish(),
            PacketData::AuthAck(AuthAck {
                recipient_ephemeral_pubk,
                recipient_nonce,
                ack_vsn,
            }) => Encoder::new(buf)
                .encode_field(recipient_ephemeral_pubk)
                .encode_field(recipient_nonce)
                .encode_field(ack_vsn)
                .finish(),
        }
    }
}

impl RLPDecode for PacketData {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        try_rlp_decode_auth(rlp).or(try_rlp_decode_auth_ack(rlp))
    }
}

fn try_rlp_decode_auth(rlp: &[u8]) -> Result<(PacketData, &[u8]), RLPDecodeError> {
    let decoder = Decoder::new(rlp)?;
    let (signature, decoder) = decoder.decode_field("signature")?;
    let (initiator_pubkey, decoder) = decoder.decode_field("initiator_pubkey")?;
    let (nonce, decoder) = decoder.decode_field("nonce")?;
    let (version, decoder) = decoder.decode_field("version")?;
    Ok((
        PacketData::Auth(Auth {
            signature,
            initiator_pubkey,
            nonce,
            version,
        }),
        decoder.finish_unchecked(),
    ))
}

fn try_rlp_decode_auth_ack(rlp: &[u8]) -> Result<(PacketData, &[u8]), RLPDecodeError> {
    let decoder = Decoder::new(rlp)?;
    let (recipient_ephemeral_pubk, decoder) = decoder.decode_field("recipient_ephemeral_pubk")?;
    let (recipient_nonce, decoder) = decoder.decode_field("recipient_nonce")?;
    let (ack_vsn, decoder) = decoder.decode_field("ack_vsn")?;
    Ok((
        PacketData::AuthAck(AuthAck {
            recipient_ephemeral_pubk,
            recipient_nonce,
            ack_vsn,
        }),
        decoder.finish_unchecked(),
    ))
}
