use aes::cipher::{KeyIvInit, StreamCipher, StreamCipherError};
use aes_gcm::{Aes128Gcm, KeyInit, aead::AeadMutInPlace};
use bytes::{BufMut, Bytes};
use ethrex_common::H256;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use std::{array::TryFromSliceError, fmt::Display, net::SocketAddr};

use crate::types::NodeRecord;

type Aes128Ctr64BE = ctr::Ctr64BE<aes::Aes128>;

// Max and min packet sizes as defined in
// https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire.md#udp-communication
// Used for package validation
const MIN_PACKET_SIZE: usize = 63;
const MAX_PACKET_SIZE: usize = 1280;
/// 32 src-id + 1 sig-size + 1 eph-key-size
const HANDSHAKE_AUTHDATA_HEAD: usize = 34;
// protocol data
const PROTOCOL_ID: &[u8] = b"discv5";
const PROTOCOL_VERSION: u16 = 0x0001;
// masking-iv size for a u128
const IV_MASKING_SIZE: usize = 16;
// static_header size is 23 bytes
const STATIC_HEADER_SIZE: usize = 23;
const STATIC_HEADER_END: usize = IV_MASKING_SIZE + STATIC_HEADER_SIZE;
// Number of distances to include in a FindNode message
pub const DISTANCES_PER_FIND_NODE_MSG: u8 = 3;

#[derive(Debug, thiserror::Error)]
pub enum PacketCodecError {
    #[error("RLP decoding error")]
    RLPDecodeError(#[from] RLPDecodeError),
    #[error("Packet header decoding error")]
    InvalidHeader,
    #[error("Message decoding error, message type: {0}")]
    InvalidMessage(u8),
    #[error("Invalid packet size")]
    InvalidSize,
    #[error("Session not established yet")]
    SessionNotEstablished,
    #[error("Invalid protocol: {0}")]
    InvalidProtocol(String),
    #[error("Stream Cipher Error: {0}")]
    CipherError(String),
    #[error("TryFromSliceError: {0}")]
    TryFromSliceError(#[from] TryFromSliceError),
    #[error("Io Error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Malformed Data")]
    MalformedData,
}

impl From<StreamCipherError> for PacketCodecError {
    fn from(error: StreamCipherError) -> Self {
        PacketCodecError::CipherError(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub(crate) masking_iv: [u8; IV_MASKING_SIZE],
    pub(crate) header: PacketHeader,
    pub(crate) encrypted_message: Vec<u8>,
}

impl Packet {
    pub fn decode(dest_id: &H256, encoded_packet: &[u8]) -> Result<Packet, PacketCodecError> {
        if encoded_packet.len() < MIN_PACKET_SIZE || encoded_packet.len() > MAX_PACKET_SIZE {
            return Err(PacketCodecError::InvalidSize);
        }

        // the packet structure is
        // masking-iv || masked-header || message
        // 16 bytes for an u128
        let masking_iv = &encoded_packet[..IV_MASKING_SIZE];

        let mut cipher = <Aes128Ctr64BE as KeyIvInit>::new(dest_id[..16].into(), masking_iv.into());

        let header = PacketHeader::decode(&mut cipher, encoded_packet)
            .map_err(|_e| PacketCodecError::InvalidHeader)?;
        let encrypted_message = encoded_packet[header.header_end_offset..].to_vec();
        Ok(Packet {
            masking_iv: masking_iv.try_into()?,
            header,
            encrypted_message,
        })
    }

    pub fn encode(&self, buf: &mut dyn BufMut, dest_id: &H256) -> Result<(), PacketCodecError> {
        let masking_iv = self.masking_iv;
        buf.put_slice(&masking_iv);

        let mut cipher =
            <Aes128Ctr64BE as KeyIvInit>::new(dest_id[..16].into(), masking_iv[..].into());

        self.header.encode(buf, &mut cipher)?;
        buf.put_slice(&self.encrypted_message);

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketHeader {
    pub static_header: [u8; STATIC_HEADER_SIZE],
    pub flag: u8,
    pub nonce: [u8; 12],
    pub authdata: Vec<u8>,
    /// Offset in the encoded packet where authdata ends, i.e where the header ends.
    pub header_end_offset: usize,
}

impl PacketHeader {
    fn decode<T: StreamCipher>(
        cipher: &mut T,
        encoded_packet: &[u8],
    ) -> Result<PacketHeader, PacketCodecError> {
        // static header
        let mut static_header: [u8; STATIC_HEADER_SIZE] =
            encoded_packet[IV_MASKING_SIZE..STATIC_HEADER_END].try_into()?;

        cipher.try_apply_keystream(&mut static_header)?;

        // static-header = protocol-id || version || flag || nonce || authdata-size
        //protocol check
        let protocol_id = &static_header[..6];
        let version = u16::from_be_bytes(static_header[6..8].try_into()?);
        if protocol_id != PROTOCOL_ID || version != PROTOCOL_VERSION {
            return Err(PacketCodecError::InvalidProtocol(
                match str::from_utf8(protocol_id) {
                    Ok(result) => format!("{} v{}", result, version),
                    Err(_) => format!("{:?} v{}", protocol_id, version),
                },
            ));
        }

        let flag = static_header[8];
        let nonce = static_header[9..21].try_into()?;
        let authdata_size = u16::from_be_bytes(static_header[21..23].try_into()?) as usize;
        let authdata_end = STATIC_HEADER_END + authdata_size;
        let authdata = &mut encoded_packet[STATIC_HEADER_END..authdata_end].to_vec();

        cipher.try_apply_keystream(authdata)?;

        Ok(PacketHeader {
            static_header,
            flag,
            nonce,
            authdata: authdata.to_vec(),
            header_end_offset: authdata_end,
        })
    }

    fn encode<T: StreamCipher>(
        &self,
        buf: &mut dyn BufMut,
        cipher: &mut T,
    ) -> Result<(), PacketCodecError> {
        let mut static_header = Vec::new();
        static_header.put_slice(PROTOCOL_ID);
        static_header.put_slice(&PROTOCOL_VERSION.to_be_bytes());
        static_header.put_u8(self.flag);
        static_header.put_slice(&self.nonce);
        static_header.put_slice(&(self.authdata.len() as u16).to_be_bytes());
        cipher.try_apply_keystream(&mut static_header)?;
        buf.put_slice(&static_header);

        let mut authdata = self.authdata.clone();
        cipher.try_apply_keystream(&mut authdata)?;
        buf.put_slice(&authdata);

        Ok(())
    }
}

pub trait PacketTrait {
    const TYPE_FLAG: u8;
    fn encode_authdata(&self, buf: &mut dyn BufMut) -> Result<(), PacketCodecError>;
    fn get_encoded_message(&self) -> Vec<u8>;

    fn build_header(&self, nonce: &[u8; 12]) -> Result<PacketHeader, PacketCodecError> {
        let mut authdata = Vec::new();
        self.encode_authdata(&mut authdata)?;

        let authdata_size =
            u16::try_from(authdata.len()).map_err(|_| PacketCodecError::InvalidSize)?;

        let mut static_header: [u8; 23] = [0; 23];
        static_header[0..6].copy_from_slice(PROTOCOL_ID);
        static_header[6..8].copy_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        static_header[8] = Self::TYPE_FLAG;
        static_header[9..21].copy_from_slice(nonce);
        static_header[21..].copy_from_slice(&authdata_size.to_be_bytes());
        let header_end_offset = 16 + authdata.len() + static_header.len();
        Ok(PacketHeader {
            static_header,
            flag: Self::TYPE_FLAG,
            nonce: *nonce,
            authdata,
            header_end_offset,
        })
    }

    /// Encodes the packet
    fn encode(
        &self,
        nonce: &[u8; 12],
        masking_iv: [u8; 16],
        encrypt_key: &[u8],
    ) -> Result<Packet, PacketCodecError> {
        if encrypt_key.len() < 16 {
            return Err(PacketCodecError::InvalidSize);
        }
        let header = self.build_header(nonce)?;

        let mut message = self.get_encoded_message();
        let mut message_ad = masking_iv.to_vec();
        message_ad.extend_from_slice(&header.static_header);
        message_ad.extend_from_slice(&header.authdata);

        let mut cipher = Aes128Gcm::new(encrypt_key[..16].into());
        cipher
            .encrypt_in_place(&header.nonce.into(), &message_ad, &mut message)
            .map_err(|e| PacketCodecError::CipherError(e.to_string()))?;

        Ok(Packet {
            masking_iv,
            header,
            encrypted_message: message,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ordinary {
    pub src_id: H256,
    pub message: Message,
}

impl PacketTrait for Ordinary {
    const TYPE_FLAG: u8 = 0x00;

    fn encode_authdata(&self, buf: &mut dyn BufMut) -> Result<(), PacketCodecError> {
        buf.put_slice(self.src_id.as_bytes());
        Ok(())
    }

    fn get_encoded_message(&self) -> Vec<u8> {
        let mut message = Vec::new();
        self.message.encode(&mut message);
        message
    }
}

impl Ordinary {
    pub fn decode(packet: &Packet, decrypt_key: &[u8]) -> Result<Ordinary, PacketCodecError> {
        if packet.header.authdata.len() != 32 {
            return Err(PacketCodecError::InvalidSize);
        }

        let mut message = packet.encrypted_message.to_vec();
        decrypt_message(decrypt_key, packet, &mut message)?;

        let src_id = H256::from_slice(&packet.header.authdata);

        let message = Message::decode(&message).map_err(|_e| {
            PacketCodecError::InvalidMessage(message.first().copied().unwrap_or(0))
        })?;
        Ok(Ordinary { src_id, message })
    }
}

/// Decrypts a message using AES-128-GCM.
/// The message is decrypted in place.
pub fn decrypt_message(
    key: &[u8],
    packet: &Packet,
    message: &mut Vec<u8>,
) -> Result<(), PacketCodecError> {
    if key.len() < 16 {
        return Err(PacketCodecError::InvalidSize);
    }

    // message-ad = masking-iv || static-header || authdata
    let mut message_ad = packet.masking_iv.to_vec();
    message_ad.extend_from_slice(&packet.header.static_header);
    message_ad.extend_from_slice(&packet.header.authdata);

    let mut cipher = Aes128Gcm::new(key[..16].into());
    cipher
        .decrypt_in_place(packet.header.nonce.as_slice().into(), &message_ad, message)
        .map_err(|e| PacketCodecError::CipherError(e.to_string()))?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoAreYou {
    pub id_nonce: u128,
    pub enr_seq: u64,
}

impl PacketTrait for WhoAreYou {
    const TYPE_FLAG: u8 = 0x01;

    fn encode_authdata(&self, buf: &mut dyn BufMut) -> Result<(), PacketCodecError> {
        buf.put_slice(&self.id_nonce.to_be_bytes());
        buf.put_slice(&self.enr_seq.to_be_bytes());
        Ok(())
    }

    fn get_encoded_message(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Encodes the WhoAreYou packet.
    /// No encryption needed, just an empty message
    fn encode(
        &self,
        nonce: &[u8; 12],
        masking_iv: [u8; 16],
        _encrypt_key: &[u8],
    ) -> Result<Packet, PacketCodecError> {
        Ok(Packet {
            masking_iv,
            header: self.build_header(nonce)?,
            encrypted_message: Vec::new(),
        })
    }
}

impl WhoAreYou {
    pub fn decode(packet: &Packet) -> Result<WhoAreYou, PacketCodecError> {
        let authdata = packet.header.authdata.clone();
        let id_nonce = u128::from_be_bytes(authdata[..16].try_into()?);
        let enr_seq = u64::from_be_bytes(authdata[16..].try_into()?);

        Ok(WhoAreYou { id_nonce, enr_seq })
    }
}

/// Parsed handshake authdata, used for signature verification and session key derivation
/// before decrypting the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeAuthdata {
    pub src_id: H256,
    pub id_signature: Vec<u8>,
    pub eph_pubkey: Vec<u8>,
    pub record: Option<NodeRecord>,
}

impl HandshakeAuthdata {
    /// Decodes the authdata from a handshake packet header.
    /// This can be called before decryption to extract the ephemeral public key
    /// needed for session key derivation.
    pub fn decode(authdata: &[u8]) -> Result<Self, PacketCodecError> {
        if authdata.len() < HANDSHAKE_AUTHDATA_HEAD {
            return Err(PacketCodecError::InvalidSize);
        }

        let src_id = H256::from_slice(&authdata[..32]);
        let sig_size = authdata[32] as usize;
        let eph_key_size = authdata[33] as usize;

        let authdata_head = HANDSHAKE_AUTHDATA_HEAD + sig_size + eph_key_size;
        if authdata.len() < authdata_head {
            return Err(PacketCodecError::InvalidSize);
        }

        let id_signature =
            authdata[HANDSHAKE_AUTHDATA_HEAD..HANDSHAKE_AUTHDATA_HEAD + sig_size].to_vec();

        let eph_key_start = HANDSHAKE_AUTHDATA_HEAD + sig_size;
        let eph_pubkey = authdata[eph_key_start..authdata_head].to_vec();

        let record = if authdata.len() > authdata_head {
            let record_bytes = &authdata[authdata_head..];
            if record_bytes.is_empty() {
                None
            } else {
                Some(NodeRecord::decode(record_bytes)?)
            }
        } else {
            None
        };

        Ok(HandshakeAuthdata {
            src_id,
            id_signature,
            eph_pubkey,
            record,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub src_id: H256,
    pub id_signature: Vec<u8>,
    pub eph_pubkey: Vec<u8>,
    /// The record field may be omitted if the enr-seq of WHOAREYOU is recent enough, i.e. when it matches the current sequence number of the sending node.
    /// If enr-seq is zero, the record must be sent.
    pub record: Option<NodeRecord>,
    pub message: Message,
}

impl PacketTrait for Handshake {
    const TYPE_FLAG: u8 = 0x02;

    fn encode_authdata(&self, buf: &mut dyn BufMut) -> Result<(), PacketCodecError> {
        let sig_size: u8 = self
            .id_signature
            .len()
            .try_into()
            .map_err(|_| PacketCodecError::InvalidSize)?;
        let eph_key_size: u8 = self
            .eph_pubkey
            .len()
            .try_into()
            .map_err(|_| PacketCodecError::InvalidSize)?;

        buf.put_slice(self.src_id.as_bytes());
        buf.put_u8(sig_size);
        buf.put_u8(eph_key_size);
        buf.put_slice(&self.id_signature);
        buf.put_slice(&self.eph_pubkey);
        if let Some(record) = &self.record {
            record.encode(buf);
        }

        Ok(())
    }

    fn get_encoded_message(&self) -> Vec<u8> {
        let mut message = Vec::new();
        self.message.encode(&mut message);
        message
    }
}

impl Handshake {
    /// Decodes a handshake packet, including decrypting the message.
    pub fn decode(packet: &Packet, decrypt_key: &[u8]) -> Result<Handshake, PacketCodecError> {
        let authdata = HandshakeAuthdata::decode(&packet.header.authdata)?;

        let mut encrypted = packet.encrypted_message.to_vec();
        decrypt_message(decrypt_key, packet, &mut encrypted)?;
        let message = Message::decode(&encrypted)?;

        Ok(Handshake {
            src_id: authdata.src_id,
            id_signature: authdata.id_signature,
            eph_pubkey: authdata.eph_pubkey,
            record: authdata.record,
            message,
        })
    }

    /// Creates a Handshake from pre-parsed authdata and a decrypted message.
    /// Useful when authdata was already parsed for signature verification.
    pub fn from_authdata(authdata: HandshakeAuthdata, message: Message) -> Self {
        Handshake {
            src_id: authdata.src_id,
            id_signature: authdata.id_signature,
            eph_pubkey: authdata.eph_pubkey,
            record: authdata.record,
            message,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message {
    Ping(PingMessage),
    Pong(PongMessage),
    FindNode(FindNodeMessage),
    Nodes(NodesMessage),
    TalkReq(TalkReqMessage),
    TalkRes(TalkResMessage),
    Ticket(TicketMessage),
    // TODO: add the other messages
}

impl Message {
    fn msg_type(&self) -> u8 {
        match self {
            Message::Ping(_) => 0x01,
            Message::Pong(_) => 0x02,
            Message::FindNode(_) => 0x03,
            Message::Nodes(_) => 0x04,
            Message::TalkReq(_) => 0x05,
            Message::TalkRes(_) => 0x06,
            Message::Ticket(_) => 0x08,
        }
    }

    pub fn encode(&self, buf: &mut dyn BufMut) {
        buf.put_u8(self.msg_type());
        match self {
            Message::Ping(ping) => ping.encode(buf),
            Message::Pong(pong) => pong.encode(buf),
            Message::FindNode(find_node) => find_node.encode(buf),
            Message::Nodes(nodes) => nodes.encode(buf),
            Message::TalkReq(talk_req) => talk_req.encode(buf),
            Message::TalkRes(talk_res) => talk_res.encode(buf),
            Message::Ticket(ticket) => ticket.encode(buf),
        }
    }

    pub fn decode(message: &[u8]) -> Result<Message, RLPDecodeError> {
        let &message_type = message.first().ok_or(RLPDecodeError::InvalidLength)?;
        match message_type {
            0x01 => {
                let ping = PingMessage::decode(&message[1..])?;
                Ok(Message::Ping(ping))
            }
            0x02 => {
                let pong = PongMessage::decode(&message[1..])?;
                Ok(Message::Pong(pong))
            }
            0x03 => {
                let find_node_msg = FindNodeMessage::decode(&message[1..])?;
                Ok(Message::FindNode(find_node_msg))
            }
            0x04 => {
                let nodes_msg = NodesMessage::decode(&message[1..])?;
                Ok(Message::Nodes(nodes_msg))
            }
            0x05 => {
                let talk_req_msg = TalkReqMessage::decode(&message[1..])?;
                Ok(Message::TalkReq(talk_req_msg))
            }
            0x06 => {
                let enr_response_msg = TalkResMessage::decode(&message[1..])?;
                Ok(Message::TalkRes(enr_response_msg))
            }
            0x08 => {
                let ticket_msg = TicketMessage::decode(&message[1..])?;
                Ok(Message::Ticket(ticket_msg))
            }
            _ => Err(RLPDecodeError::MalformedData),
        }
    }
}

impl Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Message::Ping(_) => write!(f, "Ping"),
            Message::Pong(_) => write!(f, "Pong"),
            Message::FindNode(_) => write!(f, "FindNode"),
            Message::Nodes(_) => write!(f, "Nodes"),
            Message::TalkReq(_) => write!(f, "TalkReq"),
            Message::TalkRes(_) => write!(f, "TalkRes"),
            Message::Ticket(_) => write!(f, "Ticket"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingMessage {
    /// The request id of the sender.
    pub req_id: Bytes,
    /// The ENR sequence number of the sender.
    pub enr_seq: u64,
}

impl PingMessage {
    pub fn new(req_id: Bytes, enr_seq: u64) -> Self {
        Self { req_id, enr_seq }
    }
}

impl RLPEncode for PingMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.enr_seq)
            .finish();
    }
}

impl RLPDecode for PingMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (enr_seq, decoder) = decoder.decode_field("enr_seq")?;
        let ping = PingMessage { req_id, enr_seq };
        Ok((ping, decoder.finish()?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PongMessage {
    pub req_id: Bytes,
    pub enr_seq: u64,
    pub recipient_addr: SocketAddr,
}

impl RLPEncode for PongMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.enr_seq)
            .encode_field(&self.recipient_addr.ip())
            .encode_field(&self.recipient_addr.port())
            .finish();
    }
}

impl RLPDecode for PongMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        use std::net::IpAddr;
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (enr_seq, decoder) = decoder.decode_field("enr_seq")?;
        let (recipient_ip, decoder): (IpAddr, _) = decoder.decode_field("recipient_ip")?;
        let (recipient_port, decoder): (u16, _) = decoder.decode_field("recipient_port")?;

        Ok((
            Self {
                req_id,
                enr_seq,
                recipient_addr: SocketAddr::new(recipient_ip, recipient_port),
            },
            decoder.finish()?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindNodeMessage {
    pub req_id: Bytes,
    pub distances: Vec<u32>,
}

impl RLPEncode for FindNodeMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.distances)
            .finish();
    }
}

impl RLPDecode for FindNodeMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (distance, decoder) = decoder.decode_field("distance")?;

        Ok((
            Self {
                req_id,
                distances: distance,
            },
            decoder.finish()?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodesMessage {
    pub req_id: Bytes,
    pub total: u64,
    pub nodes: Vec<NodeRecord>,
}

impl RLPEncode for NodesMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.total)
            .encode_field(&self.nodes)
            .finish();
    }
}

impl RLPDecode for NodesMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (total, decoder) = decoder.decode_field("total")?;
        let (nodes, decoder) = decoder.decode_field("nodes")?;

        Ok((
            Self {
                req_id,
                total,
                nodes,
            },
            decoder.finish()?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalkReqMessage {
    pub req_id: Bytes,
    pub protocol: Bytes,
    pub request: Bytes,
}

impl RLPEncode for TalkReqMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.protocol)
            .encode_field(&self.request)
            .finish();
    }
}

impl RLPDecode for TalkReqMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (protocol, decoder) = decoder.decode_field("protocol")?;
        let (request, decoder) = decoder.decode_field("request")?;

        Ok((
            Self {
                req_id,
                protocol,
                request,
            },
            decoder.finish()?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalkResMessage {
    pub req_id: Bytes,
    pub response: Vec<u8>,
}

impl RLPEncode for TalkResMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&Bytes::copy_from_slice(&self.response))
            .finish();
    }
}

impl RLPDecode for TalkResMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let ((req_id, response), remaining) =
            <(Bytes, Bytes) as RLPDecode>::decode_unfinished(rlp)?;

        Ok((
            Self {
                req_id,
                response: response.to_vec(),
            },
            remaining,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketMessage {
    pub req_id: Bytes,
    pub ticket: Bytes,
    pub wait_time: u64,
}

impl RLPEncode for TicketMessage {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.req_id)
            .encode_field(&self.ticket)
            .encode_field(&self.wait_time)
            .finish();
    }
}

impl RLPDecode for TicketMessage {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (req_id, decoder) = decoder.decode_field("req_id")?;
        let (ticket, decoder) = decoder.decode_field("ticket")?;
        let (wait_time, decoder) = decoder.decode_field("wait_time")?;

        Ok((
            Self {
                req_id,
                ticket,
                wait_time,
            },
            decoder.finish()?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        discv5::{
            messages::{Message, Ordinary, PingMessage, WhoAreYou},
            session::{build_challenge_data, create_id_signature, derive_session_keys},
        },
        rlpx::utils::compress_pubkey,
        types::NodeRecordPairs,
        utils::{node_id, public_key_from_signing_key},
    };
    use aes_gcm::{Aes128Gcm, KeyInit, aead::AeadMutInPlace};
    use bytes::BytesMut;
    use ethrex_common::{H264, H512};
    use hex_literal::hex;
    use secp256k1::SecretKey;
    use std::{
        net::{Ipv4Addr, SocketAddr},
        str::FromStr,
    };

    // node-a-key = 0xeef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f
    // node-b-key = 0x66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628
    // let node_a_key = SecretKey::from_byte_array(&hex!(
    //     "eef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f"
    // ))
    // .unwrap();
    // let node_b_key = SecretKey::from_byte_array(&hex!(
    //     "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
    // ))
    // .unwrap();

    /// Ping message packet (flag 0) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn decode_ping_packet() {
        /*
        # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        # nonce = 0xffffffffffffffffffffffff
        # read-key = 0x00000000000000000000000000000000
        # ping.req-id = 0x00000001
        # ping.enr-seq = 2

        00000000000000000000000000000000088b3d4342774649325f313964a39e55
        ea96c005ad52be8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d3
        4c4f53245d08dab84102ed931f66d1492acb308fa1c6715b9d139b81acbdcc
        */

        let node_a_key = SecretKey::from_byte_array(&hex!(
            "eef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f"
        ))
        .unwrap();
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let src_id = node_id(&public_key_from_signing_key(&node_a_key));
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649325f313964a39e55ea96c005ad52be8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08dab84102ed931f66d1492acb308fa1c6715b9d139b81acbdcc"
        );
        let packet = Packet::decode(&dest_id, encoded).unwrap();
        assert_eq!([0; 16], packet.masking_iv);
        assert_eq!(0x00, packet.header.flag);
        assert_eq!(hex!("ffffffffffffffffffffffff"), packet.header.nonce);

        // # read-key = 0x00000000000000000000000000000000
        let read_key = [0; 16];

        let decoded_message = Ordinary::decode(&packet, &read_key).unwrap();

        let expected_message = Ordinary {
            src_id,
            message: Message::Ping(PingMessage {
                req_id: Bytes::from(hex!("00000001").as_slice()),
                enr_seq: 2,
            }),
        };

        assert_eq!(decoded_message, expected_message);
    }

    /// Ping message packet (flag 0) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn encode_ping_packet() {
        /*
        # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        # nonce = 0xffffffffffffffffffffffff
        # read-key = 0x00000000000000000000000000000000
        # ping.req-id = 0x00000001
        # ping.enr-seq = 2

        00000000000000000000000000000000088b3d4342774649325f313964a39e55
        ea96c005ad52be8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d3
        4c4f53245d08dab84102ed931f66d1492acb308fa1c6715b9d139b81acbdcc
        */

        let node_a_key = SecretKey::from_byte_array(&hex!(
            "eef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f"
        ))
        .unwrap();
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let src_id = node_id(&public_key_from_signing_key(&node_a_key));
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let message = Ordinary {
            src_id,
            message: Message::Ping(PingMessage {
                req_id: Bytes::from(hex!("00000001").as_slice()),
                enr_seq: 2,
            }),
        };

        let masking_iv = [0; 16];
        let nonce = hex!("ffffffffffffffffffffffff");

        // # read-key = 0x00000000000000000000000000000000
        let encrypt_key = [0; 16];

        let packet = message.encode(&nonce, masking_iv, &encrypt_key).unwrap();

        let expected_encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649325f313964a39e55ea96c005ad52be8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08dab84102ed931f66d1492acb308fa1c6715b9d139b81acbdcc"
        );

        let mut buf = BytesMut::new();
        packet.encode(&mut buf, &dest_id).unwrap();

        assert_eq!(buf.to_vec(), expected_encoded);
    }

    #[test]
    fn decode_whoareyou_packet() {
        // # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        // # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        // # whoareyou.challenge-data = 0x000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000000
        // # whoareyou.request-nonce = 0x0102030405060708090a0b0c
        // # whoareyou.id-nonce = 0x0102030405060708090a0b0c0d0e0f10
        // # whoareyou.enr-seq = 0
        //
        // 00000000000000000000000000000000088b3d434277464933a1ccc59f5967ad
        // 1d6035f15e528627dde75cd68292f9e6c27d6b66c8100a873fcbaed4e16b8d
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d434277464933a1ccc59f5967ad1d6035f15e528627dde75cd68292f9e6c27d6b66c8100a873fcbaed4e16b8d"
        );

        let packet = Packet::decode(&dest_id, encoded).unwrap();
        assert_eq!([0; 16], packet.masking_iv);
        assert_eq!(0x01, packet.header.flag);
        assert_eq!(hex!("0102030405060708090a0b0c"), packet.header.nonce);

        let challenge_data = build_challenge_data(
            &packet.masking_iv,
            &packet.header.static_header,
            &packet.header.authdata,
        );

        let expected_challenge_data = &hex!(
            "000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000000"
        );
        assert_eq!(challenge_data, expected_challenge_data);
        let decoded_message = WhoAreYou::decode(&packet).unwrap();

        let expected_message = WhoAreYou {
            id_nonce: u128::from_be_bytes(
                hex!("0102030405060708090a0b0c0d0e0f10")
                    .to_vec()
                    .try_into()
                    .unwrap(),
            ),
            enr_seq: 0,
        };

        assert_eq!(decoded_message, expected_message);
    }

    #[test]
    fn encode_whoareyou_packet() {
        // # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        // # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        // # whoareyou.challenge-data = 0x000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000000
        // # whoareyou.request-nonce = 0x0102030405060708090a0b0c
        // # whoareyou.id-nonce = 0x0102030405060708090a0b0c0d0e0f10
        // # whoareyou.enr-seq = 0
        //
        // 00000000000000000000000000000000088b3d434277464933a1ccc59f5967ad
        // 1d6035f15e528627dde75cd68292f9e6c27d6b66c8100a873fcbaed4e16b8d
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let who_are_you = WhoAreYou {
            id_nonce: u128::from_be_bytes(
                hex!("0102030405060708090a0b0c0d0e0f10")
                    .to_vec()
                    .try_into()
                    .unwrap(),
            ),
            enr_seq: 0,
        };

        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let masking_iv = [0; 16];
        let nonce = hex!("0102030405060708090a0b0c");

        let packet = who_are_you.encode(&nonce, masking_iv, &[]).unwrap();

        let expected_encoded = &hex!(
            "00000000000000000000000000000000088b3d434277464933a1ccc59f5967ad1d6035f15e528627dde75cd68292f9e6c27d6b66c8100a873fcbaed4e16b8d"
        );

        let mut buf = BytesMut::new();
        packet.encode(&mut buf, &dest_id).unwrap();

        assert_eq!(buf.to_vec(), expected_encoded);
    }

    /// Ping handshake packet (flag 2) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn encode_ping_handshake_packet() {
        /*
        # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        # nonce = 0xffffffffffffffffffffffff
        # read-key = 0x4f9fac6de7567d1e3b1241dffe90f662
        # ping.req-id = 0x00000001
        # ping.enr-seq = 1
        #
        # handshake inputs:
        #
        # whoareyou.challenge-data = 0x000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000001
        # whoareyou.request-nonce = 0x0102030405060708090a0b0c
        # whoareyou.id-nonce = 0x0102030405060708090a0b0c0d0e0f10
        # whoareyou.enr-seq = 1
        # ephemeral-key = 0x0288ef00023598499cb6c940146d050d2b1fb914198c327f76aad590bead68b6
        # ephemeral-pubkey = 0x039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5

        00000000000000000000000000000000088b3d4342774649305f313964a39e55
        ea96c005ad521d8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d3
        4c4f53245d08da4bb252012b2cba3f4f374a90a75cff91f142fa9be3e0a5f3ef
        268ccb9065aeecfd67a999e7fdc137e062b2ec4a0eb92947f0d9a74bfbf44dfb
        a776b21301f8b65efd5796706adff216ab862a9186875f9494150c4ae06fa4d1
        f0396c93f215fa4ef524f1eadf5f0f4126b79336671cbcf7a885b1f8bd2a5d83
        9cf8
         */
        let node_a_key = SecretKey::from_byte_array(&hex!(
            "eef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f"
        ))
        .unwrap();
        let src_id = node_id(&public_key_from_signing_key(&node_a_key));
        let expected_src_id = H256::from_slice(&hex!(
            "aaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb"
        ));
        assert_eq!(src_id, expected_src_id);

        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();
        let dest_pub_key = public_key_from_signing_key(&node_b_key);
        let dest_pubkey = compress_pubkey(dest_pub_key).unwrap();
        let dest_id = node_id(&dest_pub_key);

        let message = Message::Ping(PingMessage {
            req_id: Bytes::from(hex!("00000001").as_slice()),
            enr_seq: 1,
        });

        let challenge_data = hex!("000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000001").to_vec();

        let ephemeral_key = SecretKey::from_byte_array(&hex!(
            "0288ef00023598499cb6c940146d050d2b1fb914198c327f76aad590bead68b6"
        ))
        .unwrap();
        let expected_ephemeral_pubkey =
            hex!("039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5");

        let ephemeral_pubkey = ephemeral_key.public_key(secp256k1::SECP256K1).serialize();

        assert_eq!(ephemeral_pubkey, expected_ephemeral_pubkey);

        let session = derive_session_keys(
            &ephemeral_key,
            &dest_pubkey,
            &src_id,
            &dest_id,
            &challenge_data,
            true, // initiator
        );

        let expected_read_key = hex!("4f9fac6de7567d1e3b1241dffe90f662");
        assert_eq!(session.outbound_key, expected_read_key);

        let signature =
            create_id_signature(&node_a_key, &challenge_data, &ephemeral_pubkey, &dest_id);

        let handshake = Handshake {
            src_id,
            id_signature: signature.serialize_compact().to_vec(),
            eph_pubkey: ephemeral_pubkey.to_vec(),
            record: None,
            message,
        };

        let masking_iv = [0; 16];
        let nonce = hex!("ffffffffffffffffffffffff");

        let packet = handshake
            .encode(&nonce, masking_iv, &session.outbound_key)
            .unwrap();

        let expected_encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649305f313964a39e55ea96c005ad521d8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08da4bb252012b2cba3f4f374a90a75cff91f142fa9be3e0a5f3ef268ccb9065aeecfd67a999e7fdc137e062b2ec4a0eb92947f0d9a74bfbf44dfba776b21301f8b65efd5796706adff216ab862a9186875f9494150c4ae06fa4d1f0396c93f215fa4ef524f1eadf5f0f4126b79336671cbcf7a885b1f8bd2a5d839cf8"
        );

        let mut buf = BytesMut::new();
        packet.encode(&mut buf, &dest_id).unwrap();

        assert_eq!(buf.to_vec(), expected_encoded);
    }

    /// Ping handshake message packet (flag 2, with ENR) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn decode_ping_handshake_packet_with_enr() {
        /*
        # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        # nonce = 0xffffffffffffffffffffffff
        # read-key = 0x53b1c075f41876423154e157470c2f48
        # ping.req-id = 0x00000001
        # ping.enr-seq = 1
        #
        # handshake inputs:
        #
        # whoareyou.challenge-data = 0x000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000000
        # whoareyou.request-nonce = 0x0102030405060708090a0b0c
        # whoareyou.id-nonce = 0x0102030405060708090a0b0c0d0e0f10
        # whoareyou.enr-seq = 0
        # ephemeral-key = 0x0288ef00023598499cb6c940146d050d2b1fb914198c327f76aad590bead68b6
        # ephemeral-pubkey = 0x039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5

        00000000000000000000000000000000088b3d4342774649305f313964a39e55
        ea96c005ad539c8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d3
        4c4f53245d08da4bb23698868350aaad22e3ab8dd034f548a1c43cd246be9856
        2fafa0a1fa86d8e7a3b95ae78cc2b988ded6a5b59eb83ad58097252188b902b2
        1481e30e5e285f19735796706adff216ab862a9186875f9494150c4ae06fa4d1
        f0396c93f215fa4ef524e0ed04c3c21e39b1868e1ca8105e585ec17315e755e6
        cfc4dd6cb7fd8e1a1f55e49b4b5eb024221482105346f3c82b15fdaae36a3bb1
        2a494683b4a3c7f2ae41306252fed84785e2bbff3b022812d0882f06978df84a
        80d443972213342d04b9048fc3b1d5fcb1df0f822152eced6da4d3f6df27e70e
        4539717307a0208cd208d65093ccab5aa596a34d7511401987662d8cf62b1394
        71
        */
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded_packet = &hex!(
            "00000000000000000000000000000000088b3d4342774649305f313964a39e55ea96c005ad539c8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08da4bb23698868350aaad22e3ab8dd034f548a1c43cd246be98562fafa0a1fa86d8e7a3b95ae78cc2b988ded6a5b59eb83ad58097252188b902b21481e30e5e285f19735796706adff216ab862a9186875f9494150c4ae06fa4d1f0396c93f215fa4ef524e0ed04c3c21e39b1868e1ca8105e585ec17315e755e6cfc4dd6cb7fd8e1a1f55e49b4b5eb024221482105346f3c82b15fdaae36a3bb12a494683b4a3c7f2ae41306252fed84785e2bbff3b022812d0882f06978df84a80d443972213342d04b9048fc3b1d5fcb1df0f822152eced6da4d3f6df27e70e4539717307a0208cd208d65093ccab5aa596a34d7511401987662d8cf62b139471"
        );
        let read_key = hex!("53b1c075f41876423154e157470c2f48");

        let packet = Packet::decode(&dest_id, encoded_packet).unwrap();
        assert_eq!([0; 16], packet.masking_iv);
        assert_eq!(0x02, packet.header.flag);
        assert_eq!(hex!("ffffffffffffffffffffffff"), packet.header.nonce);

        let handshake = Handshake::decode(&packet, &read_key).unwrap();

        assert_eq!(
            handshake.src_id,
            H256::from_slice(&hex!(
                "aaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb"
            ))
        );
        assert_eq!(
            handshake.eph_pubkey,
            hex!("039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5").to_vec()
        );
        assert_eq!(
            handshake.message,
            Message::Ping(PingMessage {
                req_id: Bytes::from(hex!("00000001").as_slice()),
                enr_seq: 1,
            })
        );

        let record = handshake.record.expect("expected ENR record");
        let pairs = record.decode_pairs();
        assert_eq!(pairs.id.as_deref(), Some("v4"));
        assert!(pairs.secp256k1.is_some());
    }

    /// Ping handshake packet (flag 2, with ENR) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn encode_ping_handshake_packet_with_enr() {
        /*
        # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        # nonce = 0xffffffffffffffffffffffff
        # read-key = 0x53b1c075f41876423154e157470c2f48
        # ping.req-id = 0x00000001
        # ping.enr-seq = 1
        #
        # handshake inputs:
        #
        # whoareyou.challenge-data = 0x000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000000
        # whoareyou.request-nonce = 0x0102030405060708090a0b0c
        # whoareyou.id-nonce = 0x0102030405060708090a0b0c0d0e0f10
        # whoareyou.enr-seq = 0
        # ephemeral-key = 0x0288ef00023598499cb6c940146d050d2b1fb914198c327f76aad590bead68b6
        # ephemeral-pubkey = 0x039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5

        00000000000000000000000000000000088b3d4342774649305f313964a39e55
        ea96c005ad539c8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d3
        4c4f53245d08da4bb23698868350aaad22e3ab8dd034f548a1c43cd246be9856
        2fafa0a1fa86d8e7a3b95ae78cc2b988ded6a5b59eb83ad58097252188b902b2
        1481e30e5e285f19735796706adff216ab862a9186875f9494150c4ae06fa4d1
        f0396c93f215fa4ef524e0ed04c3c21e39b1868e1ca8105e585ec17315e755e6
        cfc4dd6cb7fd8e1a1f55e49b4b5eb024221482105346f3c82b15fdaae36a3bb1
        2a494683b4a3c7f2ae41306252fed84785e2bbff3b022812d0882f06978df84a
        80d443972213342d04b9048fc3b1d5fcb1df0f822152eced6da4d3f6df27e70e
        4539717307a0208cd208d65093ccab5aa596a34d7511401987662d8cf62b1394
        71
        */
        let node_a_key = SecretKey::from_byte_array(&hex!(
            "eef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f"
        ))
        .unwrap();
        let src_id = node_id(&public_key_from_signing_key(&node_a_key));
        let expected_src_id = H256::from_slice(&hex!(
            "aaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb"
        ));
        assert_eq!(src_id, expected_src_id);

        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();
        let dest_pub_key = public_key_from_signing_key(&node_b_key);
        let dest_pubkey = compress_pubkey(dest_pub_key).unwrap();
        let dest_id: H256 = node_id(&dest_pub_key);

        let message = Message::Ping(PingMessage {
            req_id: Bytes::from(hex!("00000001").as_slice()),
            enr_seq: 1,
        });

        let challenge_data = hex!("000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000000").to_vec();

        let ephemeral_key = SecretKey::from_byte_array(&hex!(
            "0288ef00023598499cb6c940146d050d2b1fb914198c327f76aad590bead68b6"
        ))
        .unwrap();
        let expected_ephemeral_pubkey =
            hex!("039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5");

        let ephemeral_pubkey = ephemeral_key.public_key(secp256k1::SECP256K1).serialize();

        assert_eq!(ephemeral_pubkey, expected_ephemeral_pubkey);

        let session = derive_session_keys(
            &ephemeral_key,
            &dest_pubkey,
            &src_id,
            &dest_id,
            &challenge_data,
            true, // initiator
        );

        let expected_read_key = hex!("53b1c075f41876423154e157470c2f48");
        assert_eq!(session.outbound_key, expected_read_key);

        let signature =
            create_id_signature(&node_a_key, &challenge_data, &ephemeral_pubkey, &dest_id);

        let sig = "17e1b073918da32d640642c762c0e2781698e4971f8ab39a77746adad83f01e76ffc874c5924808bbe7c50890882c2b8a01287a0b08312d1d53a17d517f5eb27";
        let key = "0313d14211e0287b2361a1615890a9b5212080546d0a257ae4cff96cf534992cb9";

        let record = NodeRecord {
            signature: H512::from_str(sig).unwrap(),
            seq: 1,
            pairs: NodeRecordPairs {
                id: Some("v4".to_owned()),
                ip: Some(Ipv4Addr::new(127, 0, 0, 1)),
                ip6: None,
                tcp_port: None,
                udp_port: None,
                secp256k1: Some(H264::from_str(key).unwrap()),
                eth: None,
            }
            .into(),
        };

        let handshake = Handshake {
            src_id,
            id_signature: signature.serialize_compact().to_vec(),
            eph_pubkey: ephemeral_pubkey.to_vec(),
            record: Some(record),
            message,
        };

        let masking_iv = [0; 16];
        let nonce = hex!("ffffffffffffffffffffffff");

        let packet = handshake
            .encode(&nonce, masking_iv, &session.outbound_key)
            .unwrap();

        let expected_encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649305f313964a39e55ea96c005ad539c8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08da4bb23698868350aaad22e3ab8dd034f548a1c43cd246be98562fafa0a1fa86d8e7a3b95ae78cc2b988ded6a5b59eb83ad58097252188b902b21481e30e5e285f19735796706adff216ab862a9186875f9494150c4ae06fa4d1f0396c93f215fa4ef524e0ed04c3c21e39b1868e1ca8105e585ec17315e755e6cfc4dd6cb7fd8e1a1f55e49b4b5eb024221482105346f3c82b15fdaae36a3bb12a494683b4a3c7f2ae41306252fed84785e2bbff3b022812d0882f06978df84a80d443972213342d04b9048fc3b1d5fcb1df0f822152eced6da4d3f6df27e70e4539717307a0208cd208d65093ccab5aa596a34d7511401987662d8cf62b139471"
        );

        let mut buf = BytesMut::new();
        packet.encode(&mut buf, &dest_id).unwrap();

        assert_eq!(buf.to_vec(), expected_encoded);
    }

    #[test]
    fn aes_gcm_vector() {
        // https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md#encryptiondecryption
        let key = hex!("9f2d77db7004bf8a1a85107ac686990b");
        let nonce = hex!("27b5af763c446acd2749fe8e");
        let ad = hex!("93a7400fa0d6a694ebc24d5cf570f65d04215b6ac00757875e3f3a5f42107903");
        let mut pt = hex!("01c20101").to_vec();

        let mut cipher = Aes128Gcm::new_from_slice(&key).unwrap();
        cipher
            .encrypt_in_place(nonce.as_slice().into(), &ad, &mut pt)
            .unwrap();

        assert_eq!(
            pt,
            hex!("a5d12a2d94b8ccb3ba55558229867dc13bfa3648").to_vec()
        );
    }

    #[test]
    fn handshake_packet_roundtrip() {
        let node_a_key = SecretKey::from_byte_array(&hex!(
            "eef77acb6c6a6eebc5b363a475ac583ec7eccdb42b6481424c60f59aa326547f"
        ))
        .unwrap();
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();

        let src_id = node_id(&public_key_from_signing_key(&node_a_key));
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let handshake = Handshake {
            src_id,
            id_signature: vec![1; 64],
            eph_pubkey: vec![2; 33],
            record: None,
            message: Message::Ping(PingMessage {
                req_id: Bytes::from_static(&[3]),
                enr_seq: 4,
            }),
        };

        let key = [0x10; 16];
        let nonce = hex!("000102030405060708090a0b");
        let mut buf = Vec::new();

        let masking_iv = [0; 16];
        let packet = handshake.encode(&nonce, masking_iv, &key).unwrap();
        packet.encode(&mut buf, &dest_id).unwrap();

        let decoded = Packet::decode(&dest_id, &buf).unwrap();
        assert_eq!(decoded, packet);
    }

    /// Ping handshake packet (flag 2) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn handshake_packet_vector_roundtrip() {
        /*
        # src-node-id = 0xaaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb
        # dest-node-id = 0xbbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9
        # nonce = 0xffffffffffffffffffffffff
        # read-key = 0x4f9fac6de7567d1e3b1241dffe90f662
        # ping.req-id = 0x00000001
        # ping.enr-seq = 1
        #
        # handshake inputs:
        #
        # whoareyou.challenge-data = 0x000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000001
        # whoareyou.request-nonce = 0x0102030405060708090a0b0c
        # whoareyou.id-nonce = 0x0102030405060708090a0b0c0d0e0f10
        # whoareyou.enr-seq = 1
        # ephemeral-key = 0x0288ef00023598499cb6c940146d050d2b1fb914198c327f76aad590bead68b6
        # ephemeral-pubkey = 0x039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5

        00000000000000000000000000000000088b3d4342774649305f313964a39e55
        ea96c005ad521d8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d3
        4c4f53245d08da4bb252012b2cba3f4f374a90a75cff91f142fa9be3e0a5f3ef
        268ccb9065aeecfd67a999e7fdc137e062b2ec4a0eb92947f0d9a74bfbf44dfb
        a776b21301f8b65efd5796706adff216ab862a9186875f9494150c4ae06fa4d1
        f0396c93f215fa4ef524f1eadf5f0f4126b79336671cbcf7a885b1f8bd2a5d83
        9cf8
         */
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649305f313964a39e55ea96c005ad521d8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08da4bb252012b2cba3f4f374a90a75cff91f142fa9be3e0a5f3ef268ccb9065aeecfd67a999e7fdc137e062b2ec4a0eb92947f0d9a74bfbf44dfba776b21301f8b65efd5796706adff216ab862a9186875f9494150c4ae06fa4d1f0396c93f215fa4ef524f1eadf5f0f4126b79336671cbcf7a885b1f8bd2a5d839cf8"
        );
        let read_key = hex!("4f9fac6de7567d1e3b1241dffe90f662");

        let packet = Packet::decode(&dest_id, encoded).unwrap();
        let handshake = Handshake::decode(&packet, &read_key).unwrap();

        assert_eq!(
            handshake.src_id,
            H256::from_slice(&hex!(
                "aaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb"
            ))
        );
        assert_eq!(handshake.record, None);
        assert_eq!(
            handshake.eph_pubkey,
            hex!("039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5").to_vec()
        );
        assert_eq!(
            handshake.message,
            Message::Ping(PingMessage {
                req_id: Bytes::from(hex!("00000001").as_slice()),
                enr_seq: 1,
            })
        );

        let masking_iv = encoded[..16].try_into().unwrap();
        let nonce = hex!("ffffffffffffffffffffffff");
        let mut buf = Vec::new();
        let packet = handshake.encode(&nonce, masking_iv, &read_key).unwrap();
        packet.encode(&mut buf, &dest_id).unwrap();

        assert_eq!(buf, encoded.to_vec());
    }

    /// Ping handshake message packet (flag 2, with ENR) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn handshake_packet_with_enr_vector_roundtrip() {
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649305f313964a39e55ea96c005ad539c8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08da4bb23698868350aaad22e3ab8dd034f548a1c43cd246be98562fafa0a1fa86d8e7a3b95ae78cc2b988ded6a5b59eb83ad58097252188b902b21481e30e5e285f19735796706adff216ab862a9186875f9494150c4ae06fa4d1f0396c93f215fa4ef524e0ed04c3c21e39b1868e1ca8105e585ec17315e755e6cfc4dd6cb7fd8e1a1f55e49b4b5eb024221482105346f3c82b15fdaae36a3bb12a494683b4a3c7f2ae41306252fed84785e2bbff3b022812d0882f06978df84a80d443972213342d04b9048fc3b1d5fcb1df0f822152eced6da4d3f6df27e70e4539717307a0208cd208d65093ccab5aa596a34d7511401987662d8cf62b139471"
        );
        let nonce = hex!("ffffffffffffffffffffffff");
        let read_key = hex!("53b1c075f41876423154e157470c2f48");

        let packet = Packet::decode(&dest_id, encoded).unwrap();
        let handshake = Handshake::decode(&packet, &read_key).unwrap();

        assert_eq!(
            handshake.src_id,
            H256::from_slice(&hex!(
                "aaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb"
            ))
        );
        assert_eq!(
            handshake.eph_pubkey,
            hex!("039a003ba6517b473fa0cd74aefe99dadfdb34627f90fec6362df85803908f53a5").to_vec()
        );
        assert_eq!(
            handshake.message,
            Message::Ping(PingMessage {
                req_id: Bytes::from(hex!("00000001").as_slice()),
                enr_seq: 1,
            })
        );

        let record = handshake.record.clone().expect("expected ENR record");
        let pairs = record.decode_pairs();
        assert_eq!(pairs.id.as_deref(), Some("v4"));
        assert!(pairs.secp256k1.is_some());

        let masking_iv = encoded[..16].try_into().unwrap();
        let mut buf = Vec::new();

        let packet = handshake.encode(&nonce, masking_iv, &read_key).unwrap();
        packet.encode(&mut buf, &dest_id).unwrap();

        assert_eq!(buf, encoded.to_vec());
    }

    /// Ping message packet (flag 0) from https://github.com/ethereum/devp2p/blob/master/discv5/discv5-wire-test-vectors.md
    #[test]
    fn ordinary_ping_packet_vector_roundtrip() {
        let node_b_key = SecretKey::from_byte_array(&hex!(
            "66fb62bfbd66b9177a138c1e5cddbe4f7c30c343e94e68df8769459cb1cde628"
        ))
        .unwrap();
        let dest_id = node_id(&public_key_from_signing_key(&node_b_key));

        let encoded = &hex!(
            "00000000000000000000000000000000088b3d4342774649325f313964a39e55ea96c005ad52be8c7560413a7008f16c9e6d2f43bbea8814a546b7409ce783d34c4f53245d08dab84102ed931f66d1492acb308fa1c6715b9d139b81acbdcc"
        );
        let nonce = hex!("ffffffffffffffffffffffff");
        let read_key = [0; 16];

        let packet = Packet::decode(&dest_id, encoded).unwrap();
        let message = Ordinary {
            src_id: H256::from_slice(&hex!(
                "aaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb"
            )),
            message: Message::Ping(PingMessage {
                req_id: Bytes::from(hex!("00000001").as_slice()),
                enr_seq: 2,
            }),
        };
        let masking_iv = [0; 16];
        let expected = message.encode(&nonce, masking_iv, &read_key).unwrap();

        assert_eq!(packet, expected);

        let mut buf = Vec::new();
        packet.encode(&mut buf, &dest_id).unwrap();
        assert_eq!(buf, encoded.to_vec());
    }

    #[test]
    fn ping_packet_codec_roundtrip() {
        let pkt = PingMessage {
            req_id: Bytes::from_static(&[1, 2, 3, 4]),
            enr_seq: 4321,
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(PingMessage::decode(&buf).unwrap(), pkt);
    }

    // TODO: Test encode pong packet (with known good encoding).
    // TODO: Test decode pong packet (from known good encoding).
    #[test]
    fn pong_packet_codec_roundtrip() {
        let pkt = PongMessage {
            req_id: Bytes::from_static(&[1, 2, 3, 4]),
            enr_seq: 4321,
            recipient_addr: SocketAddr::new(Ipv4Addr::BROADCAST.into(), 30303),
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(PongMessage::decode(&buf).unwrap(), pkt);
    }

    #[test]
    fn findnode_packet_codec_roundtrip() {
        let pkt = FindNodeMessage {
            req_id: Bytes::from_static(&[1, 2, 3, 4]),
            distances: vec![0],
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(FindNodeMessage::decode(&buf).unwrap(), pkt);
    }

    #[test]
    fn nodes_packet_codec_roundtrip() {
        let pairs: Vec<(Bytes, Bytes)> = NodeRecordPairs {
            id: Some("id".to_string()),
            ..Default::default()
        }
        .into();

        let pkt = NodesMessage {
            req_id: Bytes::from_static(&[1, 2, 3, 4]),
            total: 2,
            nodes: vec![NodeRecord {
                seq: 4321,
                pairs,
                signature: H512::random(),
            }],
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(NodesMessage::decode(&buf).unwrap(), pkt);
    }

    #[test]
    fn talkreq_packet_codec_roundtrip() {
        let pkt = TalkReqMessage {
            req_id: Bytes::from_static(&[1, 2, 3, 4]),
            protocol: Bytes::from_static(&[1, 2, 3, 4]),
            request: Bytes::from_static(&[1, 2, 3, 4]),
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(TalkReqMessage::decode(&buf).unwrap(), pkt);
    }

    #[test]
    fn talk_res_packet_codec_roundtrip() {
        let pkt = TalkResMessage {
            req_id: Bytes::from_static(&[1, 2, 3, 4]),
            response: b"\x00\x01\x02\x03".into(),
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(TalkResMessage::decode(&buf).unwrap(), pkt);
    }

    #[test]
    fn ticket_packet_codec_roundtrip() {
        let pkt = TicketMessage {
            req_id: Bytes::from_static(&[1, 2, 3, 4]),
            ticket: Bytes::from_static(&[1, 2, 3, 4]),
            wait_time: 5,
        };

        let buf = pkt.encode_to_vec();
        assert_eq!(TicketMessage::decode(&buf).unwrap(), pkt);
    }
}
