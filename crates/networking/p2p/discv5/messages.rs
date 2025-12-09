use crate::{
    types::{Endpoint, Node, NodeRecord},
    utils::{current_unix_time, node_id},
};
use bytes::BufMut;
use ethrex_common::{H256, H512, H520, utils::keccak};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{self, Decoder, Encoder},
};
use secp256k1::{
    SecretKey,
    ecdsa::{RecoverableSignature, RecoveryId},
};
use std::{convert::Into, io::ErrorKind};

#[derive(Debug, thiserror::Error)]
pub enum PacketDecodeErr {
    #[error("RLP decoding error")]
    RLPDecodeError(#[from] RLPDecodeError),
    #[error("Invalid packet size")]
    InvalidSize,
    #[error("Hash mismatch")]
    HashMismatch,
    #[error("Invalid signature")]
    InvalidSignature,
    #[error("Discv4 decoding error: {0}")]
    Discv4DecodingError(String),
    #[error("Io Error: {0}")]
    IoError(#[from] std::io::Error),
}

impl From<PacketDecodeErr> for std::io::Error {
    fn from(error: PacketDecodeErr) -> Self {
        std::io::Error::new(ErrorKind::InvalidData, error.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Packet {
    hash: H256,
    signature: H520,
    message: Message,
    public_key: H512,
}

impl Packet {
    pub fn decode(encoded_packet: &[u8]) -> Result<Packet, PacketDecodeErr> {
        // the packet structure is
        // hash || signature || packet-type || packet-data
        let hash_len = 32;
        let signature_len = 65;
        let header_size = hash_len + signature_len; // 97

        if encoded_packet.len() < header_size + 1 {
            return Err(PacketDecodeErr::InvalidSize);
        };

        let hash = H256::from_slice(&encoded_packet[..hash_len]);
        let signature_bytes = &encoded_packet[hash_len..header_size];
        let packet_type = encoded_packet[header_size];
        let encoded_msg = &encoded_packet[header_size..];

        let header_hash = keccak(&encoded_packet[hash_len..]);

        if hash != header_hash {
            return Err(PacketDecodeErr::HashMismatch);
        }

        let digest: [u8; 32] = keccak_hash(encoded_msg);

        let rid = RecoveryId::try_from(Into::<i32>::into(signature_bytes[64]))
            .map_err(|_| PacketDecodeErr::InvalidSignature)?;

        let peer_pk = secp256k1::SECP256K1
            .recover_ecdsa(
                &secp256k1::Message::from_digest(digest),
                &RecoverableSignature::from_compact(&signature_bytes[0..64], rid)
                    .map_err(|_| PacketDecodeErr::InvalidSignature)?,
            )
            .map_err(|_| PacketDecodeErr::InvalidSignature)?;

        let encoded = peer_pk.serialize_uncompressed();

        let public_key = H512::from_slice(&encoded[1..]);
        let signature = H520::from_slice(signature_bytes);
        let message = Message::decode_with_type(packet_type, &encoded_msg[1..])
            .map_err(PacketDecodeErr::RLPDecodeError)?;

        Ok(Self {
            hash,
            signature,
            message,
            public_key,
        })
    }

    pub fn get_hash(&self) -> H256 {
        self.hash
    }

    pub fn get_message(&self) -> &Message {
        &self.message
    }

    #[allow(unused)]
    pub fn get_signature(&self) -> H520 {
        self.signature
    }

    pub fn get_public_key(&self) -> H512 {
        self.public_key
    }

    pub fn get_node_id(&self) -> H256 {
        node_id(&self.public_key)
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message {
    Ping(PingMessage),
}

impl Message {
    pub fn encode_with_header(&self, buf: &mut dyn BufMut, node_signer: &SecretKey) {
        let signature_size = 65_usize;
        let mut data: Vec<u8> = Vec::with_capacity(signature_size.next_power_of_two());
        data.resize(signature_size, 0);

        self.encode_with_type(&mut data);

        let digest: [u8; 32] = keccak_hash(&data[signature_size..]);

        let (recovery_id, signature) = secp256k1::SECP256K1
            .sign_ecdsa_recoverable(&secp256k1::Message::from_digest(digest), node_signer)
            .serialize_compact();

        data[..signature_size - 1].copy_from_slice(&signature);
        data[signature_size - 1] = Into::<i32>::into(recovery_id) as u8;

        let hash = keccak_hash(&data[..]);
        buf.put_slice(&hash);
        buf.put_slice(&data[..]);
    }

    fn encode_with_type(&self, buf: &mut dyn BufMut) {
        buf.put_u8(self.packet_type());
        match self {
            Message::Ping(msg) => msg.encode(buf),
        }
    }

    pub fn decode_with_type(packet_type: u8, msg: &[u8]) -> Result<Message, RLPDecodeError> {
        // NOTE: extra elements inside the message should be ignored, along with extra data
        // after the message.
        match packet_type {
            0x01 => {
                let (ping, _rest) = PingMessage::decode_unfinished(msg)?;
                Ok(Message::Ping(ping))
            }
            _ => Err(RLPDecodeError::MalformedData),
        }
    }

    fn packet_type(&self) -> u8 {
        match self {
            Message::Ping(_) => 0x01,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingMessage {
    /// The Ping message version. Should be set to 4, but mustn't be enforced.
    pub version: u8,
    /// The endpoint of the sender.
    pub from: Endpoint,
    /// The endpoint of the receiver.
    pub to: Endpoint,
    /// The expiration time of the message. If the message is older than this time,
    /// it shouldn't be responded to.
    pub expiration: u64,
    /// The ENR sequence number of the sender. This field is optional.
    pub enr_seq: Option<u64>,
}

impl PingMessage {
    pub fn new(from: Endpoint, to: Endpoint, expiration: u64) -> Self {
        Self {
            version: 4,
            from,
            to,
            expiration,
            enr_seq: None,
        }
    }

    // TODO: remove when used
    #[allow(unused)]
    pub fn with_enr_seq(self, enr_seq: u64) -> Self {
        Self {
            enr_seq: Some(enr_seq),
            ..self
        }
    }
}

impl RLPEncode for PingMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.version)
            .encode_field(&self.from)
            .encode_field(&self.to)
            .encode_field(&self.expiration)
            .encode_optional_field(&self.enr_seq)
            .finish();
    }
}

impl RLPDecode for PingMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (version, decoder): (u8, Decoder) = decoder.decode_field("version")?;
        let (from, decoder) = decoder.decode_field("from")?;
        let (to, decoder) = decoder.decode_field("to")?;
        let (expiration, decoder) = decoder.decode_field("expiration")?;
        let (enr_seq, decoder) = decoder.decode_optional_field();

        let ping = PingMessage {
            version,
            from,
            to,
            expiration,
            enr_seq,
        };
        // NOTE: as per the spec, any additional elements should be ignored.
        let remaining = decoder.finish_unchecked();
        Ok((ping, remaining))
    }
}